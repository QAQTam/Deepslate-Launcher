//! 外置世界生成。
//!
//! 世界生成不发生在游戏客户端里，而是在启动器后台启动一次原版 `server.jar`：
//!
//! ```text
//! maps/.staging/worldgen-<id>/
//!   eula.txt / server.properties / world/
//!         ↓ Done ( + stop
//! maps/<map-id>/                 # 通过 maps::adopt_generated_dir 原子纳入地图库
//! ```
//!
//! 这样做的好处：
//! - 不需要 Fabric mod；
//! - 世界生成和游戏客户端版本解耦，只依赖 Mojang server.jar；
//! - 失败/超时不会留下半成品地图；
//! - 生成完成后地图可以直接被 `--quickPlaySingleplayer` 打开。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::events::CoreEvent;
use crate::maps::{self, GeneratorInfo, MapSummary};
use crate::saves;
use crate::server::{self, ServerOptions, WorldCreationOptions, WorldType};

#[derive(Debug, Clone)]
pub struct WorldSpec {
    pub name: String,
    pub seed: String,
    pub world_type: WorldType,
    /// survival / creative / adventure / spectator
    pub game_mode: String,
    pub cheats: bool,
}

impl WorldSpec {
    fn generator_info(&self) -> GeneratorInfo {
        GeneratorInfo {
            kind: world_type_key(self.world_type).to_string(),
            seed: self.seed.trim().to_string(),
        }
    }
}

fn world_type_key(world_type: WorldType) -> &'static str {
    match world_type {
        WorldType::Normal => "normal",
        WorldType::Flat => "flat",
        WorldType::LargeBiomes => "large_biomes",
        WorldType::Amplified => "amplified",
    }
}

/// 用后台原版服务端生成一个世界，并纳入地图库。
///
/// `memory_mb` 由调用方限制在合理范围；首次会下载 server.jar。
pub async fn generate(
    config: &AppConfig,
    version: &str,
    java_home: PathBuf,
    memory_mb: u32,
    spec: &WorldSpec,
    events: &mpsc::Sender<CoreEvent>,
) -> Result<MapSummary, String> {
    let name = spec.name.trim();
    if name.is_empty() {
        return Err("世界名称不能为空".to_string());
    }
    if name.chars().count() > 64 {
        return Err("世界名称过长（最多 64 字符）".to_string());
    }

    let _ = maps::ensure_root(config)?;
    // 开始新任务前顺手清理上一次崩溃留下的死任务；活进程会被保留。
    let _ = cleanup_stale(config);
    let paths = crate::runtime::paths(config);
    std::fs::create_dir_all(&paths.cache_dir).map_err(|e| format!("创建缓存目录失败: {e}"))?;

    let staging = paths
        .maps_dir
        .join(maps::STAGING_DIR)
        .join(format!("worldgen-{}", maps::new_id()));
    std::fs::create_dir_all(&staging).map_err(|e| format!("创建生成暂存目录失败: {e}"))?;
    let job_file = staging.join(".job.json");
    write_job_start(&job_file)?;

    let mut server = ServerOptions::new(
        config,
        version,
        java_home,
        memory_mb.clamp(1024, 8192),
        0,
        false,
        true,
    );
    // server.jar 放缓存目录，不污染长期开服目录。
    server.server_dir = paths.cache_dir.join("server").join(version);

    let opts = WorldCreationOptions {
        server,
        saves_dir: staging.clone(),
        name: name.to_string(),
        seed: spec.seed.trim().to_string(),
        world_type: spec.world_type,
        game_mode: if spec.game_mode.trim().is_empty() {
            "survival".to_string()
        } else {
            spec.game_mode.clone()
        },
        cheats: spec.cheats,
    };

    let result = async {
        let id = server::create_world_with_job(&opts, events, Some(&job_file)).await?;
        let generated = staging.join(&id);
        if !generated.join("level.dat").is_file() {
            return Err(format!(
                "服务端生成结束，但找不到 {}",
                generated.join("level.dat").display()
            ));
        }
        saves::validate_world_dir(&generated)?;
        write_complete_marker(&staging, &generated, spec)?;
        maps::adopt_generated_dir(config, &generated, Some(spec.generator_info()))
    }
    .await;

    finish_staging(&staging, result.is_ok());
    result
}

fn finish_staging(staging: &Path, success: bool) {
    if success || !staging.join(".complete.json").exists() {
        let _ = fs::remove_dir_all(staging);
    } else {
        // The launcher still runs, but this job has ended; allow the next cleanup to recover it.
        let _ = fs::write(staging.join(".job.json"), r#"{"pid":0,"owner_pid":0}"#);
    }
}

fn write_job_start(path: &Path) -> Result<(), String> {
    let value = serde_json::json!({
        "pid": serde_json::Value::Null,
        "owner_pid": std::process::id(),
        "started_at": maps::now_ms(),
    });
    fs::write(
        path,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入生成任务标记失败 {}: {e}", path.display()))
}

fn write_complete_marker(staging: &Path, world: &Path, spec: &WorldSpec) -> Result<(), String> {
    let relative = world
        .strip_prefix(staging)
        .map_err(|e| format!("生成世界不在 staging 内: {e}"))?;
    let value = serde_json::json!({
        "world": relative.to_string_lossy(),
        "generator": {
            "kind": world_type_key(spec.world_type),
            "seed": spec.seed.trim(),
        },
    });
    let path = staging.join(".complete.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入生成完成标记失败 {}: {e}", path.display()))
}

fn marker_field_pid(path: &Path, field: &str) -> Option<u32> {
    let text = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get(field)?
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
}

fn marker_pid(path: &Path) -> Option<u32> {
    marker_field_pid(path, "pid")
}

fn old_enough(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|age| age > Duration::from_secs(3600))
        .unwrap_or(false)
}

fn cleanup_generation_dir(dir: &Path, marker_name: &str) -> Result<bool, String> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let marker = dir.join(marker_name);
    if let Some(pid) = marker_pid(&marker) {
        if pid != 0 && server::pid_alive(pid) {
            return Ok(false);
        }
    } else if !old_enough(dir) {
        // 刚创建、还没写 PID 的任务可能正在进行，先不动。
        return Ok(false);
    }
    fs::remove_dir_all(dir).map_err(|e| format!("清理陈旧生成目录 {} 失败: {e}", dir.display()))?;
    Ok(true)
}

/// 生成完成但 launcher 在纳入地图库前崩溃：完成标记 + 可解析世界，恢复纳入。
fn cleanup_worldgen_staging(config: &AppConfig, dir: &Path) -> Result<bool, String> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let marker = dir.join(".job.json");
    if marker_field_pid(&marker, "owner_pid").is_some_and(server::pid_alive) {
        return Ok(false);
    }
    if let Some(pid) = marker_pid(&marker) {
        if pid != 0 && server::pid_alive(pid) {
            return Ok(false);
        }
    } else if !old_enough(dir) {
        return Ok(false);
    }

    let complete_path = dir.join(".complete.json");
    if complete_path.is_file() {
        if let Ok(text) = fs::read_to_string(&complete_path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(relative) = value.get("world").and_then(|v| v.as_str()) {
                    let relative = Path::new(relative);
                    if relative
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_)))
                    {
                        let world = dir.join(relative);
                        if saves::validate_world_dir(&world).is_ok() {
                            let generator = value.get("generator").map(|g| GeneratorInfo {
                                kind: g
                                    .get("kind")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("normal")
                                    .to_string(),
                                seed: g
                                    .get("seed")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            });
                            maps::adopt_generated_dir(config, &world, generator)?;
                            fs::remove_dir_all(dir).map_err(|e| {
                                format!("清理已恢复的生成目录失败 {}: {e}", dir.display())
                            })?;
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }

    fs::remove_dir_all(dir).map_err(|e| format!("清理陈旧生成目录 {} 失败: {e}", dir.display()))?;
    Ok(true)
}

/// 启动时调用：清理上次启动器崩溃留下的生成暂存目录和孤儿 server 进程。
///
/// 只处理带有 `.job.json` / `.deepslate-worldgen.json` 的生成目录，
/// 不会碰地图库中的正式地图或备份。
pub fn cleanup_stale(config: &AppConfig) -> Result<usize, String> {
    let paths = crate::runtime::paths(config);
    let mut removed = 0usize;

    let staging = paths.maps_dir.join(maps::STAGING_DIR);
    if let Ok(entries) = fs::read_dir(&staging) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("worldgen-")
                && cleanup_worldgen_staging(config, &path).unwrap_or(false)
            {
                removed += 1;
            }
        }
    }

    let server_cache = paths.cache_dir.join("server");
    if let Ok(versions) = fs::read_dir(&server_cache) {
        for version in versions.flatten() {
            let version_path = version.path();
            if !version_path.is_dir() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(&version_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(".worldgen-")
                        && cleanup_generation_dir(&path, ".deepslate-worldgen.json")
                            .unwrap_or(false)
                    {
                        removed += 1;
                    }
                }
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_preserves_live_server_generation() {
        let root = std::env::temp_dir().join(format!("deepslate-worldgen-live-{}", maps::new_id()));
        let dir = root.join("cache/server/26.3/.worldgen-live");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".deepslate-worldgen.json"),
            format!(r#"{{"pid":{}}}"#, std::process::id()),
        )
        .unwrap();
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().into();
        cleanup_stale(&config).unwrap();
        assert!(dir.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_preserves_completed_world_when_library_is_locked() {
        let root =
            std::env::temp_dir().join(format!("deepslate-worldgen-locked-{}", maps::new_id()));
        let staging = root.join("maps/.staging/worldgen-done");
        let world = staging.join("world");
        fs::create_dir_all(&world).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat"),
            world.join("level.dat"),
        )
        .unwrap();
        fs::write(staging.join(".job.json"), r#"{"pid":999999}"#).unwrap();
        fs::write(staging.join(".complete.json"), r#"{"world":"world"}"#).unwrap();
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().into();
        let guard = maps::lock_library(&config).unwrap();
        cleanup_stale(&config).unwrap();
        assert!(world.join("level.dat").exists());
        drop(guard);
        cleanup_stale(&config).unwrap();
        assert_eq!(maps::list(&config).unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_preserves_active_launcher_job_after_server_exits() {
        let root = std::env::temp_dir().join(format!("deepslate-owner-{}", maps::new_id()));
        let staging = root.join("maps/.staging/worldgen-live");
        fs::create_dir_all(&staging).unwrap();
        fs::write(
            staging.join(".job.json"),
            format!(r#"{{"pid":999999,"owner_pid":{}}}"#, std::process::id()),
        )
        .unwrap();
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().into();
        cleanup_stale(&config).unwrap();
        assert!(staging.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generation_failure_does_not_discard_completed_world() {
        let root = std::env::temp_dir().join(format!("deepslate-preserve-{}", maps::new_id()));
        let staging = root.join("worldgen-test");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join(".complete.json"), "{}").unwrap();
        finish_staging(&staging, false);
        assert!(staging.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn world_type_keys_are_stable() {
        assert_eq!(world_type_key(WorldType::Normal), "normal");
        assert_eq!(world_type_key(WorldType::Flat), "flat");
        assert_eq!(world_type_key(WorldType::LargeBiomes), "large_biomes");
        assert_eq!(world_type_key(WorldType::Amplified), "amplified");
    }

    #[test]
    fn spec_generator_info_trims_seed() {
        let spec = WorldSpec {
            name: "x".into(),
            seed: "  12345  ".into(),
            world_type: WorldType::Flat,
            game_mode: "creative".into(),
            cheats: true,
        };
        let info = spec.generator_info();
        assert_eq!(info.kind, "flat");
        assert_eq!(info.seed, "12345");
    }

    #[test]
    fn cleanup_stale_removes_dead_jobs_but_keeps_live_pid() {
        let root = std::env::temp_dir().join(format!("deepslate-worldgen-{}", maps::new_id()));
        std::fs::create_dir_all(root.join("maps").join(".staging")).unwrap();
        std::fs::create_dir_all(root.join("cache").join("server").join("26.3")).unwrap();
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().to_string();

        let dead = root.join("maps").join(".staging").join("worldgen-dead");
        std::fs::create_dir_all(&dead).unwrap();
        std::fs::write(
            dead.join(".job.json"),
            r#"{"pid": 999999, "started_at": 1}"#,
        )
        .unwrap();

        let live = root.join("maps").join(".staging").join("worldgen-live");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(
            live.join(".job.json"),
            format!(r#"{{"pid": {}, "started_at": 1}}"#, std::process::id()),
        )
        .unwrap();

        let server_dead = root
            .join("cache")
            .join("server")
            .join("26.3")
            .join(".worldgen-dead");
        std::fs::create_dir_all(&server_dead).unwrap();
        std::fs::write(
            server_dead.join(".deepslate-worldgen.json"),
            r#"{"pid": 999999, "started_at": 1}"#,
        )
        .unwrap();

        let removed = cleanup_stale(&config).unwrap();
        assert_eq!(removed, 2);
        assert!(!dead.exists());
        assert!(!server_dead.exists());
        assert!(live.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_recovers_completed_generation_before_adoption() {
        let root = std::env::temp_dir().join(format!("deepslate-worldgen-{}", maps::new_id()));
        let staging = root.join("maps").join(".staging").join("worldgen-done");
        let world = staging.join("world");
        std::fs::create_dir_all(&world).unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat");
        std::fs::copy(fixture, world.join("level.dat")).unwrap();
        std::fs::write(
            staging.join(".job.json"),
            r#"{"pid": 999999, "started_at": 1}"#,
        )
        .unwrap();
        std::fs::write(
            staging.join(".complete.json"),
            r#"{"world":"world","generator":{"kind":"flat","seed":"123"}}"#,
        )
        .unwrap();

        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().to_string();
        let removed = cleanup_stale(&config).unwrap();
        assert_eq!(removed, 1);
        assert!(!staging.exists());
        let maps = maps::list(&config).unwrap();
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].manifest.generator.as_ref().unwrap().kind, "flat");
        let _ = std::fs::remove_dir_all(root);
    }
}
