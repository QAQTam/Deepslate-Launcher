//! 运行时目录。
//!
//! 产品方向是“只维护最新版”的游戏客户端，因此不再把地图绑在某个版本实例上：
//!
//! ```text
//! <root>/
//!   game/versions| libraries| assets| natives
//!   game/saves -> ../maps          # 游戏看到的 saves，实际落到地图库
//!   maps/<map-id>/                 # 长期资产
//!   cache/                         # server.jar / 世界生成临时目录
//!   backups/                       # 地图备份
//! ```
//!
//! `game/` 仍是共享游戏文件目录（沿用历史布局）；地图库独立于它。
//! 版本隔离/存档隔离是旧功能，这里不再使用；迁移时会保留旧目录，不删除。

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{self, AppConfig};
use crate::maps;

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub root_dir: PathBuf,
    /// 共享游戏文件：versions / libraries / assets / natives
    pub game_dir: PathBuf,
    /// `--gameDir`：options.txt / logs / mods / saves
    pub instance_dir: PathBuf,
    /// 地图库：saves 链接到这里
    pub maps_dir: PathBuf,
    /// 下载缓存 / 世界生成临时目录
    pub cache_dir: PathBuf,
    /// 地图备份
    pub backups_dir: PathBuf,
}

/// 显式选择实例，避免更改配置时影响已经运行的会话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    Normal,
    Enhanced,
}

pub fn paths(config: &AppConfig) -> RuntimePaths {
    paths_for_mode(config, RuntimeMode::Normal)
}

pub fn paths_for_mode(config: &AppConfig, mode: RuntimeMode) -> RuntimePaths {
    let root_dir = PathBuf::from(&config.root_dir);
    let game_dir = config::resolve_game_dir(config);
    let instance_dir = match mode {
        RuntimeMode::Normal => game_dir.clone(),
        RuntimeMode::Enhanced => root_dir.join("instances").join("enhanced-26.3"),
    };
    RuntimePaths {
        maps_dir: maps::root(config),
        cache_dir: root_dir.join("cache"),
        backups_dir: root_dir.join("backups"),
        root_dir,
        game_dir,
        instance_dir,
    }
}

pub fn saves_link(config: &AppConfig) -> PathBuf {
    paths(config).instance_dir.join("saves")
}

/// 建立运行时目录，并确保 `instance/saves` 指向地图库。
///
/// 幂等；可以在打开地图页、启动游戏、导入地图前调用。
pub fn ensure(config: &AppConfig) -> Result<RuntimePaths, String> {
    ensure_for_mode(config, RuntimeMode::Normal)
}

pub fn ensure_for_mode(config: &AppConfig, mode: RuntimeMode) -> Result<RuntimePaths, String> {
    let paths = paths_for_mode(config, mode);
    for dir in [
        &paths.game_dir,
        &paths.instance_dir,
        &paths.cache_dir,
        &paths.backups_dir,
    ] {
        fs::create_dir_all(dir)
            .map_err(|e| format!("创建运行时目录失败 {}: {e}", dir.display()))?;
    }
    maps::ensure_root(config)?;
    ensure_saves_link(config, &paths)?;
    Ok(paths)
}

fn ensure_saves_link(config: &AppConfig, paths: &RuntimePaths) -> Result<(), String> {
    let link = paths.instance_dir.join("saves");
    let target = &paths.maps_dir;

    if let Ok(meta) = fs::symlink_metadata(&link) {
        if meta.file_type().is_symlink() {
            let current = fs::canonicalize(&link).unwrap_or_else(|_| link.clone());
            let wanted = fs::canonicalize(target).unwrap_or_else(|_| target.clone());
            if current == wanted {
                return Ok(());
            }
            // 旧版 saves_isolation 会留下 saves -> root/saves/<version> 的链接。
            // 先把目标里的世界导入地图库，再替换链接；目标目录本身保留。
            let legacy_target = fs::canonicalize(&link)
                .map_err(|e| format!("saves 链接指向了不存在的位置: {} ({e})", link.display()))?;
            if !legacy_target.is_dir() {
                return Err(format!(
                    "saves 链接目标不是目录: {}",
                    legacy_target.display()
                ));
            }
            migrate_legacy_saves(config, &legacy_target, target)?;
            fs::remove_file(&link).map_err(|e| format!("移除旧 saves 链接失败: {e}"))?;
            return create_dir_link(target, &link);
        }
        if meta.is_dir() {
            migrate_legacy_saves(config, &link, target)?;
            return create_dir_link(target, &link);
        }
        return Err(format!("saves 路径不是目录: {}", link.display()));
    }

    create_dir_link(target, &link)
}

/// 旧实例的 saves 里如果已经有世界：先复制进地图库，成功后把整个旧目录
/// 改名为 `saves.legacy-<时间戳>`，绝不直接删除。
fn migrate_legacy_saves(
    config: &AppConfig,
    saves_dir: &Path,
    maps_dir: &Path,
) -> Result<(), String> {
    let mut worlds = Vec::new();
    for entry in fs::read_dir(saves_dir)
        .map_err(|e| format!("读取旧存档目录失败 {}: {e}", saves_dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || !path.join("level.dat").is_file() {
            continue;
        }
        worlds.push(path);
    }

    if worlds.is_empty() {
        let mut entries =
            fs::read_dir(saves_dir).map_err(|e| format!("读取旧 saves 目录失败: {e}"))?;
        if entries.next().is_none() {
            fs::remove_dir(saves_dir).map_err(|e| format!("移除空 saves 目录失败: {e}"))?;
        } else {
            let legacy = unique_legacy_path(saves_dir);
            fs::rename(saves_dir, &legacy).map_err(|e| format!("保留旧 saves 目录失败: {e}"))?;
            println!("旧 saves 没有世界，已保留在 {}", legacy.display());
        }
        return Ok(());
    }

    // 先导入，任何一个失败就保留旧目录原样，避免半迁移。
    for world in &worlds {
        maps::import_dir_with_source(
            config,
            world,
            maps::MapSource::Migrated,
            Some(world.display().to_string()),
        )
        .map_err(|e| format!("迁移旧存档 {} 失败: {e}；旧目录保持不变", world.display()))?;
    }

    let legacy = unique_legacy_path(saves_dir);
    fs::rename(saves_dir, &legacy).map_err(|e| {
        format!(
            "旧存档已复制到地图库，但重命名旧目录失败: {e}（旧目录: {}）",
            saves_dir.display()
        )
    })?;
    println!(
        "已迁移 {} 个旧存档到 {}，旧目录保留在 {}",
        worlds.len(),
        maps_dir.display(),
        legacy.display()
    );
    Ok(())
}

fn unique_legacy_path(saves_dir: &Path) -> PathBuf {
    let parent = saves_dir.parent().unwrap_or(saves_dir);
    let base = format!(
        "{}.legacy-{}",
        saves_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("saves"),
        crate::maps::now_ms()
    );
    let mut candidate = parent.join(&base);
    let mut n = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

fn create_dir_link(target: &Path, link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建链接父目录失败: {e}"))?;
    }
    #[cfg(unix)]
    {
        let target = fs::canonicalize(target)
            .map_err(|e| format!("解析 saves 目标失败 {}: {e}", target.display()))?;
        std::os::unix::fs::symlink(&target, link).map_err(|e| {
            format!(
                "创建 saves 链接失败 {} -> {}: {e}",
                link.display(),
                target.display()
            )
        })?;
        return Ok(());
    }
    #[cfg(windows)]
    {
        let target = fs::canonicalize(target)
            .map_err(|e| format!("解析 saves 目标失败 {}: {e}", target.display()))?;
        if std::os::windows::fs::symlink_dir(&target, link).is_ok() {
            return Ok(());
        }
        // 无开发者模式时 symlink 可能失败；目录 junction 不需要管理员权限。
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(&target)
            .status()
            .map_err(|e| format!("创建 saves junction 失败: {e}"))?;
        if status.success() {
            return Ok(());
        }
        return Err(format!(
            "创建 saves 链接失败: {} -> {}（可尝试以开发者模式运行，或手动创建 junction）",
            link.display(),
            target.display()
        ));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err("当前平台不支持目录链接".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn relative_root_creates_resolvable_saves_link() {
        let root = PathBuf::from(format!("target/runtime-relative-{}", uuid::Uuid::new_v4()));
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().into();
        let paths = ensure(&config).unwrap();
        assert_eq!(
            fs::canonicalize(paths.instance_dir.join("saves")).unwrap(),
            fs::canonicalize(&paths.maps_dir).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_config() -> (AppConfig, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "deepslate-runtime-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().to_string();
        (config, root)
    }

    fn put_world(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat");
        fs::copy(fixture, dir.join("level.dat")).unwrap();
    }

    #[test]
    fn migrates_legacy_saves_and_links_maps() {
        let (config, root) = temp_config();
        let legacy = root.join("game").join("saves");
        put_world(&legacy.join("old-world"));

        let paths = ensure(&config).unwrap();
        assert!(paths.instance_dir.join("saves").is_dir());
        assert_eq!(maps::list(&config).unwrap().len(), 1);
        assert!(!maps::list(&config).unwrap()[0].name.is_empty());
        // 旧目录保留
        let legacy_dirs: Vec<_> = fs::read_dir(root.join("game"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("saves.legacy-"))
            .collect();
        assert_eq!(legacy_dirs.len(), 1);
        // 幂等
        ensure(&config).unwrap();
        assert_eq!(maps::list(&config).unwrap().len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn creates_link_for_empty_layout() {
        let (config, root) = temp_config();
        let paths = ensure(&config).unwrap();
        let meta = fs::symlink_metadata(paths.instance_dir.join("saves")).unwrap();
        assert!(meta.file_type().is_symlink());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn enhanced_instance_is_separate_and_shares_maps() {
        let (config, root) = temp_config();
        let normal = ensure_for_mode(&config, RuntimeMode::Normal).unwrap();
        let enhanced = ensure_for_mode(&config, RuntimeMode::Enhanced).unwrap();
        assert_eq!(normal.game_dir, enhanced.game_dir);
        assert_ne!(normal.instance_dir, enhanced.instance_dir);
        assert_eq!(enhanced.instance_dir, root.join("instances/enhanced-26.3"));
        assert_eq!(
            fs::canonicalize(normal.instance_dir.join("saves")).unwrap(),
            fs::canonicalize(enhanced.instance_dir.join("saves")).unwrap()
        );
        fs::write(normal.instance_dir.join("options.txt"), "normal").unwrap();
        assert!(!enhanced.instance_dir.join("options.txt").exists());
        ensure_for_mode(&config, RuntimeMode::Enhanced).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn migrates_legacy_saves_isolation_symlink() {
        let (config, root) = temp_config();
        let legacy = root.join("saves").join("26.3");
        put_world(&legacy.join("old-world"));
        fs::create_dir_all(root.join("game")).unwrap();
        std::os::unix::fs::symlink(&legacy, root.join("game").join("saves")).unwrap();

        let paths = ensure(&config).unwrap();
        assert_eq!(maps::list(&config).unwrap().len(), 1);
        assert_eq!(
            fs::canonicalize(paths.instance_dir.join("saves")).unwrap(),
            fs::canonicalize(root.join("maps")).unwrap()
        );
        // 旧目标被保留为 legacy 目录
        let legacy_dirs: Vec<_> = fs::read_dir(root.join("saves"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".legacy-"))
            .collect();
        assert_eq!(legacy_dirs.len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
