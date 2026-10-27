//! Fabric 加载器安装。
//!
//! 数据来源：<https://meta.fabricmc.net/v2>
//! - `GET /versions/loader` → 全局加载器列表
//! - `GET /versions/loader/{game}` → 该游戏版本可用的加载器（带 intermediary / launcherMeta）
//! - `GET /versions/loader/{game}/{loader}/profile/json` → 标准 version profile（带 `inheritsFrom`）
//!
//! 安装策略与主流启动器一致：
//! 1. 确保原版基础版本完整（版本 JSON + 客户端 jar + 库 + 资源）
//! 2. 取 Fabric profile JSON，原样写入 `versions/{id}/{id}.json`
//! 3. 下载 profile 里声明的 maven 库（asm / sponge-mixin / intermediary / fabric-loader）
//! 4. 创建实例 `mods/` 目录（Fabric loader 自己负责发现与注入 mod）
//!
//! 启动时 `launch::prepare` 会沿 `inheritsFrom` 合并原版 JSON，主类切换为
//! `net.fabricmc.loader.impl.launch.knot.KnotClient`，并保留 Fabric 追加的 JVM 参数
//! （`-DFabricMcEmu=...`）。**mods 的字节码注入由 Fabric loader 在进程内完成，
//! 启动器不需要也不应该自己改 jar。**

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::mpsc;

use crate::download::{self, DownloadTask};
use crate::events::CoreEvent;
use crate::launch::{maven_artifact_path, rules_allow, Rule};
use crate::net;

pub const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
pub const FABRIC_MAVEN: &str = "https://maven.fabricmc.net/";

/// 加载器版本元数据（全局列表与 per-game 列表里的 `loader` 字段同构）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoaderVersion {
    pub version: String,
    #[serde(default)]
    pub stable: bool,
    #[serde(default)]
    pub maven: String,
    #[serde(default)]
    pub build: u32,
    #[serde(default)]
    pub separator: String,
}

impl LoaderVersion {
    pub fn is_stable(&self) -> bool {
        self.stable
    }
}

impl std::fmt::Display for LoaderVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.stable {
            write!(f, "{}", self.version)
        } else {
            write!(f, "{} (beta)", self.version)
        }
    }
}

#[derive(Debug, Deserialize)]
struct LoaderEntry {
    loader: LoaderVersion,
}

/// `versions/{id}/{id}.json` 的目录名 / id 约定（Fabric 官方 installer 同款）。
pub fn version_id(game_version: &str, loader_version: &str) -> String {
    format!("fabric-loader-{loader_version}-{game_version}")
}

/// 该游戏版本可用的 Fabric 加载器列表（推荐使用）。
pub async fn list_loaders(game_version: impl Into<String>) -> Result<Vec<LoaderVersion>, String> {
    let game_version = game_version.into();
    let client = net::client()?;
    let url = format!("{FABRIC_META}/versions/loader/{game_version}");
    match net::get_json::<Vec<LoaderEntry>>(&client, &url).await {
        Ok(entries) => {
            let mut loaders: Vec<LoaderVersion> = entries.into_iter().map(|e| e.loader).collect();
            // 服务端已按新→旧排序；把稳定版排在前面，避免默认选中 beta
            loaders.sort_by_key(|l| !l.stable);
            Ok(loaders)
        }
        Err(e) if e.contains("(404)") => Err(format!(
            "Fabric 没有为 Minecraft {game_version} 提供加载器: {e}"
        )),
        Err(_) => {
            // 元数据服务偶发抖动：退回全局加载器列表，最终兼容性由安装时的
            // profile 端点（404）判定。
            let mut loaders = all_loaders().await?;
            loaders.sort_by_key(|l| !l.stable);
            Ok(loaders)
        }
    }
}

/// Enhanced sessions must never fall back to the global, potentially incompatible list.
pub async fn latest_stable_loader_for_game(game_version: &str) -> Result<LoaderVersion, String> {
    crate::launch::validate_version_id(game_version)?;
    let client = net::client()?;
    let url = format!("{FABRIC_META}/versions/loader/{game_version}");
    let entries: Vec<LoaderEntry> = net::get_json(&client, &url).await?;
    entries
        .into_iter()
        .map(|entry| entry.loader)
        .find(|loader| loader.stable)
        .ok_or_else(|| format!("Fabric 没有为 Minecraft {game_version} 提供稳定加载器"))
}

/// 全局加载器列表（不保证与某个游戏版本兼容，作为兜底/展示用）。
pub async fn all_loaders() -> Result<Vec<LoaderVersion>, String> {
    let client = net::client()?;
    let url = format!("{FABRIC_META}/versions/loader");
    net::get_json(&client, &url).await
}

/// 默认选中：第一个稳定版，否则第一个。
pub fn default_loader(loaders: &[LoaderVersion]) -> Option<LoaderVersion> {
    loaders
        .iter()
        .find(|l| l.stable)
        .or_else(|| loaders.first())
        .cloned()
}

// ---------------------------------------------------------------------------
// profile JSON
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FabricProfile {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "inheritsFrom", default)]
    pub inherits_from: Option<String>,
    #[serde(rename = "mainClass", default)]
    pub main_class: String,
    #[serde(default)]
    pub libraries: Vec<ProfileLibrary>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProfileLibrary {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    /// 部分镜像 / 旧版 profile 会给出直链 artifact，而不是 maven 仓库前缀
    #[serde(default)]
    pub downloads: Option<ProfileDownloads>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProfileDownloads {
    #[serde(default)]
    pub artifact: Option<ProfileArtifact>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProfileArtifact {
    pub url: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
}

impl ProfileLibrary {
    fn repository(&self) -> &str {
        self.url.as_deref().unwrap_or(FABRIC_MAVEN)
    }
}

/// 把 profile 里的 maven 坐标库转换为下载任务。纯函数，便于单测。
pub fn library_tasks(
    libraries: &[ProfileLibrary],
    game_dir: &Path,
) -> Result<Vec<DownloadTask>, String> {
    let mut tasks = Vec::new();
    for lib in libraries {
        if !rules_allow(lib.rules.as_deref(), &Default::default()) {
            continue;
        }

        // 直链 artifact 优先（官方 Fabric profile 通常没有这一段）
        if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            let rel = artifact
                .path
                .clone()
                .or_else(|| maven_artifact_path(&lib.name))
                .ok_or_else(|| format!("无法解析 Fabric 库路径: {}", lib.name))?;
            tasks.push(DownloadTask {
                url: artifact.url.clone(),
                path: game_dir.join("libraries").join(rel),
                sha1: artifact
                    .sha1
                    .clone()
                    .or_else(|| lib.sha1.clone())
                    .filter(|s| !s.is_empty()),
            });
            continue;
        }

        let rel = maven_artifact_path(&lib.name)
            .ok_or_else(|| format!("无法解析 Fabric 库坐标: {}", lib.name))?;
        let url = format!("{}/{}", lib.repository().trim_end_matches('/'), rel);
        tasks.push(DownloadTask {
            url,
            path: game_dir.join("libraries").join(rel),
            sha1: lib.sha1.clone().filter(|s| !s.is_empty()),
        });
    }
    Ok(tasks)
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub game_version: String,
    pub loader_version: String,
    pub game_dir: PathBuf,
    /// 实例目录（用于创建 mods/）；版本隔离时通常是 versions/{id}
    pub instance_dir: PathBuf,
    pub concurrency: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub version_id: String,
    pub game_version: String,
    pub loader_version: String,
    pub libraries_total: usize,
    pub libraries_downloaded: usize,
}

/// 下载/校验原版基础版本，返回是否发生了下载。
async fn ensure_base_version(
    opts: &InstallOptions,
    events: &mpsc::Sender<CoreEvent>,
) -> Result<(), String> {
    if download::version_installation_is_complete(&opts.game_version, &opts.game_dir).await {
        return Ok(());
    }

    println!("基础版本 {} 不完整，先下载原版...", opts.game_version);
    download::run(
        &opts.game_version,
        &opts.game_dir,
        opts.concurrency,
        events.clone(),
    )
    .await
    .map_err(|e| format!("下载基础版本 {} 失败: {e}", opts.game_version))?;
    Ok(())
}

async fn fetch_profile(
    client: &reqwest::Client,
    game_version: &str,
    loader_version: &str,
) -> Result<(FabricProfile, String), String> {
    let url = format!("{FABRIC_META}/versions/loader/{game_version}/{loader_version}/profile/json");
    let text = net::get_text(client, &url).await.map_err(|e| {
        format!("获取 Fabric {loader_version} / MC {game_version} 的安装配置失败: {e}")
    })?;
    let profile: FabricProfile =
        serde_json::from_str(&text).map_err(|e| format!("解析 Fabric profile JSON 失败: {e}"))?;
    if profile.id.is_empty() || profile.main_class.is_empty() {
        return Err("Fabric profile JSON 缺少 id/mainClass".to_string());
    }
    Ok((profile, text))
}

/// 安装 Fabric：下载基础版本 + profile 声明的库，并写入版本 JSON。
pub async fn install(
    opts: InstallOptions,
    events: mpsc::Sender<CoreEvent>,
) -> Result<InstallResult, String> {
    crate::launch::validate_version_id(&opts.game_version)?;
    crate::launch::validate_version_id(&opts.loader_version)?;
    if opts.game_version.trim().is_empty() || opts.loader_version.trim().is_empty() {
        return Err("游戏版本与 Fabric 加载器版本都不能为空".to_string());
    }

    // 1. 原版基础版本
    ensure_base_version(&opts, &events).await?;

    // 2. Fabric profile
    let client = net::client()?;
    let (profile, raw_json) =
        fetch_profile(&client, &opts.game_version, &opts.loader_version).await?;
    crate::launch::validate_version_id(&profile.id)?;

    match profile.inherits_from.as_deref() {
        Some(parent) if parent == opts.game_version => {}
        Some(parent) => {
            return Err(format!(
                "Fabric profile 的 inheritsFrom ({parent}) 与请求的 {} 不一致",
                opts.game_version
            ));
        }
        None => return Err("Fabric profile 缺少 inheritsFrom，无法作为加载器版本安装".to_string()),
    }

    // 3. 下载库
    let tasks = library_tasks(&profile.libraries, &opts.game_dir)?;
    let libraries_total = tasks.len();
    let libraries_downloaded = download::download_all(
        tasks,
        opts.concurrency,
        events.clone(),
        "正在下载 Fabric 库...",
    )
    .await?;

    // 4. 最后写版本 JSON：它存在即代表安装完成
    let version_dir = opts.game_dir.join("versions").join(&profile.id);
    fs::create_dir_all(&version_dir)
        .await
        .map_err(|e| format!("创建版本目录失败 {}: {e}", version_dir.display()))?;
    let json_path = version_dir.join(format!("{}.json", profile.id));
    fs::write(&json_path, raw_json.as_bytes())
        .await
        .map_err(|e| format!("写入版本 JSON 失败 {}: {e}", json_path.display()))?;

    // 5. mods 目录（Fabric loader 从这里读取 mod；启动器只负责创建）
    let mods_dir = opts.instance_dir.join("mods");
    fs::create_dir_all(&mods_dir)
        .await
        .map_err(|e| format!("创建 mods 目录失败 {}: {e}", mods_dir.display()))?;

    println!(
        "Fabric 安装完成: {}（{} 个库，下载 {}）",
        profile.id, libraries_total, libraries_downloaded
    );

    Ok(InstallResult {
        version_id: profile.id,
        game_version: opts.game_version,
        loader_version: opts.loader_version,
        libraries_total,
        libraries_downloaded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 来自 meta.fabricmc.net 的真实 profile 片段（0.16.14 / 1.21.1）
    const PROFILE: &str = r#"{
        "id": "fabric-loader-0.16.14-1.21.1",
        "inheritsFrom": "1.21.1",
        "releaseTime": "2026-10-26T10:28:53+0000",
        "time": "2026-10-26T10:28:53+0000",
        "type": "release",
        "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "arguments": {"game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "]},
        "libraries": [
            {
                "name": "org.ow2.asm:asm:9.8",
                "url": "https://maven.fabricmc.net/",
                "sha1": "dc19ecb3f7889b7860697215cae99c0f9b6f6b4b"
            },
            {
                "name": "net.fabricmc:intermediary:1.21.1",
                "url": "https://maven.fabricmc.net/"
            },
            {
                "name": "net.fabricmc:fabric-loader:0.16.14",
                "url": "https://maven.fabricmc.net/"
            }
        ]
    }"#;

    #[test]
    fn version_id_matches_fabric_convention() {
        assert_eq!(
            version_id("1.21.1", "0.16.14"),
            "fabric-loader-0.16.14-1.21.1"
        );
        assert_eq!(
            version_id("1.21.1-pre1", "0.16.14"),
            "fabric-loader-0.16.14-1.21.1-pre1"
        );
    }

    #[test]
    fn profile_parses_and_builds_library_tasks() {
        let profile: FabricProfile = serde_json::from_str(PROFILE).unwrap();
        assert_eq!(profile.id, "fabric-loader-0.16.14-1.21.1");
        assert_eq!(profile.inherits_from.as_deref(), Some("1.21.1"));
        assert_eq!(
            profile.main_class,
            "net.fabricmc.loader.impl.launch.knot.KnotClient"
        );
        assert_eq!(profile.libraries.len(), 3);

        let dir = std::path::Path::new("/game");
        let tasks = library_tasks(&profile.libraries, dir).unwrap();
        assert_eq!(tasks.len(), 3);
        assert!(tasks[0].path.ends_with("org/ow2/asm/asm/9.8/asm-9.8.jar"));
        assert_eq!(
            tasks[0].url,
            "https://maven.fabricmc.net/org/ow2/asm/asm/9.8/asm-9.8.jar"
        );
        assert_eq!(
            tasks[0].sha1.as_deref(),
            Some("dc19ecb3f7889b7860697215cae99c0f9b6f6b4b")
        );
        assert!(tasks[1]
            .path
            .ends_with("net/fabricmc/intermediary/1.21.1/intermediary-1.21.1.jar"));
        assert!(tasks[1].sha1.is_none());
        assert!(tasks[2]
            .path
            .ends_with("net/fabricmc/fabric-loader/0.16.14/fabric-loader-0.16.14.jar"));
    }

    #[test]
    fn default_loader_prefers_stable() {
        let loaders = vec![
            LoaderVersion {
                version: "0.20.0".into(),
                stable: false,
                ..Default::default()
            },
            LoaderVersion {
                version: "0.19.5".into(),
                stable: true,
                ..Default::default()
            },
        ];
        assert_eq!(default_loader(&loaders).unwrap().version, "0.19.5");
        assert!(default_loader(&[]).is_none());
    }

    #[test]
    fn library_rules_are_respected() {
        let lib = ProfileLibrary {
            name: "g:a:1".into(),
            url: None,
            sha1: None,
            downloads: None,
            rules: Some(vec![Rule {
                action: "allow".into(),
                os: Some(crate::launch::OsRestriction {
                    name: Some("definitely-not-this-os".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
        };
        let tasks = library_tasks(&[lib], std::path::Path::new("/game")).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn library_tasks_support_direct_artifact_urls() {
        // 部分镜像 / 旧 profile 直接给 artifact URL，而不是 maven 仓库前缀
        let libs: Vec<ProfileLibrary> = serde_json::from_str(
            r#"[{
                "name": "example:demo:1.0",
                "downloads": {
                    "artifact": {
                        "url": "https://cdn.example.com/demo-1.0.jar",
                        "path": "example/demo/1.0/demo-1.0.jar",
                        "sha1": "0123456789abcdef0123456789abcdef01234567"
                    }
                }
            }]"#,
        )
        .unwrap();
        let tasks = library_tasks(&libs, std::path::Path::new("/game")).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].url, "https://cdn.example.com/demo-1.0.jar");
        assert!(tasks[0].path.ends_with("example/demo/1.0/demo-1.0.jar"));
        assert_eq!(
            tasks[0].sha1.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
    }

    /// 真实网络：拉取 profile 并检查库任务（不下载大文件）。
    #[test]
    #[ignore = "需要网络访问 meta.fabricmc.net"]
    fn real_profile_from_meta() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = net::client().unwrap();
            let (profile, _raw) = fetch_profile(&client, "1.21.1", "0.16.14").await.unwrap();
            assert_eq!(profile.inherits_from.as_deref(), Some("1.21.1"));
            let tasks = library_tasks(&profile.libraries, std::path::Path::new("/tmp/x")).unwrap();
            assert!(tasks.iter().any(|t| t.url.contains("fabric-loader")));
            assert!(tasks.iter().any(|t| t.url.contains("intermediary")));
        });
    }
}
