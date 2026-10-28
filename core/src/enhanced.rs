//! Resolution and verification of the components of the 26.3 managed runtime.
use crate::{fabric, net};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::io::{Cursor, Read};

pub const MINECRAFT: &str = "26.3";
pub const BRIDGE_VERSION: &str = "0.3.0";
pub const PROTOCOL: u32 = 2;
pub const BRIDGE_JAR: &[u8] = include_bytes!("../assets/deepslate-bridge-0.3.0.jar");
const API_ROOT: &str = "https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Components {
    pub minecraft: String,
    pub loader: String,
    pub fabric_api: String,
    pub fabric_api_sha1: String,
    pub bridge: String,
    pub bridge_sha1: String,
    pub protocol: u32,
}

pub struct ResolvedComponents {
    pub lock: Components,
    pub api_jar: Vec<u8>,
}

fn numeric_version(version: &str) -> Option<Vec<u32>> {
    let mut result = Vec::new();
    for part in version.split('.') {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        result.push(part.parse().ok()?);
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

fn at_least(actual: &str, minimum: &str) -> bool {
    match (numeric_version(actual), numeric_version(minimum)) {
        (Some(mut a), Some(mut b)) => {
            let len = a.len().max(b.len());
            a.resize(len, 0);
            b.resize(len, 0);
            a >= b
        }
        _ => false,
    }
}

/// Choose a formal release for precisely 26.3; reject snapshots and pre-releases.
fn select_api(xml: &str) -> Result<String, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("Fabric API 元数据无效: {e}"))?;
    let mut candidates = doc
        .descendants()
        .filter(|n| n.has_tag_name("version"))
        .filter_map(|n| n.text())
        .filter_map(|v| {
            let (api, game) = v.split_once('+')?;
            if game != MINECRAFT {
                return None;
            }
            let parsed = numeric_version(api)?;
            if parsed.len() != 3 || !at_least(api, "0.165.0") {
                return None;
            }
            Some((parsed, v.to_string()))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates
        .pop()
        .map(|(_, v)| v)
        .ok_or_else(|| "没有找到兼容 26.3 的正式 Fabric API".into())
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

fn read_metadata(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("组件不是有效 JAR: {e}"))?;
    let entry = zip
        .by_name("fabric.mod.json")
        .map_err(|e| format!("组件缺少 fabric.mod.json: {e}"))?;
    let mut text = Vec::new();
    entry
        .take(1024 * 1024 + 1)
        .read_to_end(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 1024 * 1024 {
        return Err("组件元数据过大".into());
    }
    serde_json::from_slice(&text).map_err(|e| format!("组件元数据无效: {e}"))
}

fn validate_api(bytes: &[u8], version: &str, loader: &str) -> Result<(), String> {
    let meta = read_metadata(bytes)?;
    if meta["id"] != "fabric-api" || meta["version"] != version {
        return Err("Fabric API 标识或版本不匹配".into());
    }
    let deps = meta
        .get("depends")
        .and_then(|v| v.as_object())
        .ok_or("Fabric API 缺少依赖约束")?;
    let minecraft = deps
        .get("minecraft")
        .and_then(|v| v.as_str())
        .ok_or("Fabric API 缺少游戏版本约束")?;
    if !matches!(minecraft, "~26.3-" | "~26.3" | "26.3") {
        return Err(format!("Fabric API 游戏版本约束不受支持: {minecraft}"));
    }
    let loader_req = deps
        .get("fabricloader")
        .and_then(|v| v.as_str())
        .and_then(|s| s.strip_prefix(">="))
        .ok_or("Fabric API Loader 约束不受支持")?;
    if !at_least(loader, loader_req) {
        return Err(format!("Fabric API 需要 Loader >= {loader_req}"));
    }
    let java_req = deps
        .get("java")
        .and_then(|v| v.as_str())
        .and_then(|s| s.strip_prefix(">="))
        .ok_or("Fabric API Java 约束不受支持")?;
    if !at_least("25", java_req) {
        return Err(format!("Fabric API 需要 Java >= {java_req}"));
    }
    Ok(())
}

fn validate_bridge(loader: &str, api: &str) -> Result<(), String> {
    let meta = read_metadata(BRIDGE_JAR)?;
    if meta["id"] != "deepslate-bridge"
        || meta["version"] != BRIDGE_VERSION
        || meta["depends"]["minecraft"] != MINECRAFT
    {
        return Err("内置 Bridge 元数据不匹配".into());
    }
    if !at_least(loader, "0.20.1") || !at_least(api.split('+').next().unwrap_or(""), "0.165.0") {
        return Err("Loader 或 Fabric API 不满足 Bridge 兼容范围".into());
    }
    Ok(())
}

/// Fetch official metadata and verify the advertised digest and embedded constraints.
pub async fn resolve() -> Result<ResolvedComponents, String> {
    let loader = fabric::latest_stable_loader_for_game(MINECRAFT)
        .await?
        .version;
    let client = net::client()?;
    let xml = net::get_text(&client, &format!("{API_ROOT}/maven-metadata.xml")).await?;
    let api = select_api(&xml)?;
    let url = format!("{API_ROOT}/{api}/fabric-api-{api}.jar");
    let digest = net::get_text(&client, &format!("{url}.sha1"))
        .await?
        .trim()
        .to_ascii_lowercase();
    if digest.len() != 40 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Fabric API 摘要无效".into());
    }
    let api_jar = net::get_bytes(&client, &url).await?;
    if hash(&api_jar) != digest {
        return Err("Fabric API SHA-1 校验失败".into());
    }
    validate_api(&api_jar, &api, &loader)?;
    validate_bridge(&loader, &api)?;
    Ok(ResolvedComponents {
        lock: Components {
            minecraft: MINECRAFT.into(),
            loader,
            fabric_api: api,
            fabric_api_sha1: digest,
            bridge: BRIDGE_VERSION.into(),
            bridge_sha1: hash(BRIDGE_JAR),
            protocol: PROTOCOL,
        },
        api_jar,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn api_jar(loader: &str, game: &str, java: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        let metadata = serde_json::json!({ "id":"fabric-api", "version":"0.165.0+26.3",
            "depends": { "fabricloader":loader, "minecraft":game, "java":java } });
        zip.write_all(metadata.to_string().as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn selects_only_formal_releases_for_263() {
        let xml = "<metadata><versioning><versions><version>0.170.0+26.2</version><version>0.166.0-alpha+26.3</version><version>0.165.0+26.3</version><version>0.166.0+26.3</version><version>0.200.0+26.3.1</version></versions></versioning></metadata>";
        assert_eq!(select_api(xml).unwrap(), "0.166.0+26.3");
        assert!(select_api("<metadata><version>0.1.0+26.2</version></metadata>").is_err());
    }
    #[test]
    fn embedded_bridge_has_expected_identity_and_compatibility() {
        validate_bridge("0.20.1", "0.165.0+26.3").unwrap();
        assert!(validate_bridge("0.19.1", "0.165.0+26.3").is_err());
        assert!(validate_bridge("0.20.1", "0.164.0+26.3").is_err());
    }
    #[test]
    fn api_checks_embedded_constraints_and_identity() {
        let valid = api_jar(">=0.19.3", "~26.3-", ">=25");
        validate_api(&valid, "0.165.0+26.3", "0.20.1").unwrap();
        assert!(validate_api(&valid, "0.165.1+26.3", "0.20.1").is_err());
        assert!(validate_api(
            &api_jar(">=0.21.0", "~26.3-", ">=25"),
            "0.165.0+26.3",
            "0.20.1"
        )
        .is_err());
        assert!(validate_api(
            &api_jar(">=0.19.3", "~26.2-", ">=25"),
            "0.165.0+26.3",
            "0.20.1"
        )
        .is_err());
        assert!(validate_api(
            &api_jar(">=0.19.3", "~26.3-", ">=26"),
            "0.165.0+26.3",
            "0.20.1"
        )
        .is_err());
    }
    #[tokio::test]
    #[ignore = "需要联网访问 Fabric 官方元数据与 Maven"]
    async fn resolve_real_components() {
        let resolved = resolve().await.unwrap();
        assert_eq!(resolved.lock.minecraft, "26.3");
        assert_eq!(hash(&resolved.api_jar), resolved.lock.fabric_api_sha1);
    }
    #[tokio::test]
    #[ignore = "需要联网下载安装 Minecraft 26.3 和 Fabric"]
    async fn install_real_components_into_isolated_root() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target")
            .join(format!("enhanced-install-test-{}", uuid::Uuid::new_v4()));
        let mut config = crate::config::AppConfig::default();
        config.root_dir = root.to_string_lossy().into_owned();
        let (tx, mut rx) = tokio::sync::mpsc::channel(128);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = install(&config, tx).await;
        drain.await.unwrap();
        let installed = result.unwrap();
        assert_eq!(installed.lock.components.minecraft, MINECRAFT);
        assert_eq!(
            load_installed(&config).unwrap().lock.generation,
            installed.lock.generation
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn installation_preserves_previous_lock_on_failure_and_detects_corruption() {
        let root = std::env::temp_dir().join(format!("enhanced-test-{}", uuid::Uuid::new_v4()));
        let mut config = crate::config::AppConfig::default();
        config.root_dir = root.to_string_lossy().into_owned();
        let api = api_jar(">=0.19.3", "~26.3-", ">=25");
        let resolved = ResolvedComponents {
            lock: Components {
                minecraft: MINECRAFT.into(),
                loader: "0.20.1".into(),
                fabric_api: "0.165.0+26.3".into(),
                fabric_api_sha1: hash(&api),
                bridge: BRIDGE_VERSION.into(),
                bridge_sha1: hash(BRIDGE_JAR),
                protocol: PROTOCOL,
            },
            api_jar: api,
        };
        let version = fabric::version_id(MINECRAFT, "0.20.1");
        let profile_dir =
            crate::runtime::paths_for_mode(&config, crate::runtime::RuntimeMode::Enhanced)
                .game_dir
                .join("versions")
                .join(&version);
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(profile_dir.join(format!("{version}.json")), "{}").unwrap();
        let installed = write_generation(&config, &resolved, version.clone()).unwrap();
        let original = std::fs::read(lock_path(&config)).unwrap();
        let mut bad = resolved;
        bad.api_jar.push(0);
        assert!(write_generation(&config, &bad, version).is_err());
        assert_eq!(std::fs::read(lock_path(&config)).unwrap(), original);
        assert_eq!(
            load_installed(&config).unwrap().lock.generation,
            installed.lock.generation
        );
        std::fs::write(&installed.api_jar, b"corrupt").unwrap();
        assert!(load_installed(&config).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// A lock points to a complete immutable component generation. It is replaced only after
/// all downloads and validation succeed; the previous generation stays available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLock {
    pub generation: String,
    pub components: Components,
    pub version_id: String,
}

#[derive(Debug, Clone)]
pub struct InstalledRuntime {
    pub lock: RuntimeLock,
    pub api_jar: std::path::PathBuf,
    pub bridge_jar: std::path::PathBuf,
}

fn lock_path(config: &crate::config::AppConfig) -> std::path::PathBuf {
    std::path::PathBuf::from(&config.root_dir)
        .join("config")
        .join("enhanced-runtime.lock.json")
}
fn generation_root(config: &crate::config::AppConfig) -> std::path::PathBuf {
    std::path::PathBuf::from(&config.root_dir)
        .join("cache")
        .join("enhanced-components")
}

/// Load and revalidate an installed combination before use.
pub fn load_installed(config: &crate::config::AppConfig) -> Result<InstalledRuntime, String> {
    let lock: RuntimeLock = serde_json::from_slice(
        &std::fs::read(lock_path(config)).map_err(|e| format!("读取增强体验配置失败: {e}"))?,
    )
    .map_err(|e| format!("增强体验配置无效: {e}"))?;
    let generation = uuid::Uuid::parse_str(&lock.generation).map_err(|_| "组件代次无效")?;
    if generation.to_string() != lock.generation
        || lock.components.minecraft != MINECRAFT
        || lock.components.bridge != BRIDGE_VERSION
        || lock.components.protocol != PROTOCOL
        || lock.version_id != fabric::version_id(MINECRAFT, &lock.components.loader)
    {
        return Err("增强体验组件记录不兼容".into());
    }
    let base = generation_root(config).join(&lock.generation);
    let api_jar = base.join("fabric-api.jar");
    let bridge_jar = base.join("deepslate-bridge.jar");
    let api = std::fs::read(&api_jar).map_err(|e| format!("读取 Fabric API 失败: {e}"))?;
    let bridge = std::fs::read(&bridge_jar).map_err(|e| format!("读取 Bridge 失败: {e}"))?;
    if hash(&api) != lock.components.fabric_api_sha1
        || hash(&bridge) != lock.components.bridge_sha1
        || hash(BRIDGE_JAR) != lock.components.bridge_sha1
    {
        return Err("增强体验组件摘要不匹配".into());
    }
    validate_api(&api, &lock.components.fabric_api, &lock.components.loader)?;
    validate_bridge(&lock.components.loader, &lock.components.fabric_api)?;
    let paths = crate::runtime::paths_for_mode(config, crate::runtime::RuntimeMode::Enhanced);
    let profile = paths
        .game_dir
        .join("versions")
        .join(&lock.version_id)
        .join(format!("{}.json", lock.version_id));
    if !profile.is_file() {
        return Err("增强体验 Fabric profile 缺失".into());
    }
    Ok(InstalledRuntime {
        lock,
        api_jar,
        bridge_jar,
    })
}

fn write_generation(
    config: &crate::config::AppConfig,
    resolved: &ResolvedComponents,
    version_id: String,
) -> Result<InstalledRuntime, String> {
    use std::io::Write;
    if hash(&resolved.api_jar) != resolved.lock.fabric_api_sha1
        || hash(BRIDGE_JAR) != resolved.lock.bridge_sha1
    {
        return Err("组件摘要不匹配，未更新运行配置".into());
    }
    validate_api(
        &resolved.api_jar,
        &resolved.lock.fabric_api,
        &resolved.lock.loader,
    )?;
    validate_bridge(&resolved.lock.loader, &resolved.lock.fabric_api)?;
    if version_id != fabric::version_id(MINECRAFT, &resolved.lock.loader) {
        return Err("Fabric profile 标识不匹配".into());
    }
    if resolved.lock.minecraft != MINECRAFT
        || resolved.lock.bridge != BRIDGE_VERSION
        || resolved.lock.protocol != PROTOCOL
    {
        return Err("组件记录不兼容".into());
    }
    let game_dir =
        crate::runtime::paths_for_mode(config, crate::runtime::RuntimeMode::Enhanced).game_dir;
    let profile = game_dir
        .join("versions")
        .join(&version_id)
        .join(format!("{version_id}.json"));
    if !profile.is_file() {
        return Err("增强体验 Fabric profile 缺失".into());
    }
    let generation = uuid::Uuid::new_v4().to_string();
    let dir = generation_root(config).join(&generation);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        for (name, data) in [
            ("fabric-api.jar", resolved.api_jar.as_slice()),
            ("deepslate-bridge.jar", BRIDGE_JAR),
        ] {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(dir.join(name))
                .map_err(|e| e.to_string())?;
            file.write_all(data)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        let lock = RuntimeLock {
            generation: generation.clone(),
            components: resolved.lock.clone(),
            version_id,
        };
        let destination = lock_path(config);
        std::fs::create_dir_all(destination.parent().ok_or("运行配置目录无效")?)
            .map_err(|e| e.to_string())?;
        let temporary = destination.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let write_result = (|| -> Result<(), String> {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            serde_json::to_writer_pretty(&mut file, &lock).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(&temporary, &destination)
                .map_err(|e| format!("切换增强体验配置失败: {e}"))?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        write_result
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    load_installed(config)
}

/// Install into an isolated instance; user mods remain in that instance's mods directory.
pub async fn install(
    config: &crate::config::AppConfig,
    events: tokio::sync::mpsc::Sender<crate::events::CoreEvent>,
) -> Result<InstalledRuntime, String> {
    let _guard = crate::maps::lock_library(config)?;
    report_stage(&events, "resolving_components").await;
    let paths = crate::runtime::ensure_for_mode(config, crate::runtime::RuntimeMode::Enhanced)?;
    let resolved = resolve().await?;
    report_stage(&events, "checking_game_files").await;
    let installed = fabric::install(
        fabric::InstallOptions {
            game_version: MINECRAFT.into(),
            loader_version: resolved.lock.loader.clone(),
            game_dir: paths.game_dir,
            instance_dir: paths.instance_dir,
            concurrency: config.download_concurrency,
        },
        events.clone(),
    )
    .await?;
    report_stage(&events, "validating_components").await;
    write_generation(config, &resolved, installed.version_id)
}

async fn report_stage(events: &tokio::sync::mpsc::Sender<crate::events::CoreEvent>, stage: &str) {
    let _ = events
        .send(crate::events::CoreEvent::BridgeState {
            state: format!("phase_{stage}"),
            message: None,
        })
        .await;
}
