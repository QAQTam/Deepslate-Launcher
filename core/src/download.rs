//! 原版文件下载（versions / libraries / assets）。
//!
//! 具体下载实现 [`download_all`] 也被 Fabric 安装器复用：
//! - 统一的并发控制与进度事件
//! - 3 次重试 + SHA1 校验 + `.part` 原子落盘
//! - 已有文件在提供 SHA1 时校验，通过后才跳过

use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::events::CoreEvent;
use crate::launch::Rule;
use crate::net;

pub const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest.json";

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub current: usize,
    pub total: usize,
    pub percent: u32,
    pub current_file: String,
}

#[derive(Deserialize, Debug)]
pub struct Manifest {
    pub latest: Latest,
    pub versions: Vec<VersionInfo>,
}

#[derive(Deserialize, Debug)]
pub struct Latest {
    pub release: String,
    #[allow(dead_code)]
    pub snapshot: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct VersionInfo {
    pub id: String,
    /// "release" | "snapshot" | "old_beta" | "old_alpha"；
    /// 本地扫描出来的版本用 "installed"
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    /// 本地已安装（不是来自 Mojang manifest）
    #[serde(default)]
    pub local: bool,
    /// 加载器展示名，如 "Fabric 0.16.14"
    #[serde(default)]
    pub loader: Option<String>,
    /// 基础游戏版本（Fabric profile 的 inheritsFrom）
    #[serde(default)]
    pub game_version: Option<String>,
}

impl std::fmt::Display for VersionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 本地 Fabric 版本用「游戏版本 · 加载器」展示，比长 id 更易读
        if self.local {
            if let (Some(loader), Some(game)) = (&self.loader, &self.game_version) {
                return write!(f, "{game} · {loader}");
            }
        }
        write!(f, "{}", self.id)
    }
}

/// 获取全部版本列表（版本清单）
pub async fn get_versions() -> Result<Vec<VersionInfo>, String> {
    let client = net::client()?;
    let manifest: Manifest = net::get_json(&client, MANIFEST_URL).await?;
    Ok(manifest.versions)
}

#[derive(Deserialize, Debug)]
struct VersionJson {
    id: String,
    #[serde(rename = "assetIndex")]
    asset_index: AssetIndex,
    #[serde(default)]
    assets: Option<String>,
    #[serde(rename = "mainClass")]
    main_class: String,
    downloads: Downloads,
    #[serde(default)]
    libraries: Vec<Library>,
}

#[derive(Deserialize, Debug)]
struct AssetIndex {
    id: String,
    url: String,
    sha1: String,
}

#[derive(Deserialize, Debug)]
struct Downloads {
    client: ClientDownload,
}

#[derive(Deserialize, Debug)]
struct ClientDownload {
    url: String,
    sha1: String,
    #[allow(dead_code)]
    size: u64,
}

#[derive(Deserialize, Debug)]
struct Library {
    #[allow(dead_code)]
    name: String,
    downloads: Option<LibraryDownloads>,
    rules: Option<Vec<Rule>>,
}

#[derive(Deserialize, Debug)]
struct LibraryDownloads {
    artifact: Option<Artifact>,
}

#[derive(Deserialize, Debug)]
struct Artifact {
    url: String,
    path: String,
    sha1: String,
    #[allow(dead_code)]
    size: u64,
}

#[derive(Deserialize, Debug)]
struct AssetIndexJson {
    objects: HashMap<String, AssetObject>,
}

#[derive(Deserialize, Debug)]
struct AssetObject {
    hash: String,
    #[allow(dead_code)]
    size: u64,
}

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
}

async fn file_is_usable(task: &DownloadTask) -> bool {
    if !task
        .path
        .symlink_metadata()
        .map(|m| m.file_type().is_file() && m.len() > 0)
        .unwrap_or(false)
    {
        return false;
    }
    let Some(expected) = task.sha1.as_deref().filter(|s| !s.is_empty()) else {
        return true;
    };
    let Ok(mut file) = File::open(&task.path).await else {
        return false;
    };
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 65536];
    loop {
        match file.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return false,
        }
    }
    hex::encode(hasher.finalize()).eq_ignore_ascii_case(expected)
}

fn valid_sha1(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Check the locally installed Mojang files against the hashes in their metadata.
/// Fabric installation uses this before its fast path so missing or damaged assets
/// trigger the normal download-and-repair path.
pub async fn version_installation_is_complete(version: &str, game_dir: &Path) -> bool {
    if crate::launch::validate_version_id(version).is_err() {
        return false;
    }
    let version_json_path = game_dir
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    let Ok(version_bytes) = fs::read(&version_json_path).await else {
        return false;
    };
    let Ok(version_json) = serde_json::from_slice::<VersionJson>(&version_bytes) else {
        return false;
    };
    if version_json.id != version
        || !valid_sha1(&version_json.downloads.client.sha1)
        || !valid_sha1(&version_json.asset_index.sha1)
        || crate::launch::validate_version_id(&version_json.asset_index.id).is_err()
    {
        return false;
    }

    let client_jar = game_dir
        .join("versions")
        .join(version)
        .join(format!("{version}.jar"));
    if !file_is_usable(&DownloadTask {
        url: String::new(),
        path: client_jar,
        sha1: Some(version_json.downloads.client.sha1),
    })
    .await
    {
        return false;
    }

    // Mojang libraries with platform rules are checked only when applicable.
    for library in &version_json.libraries {
        if !library_allowed(library.rules.as_deref()) {
            continue;
        }
        let Some(artifact) = library.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        if !valid_sha1(&artifact.sha1) {
            return false;
        }
        let Ok(relative) = crate::launch::safe_library_path(&artifact.path) else {
            return false;
        };
        if !file_is_usable(&DownloadTask {
            url: String::new(),
            path: game_dir.join("libraries").join(relative),
            sha1: Some(artifact.sha1.clone()),
        })
        .await
        {
            return false;
        }
    }

    let asset_index_path = game_dir
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", version_json.asset_index.id));
    let Ok(asset_index_bytes) = fs::read(&asset_index_path).await else {
        return false;
    };
    let index_hash = hex::encode(Sha1::digest(&asset_index_bytes));
    if !index_hash.eq_ignore_ascii_case(&version_json.asset_index.sha1) {
        return false;
    }
    let Ok(asset_index) = serde_json::from_slice::<AssetIndexJson>(&asset_index_bytes) else {
        return false;
    };
    for object in asset_index.objects.values() {
        if !valid_sha1(&object.hash) {
            return false;
        }
        let path = game_dir
            .join("assets")
            .join("objects")
            .join(&object.hash[..2])
            .join(&object.hash);
        if !file_is_usable(&DownloadTask {
            url: String::new(),
            path,
            sha1: Some(object.hash.clone()),
        })
        .await
        {
            return false;
        }
    }
    true
}

/// 并发下载一组文件，返回实际下载（不含跳过）的数量。
pub async fn download_all(
    tasks: Vec<DownloadTask>,
    concurrency: usize,
    events: mpsc::Sender<CoreEvent>,
    label: &str,
) -> Result<usize, String> {
    let total = tasks.len();
    let client = std::sync::Arc::new(net::client()?);
    let downloaded = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let label = label.to_string();

    // 已存在的文件先计入进度，避免进度条从 0 开始却瞬间跳到 80%
    let mut pending = Vec::with_capacity(total);
    let mut already = 0usize;
    for task in tasks {
        if file_is_usable(&task).await {
            already += 1;
        } else {
            pending.push(task);
        }
    }

    let emit = |current: usize, events: &mpsc::Sender<CoreEvent>, text: &str| {
        let percent = if total == 0 {
            100
        } else {
            (current as f64 / total as f64 * 100.0).round() as u32
        };
        let event = CoreEvent::DownloadProgress(DownloadProgress {
            current,
            total,
            percent,
            current_file: text.to_string(),
        });
        let events = events.clone();
        async move {
            let _ = events.send(event).await;
        }
    };

    emit(already, &events, &label).await;

    if pending.is_empty() {
        emit(total, &events, "所有文件已存在").await;
        return Ok(0);
    }

    let workers = concurrency.max(1);
    let results: Vec<Result<(), String>> = stream::iter(pending)
        .map(|task| {
            let client = client.clone();
            let events = events.clone();
            let downloaded = downloaded.clone();
            let completed = completed.clone();
            let label = label.clone();
            async move {
                let result = download_one(&client, &task).await;
                if result.is_ok() {
                    downloaded.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let done = completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let current = already + done;
                let percent = if total == 0 {
                    100
                } else {
                    (current as f64 / total as f64 * 100.0).round() as u32
                };
                let _ = events
                    .send(CoreEvent::DownloadProgress(DownloadProgress {
                        current,
                        total,
                        percent,
                        current_file: label,
                    }))
                    .await;
                result
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    let failures: Vec<String> = results.into_iter().filter_map(|r| r.err()).collect();
    if !failures.is_empty() {
        return Err(format!(
            "{} 个文件下载失败（首个错误: {}）",
            failures.len(),
            failures[0]
        ));
    }

    emit(total, &events, "下载完成").await;
    Ok(downloaded.load(std::sync::atomic::Ordering::SeqCst))
}

async fn download_one(client: &Client, task: &DownloadTask) -> Result<(), String> {
    let mut last_error = String::new();
    // net::get_bytes 已负责传输层重试；这里只额外重试「SHA1 不匹配」这种内容错误。
    for attempt in 0..2u32 {
        let bytes = net::get_bytes(client, &task.url).await?;

        if let Some(expected) = task.sha1.as_deref().filter(|s| !s.is_empty()) {
            let mut hasher = Sha1::new();
            hasher.update(&bytes);
            let hash = hex::encode(hasher.finalize());
            if !hash.eq_ignore_ascii_case(expected) {
                last_error = format!(
                    "SHA1 不匹配: {} (期望: {expected}, 实际: {hash})",
                    task.path.display()
                );
                if attempt == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                continue;
            }
        }

        if let Some(parent) = task.path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
        }
        let tmp = {
            let mut name = task.path.as_os_str().to_os_string();
            name.push(format!(".{}.part", uuid::Uuid::new_v4()));
            PathBuf::from(name)
        };
        let mut file = File::create(&tmp)
            .await
            .map_err(|e| format!("创建文件失败 {}: {e}", tmp.display()))?;
        file.write_all(&bytes)
            .await
            .map_err(|e| format!("写入文件失败 {}: {e}", tmp.display()))?;
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&tmp, &task.path)
            .await
            .map_err(|e| format!("替换文件失败 {}: {e}", task.path.display()))?;
        return Ok(());
    }
    Err(last_error)
}

/// 规则求值：Mojang 语义，最后一条命中者生效。
fn library_allowed(rules: Option<&[Rule]>) -> bool {
    crate::launch::rules_allow(rules, &Default::default())
}

/// 下载并安装一个原版版本（版本 JSON / 客户端 jar / 库 / 资源）。
pub async fn run(
    version: &str,
    game_dir: &Path,
    concurrency: usize,
    events: mpsc::Sender<CoreEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    crate::launch::validate_version_id(version)?;
    let client = net::client()?;

    let _ = events
        .send(CoreEvent::DownloadProgress(DownloadProgress {
            current: 0,
            total: 100,
            percent: 0,
            current_file: "正在获取版本信息...".to_string(),
        }))
        .await;

    println!("[1/5] 获取版本列表...");
    let manifest: Manifest = net::get_json(&client, MANIFEST_URL).await?;
    println!("最新版本: {}", manifest.latest.release);

    let version_url = manifest
        .versions
        .iter()
        .find(|v| v.id == version)
        .ok_or_else(|| format!("版本 {version} 不存在"))?
        .url
        .clone();
    println!("版本 URL: {version_url}");

    println!("[2/5] 下载版本配置...");
    let version_json_text = net::get_text(&client, &version_url).await?;
    let version_json: VersionJson = serde_json::from_str(&version_json_text)?;
    if version_json.id != version
        || !valid_sha1(&version_json.downloads.client.sha1)
        || !valid_sha1(&version_json.asset_index.sha1)
    {
        return Err(format!("版本 {version} 的官方配置标识或资源索引摘要无效").into());
    }
    crate::launch::validate_version_id(&version_json.asset_index.id)?;
    println!("主类: {}", version_json.main_class);

    let version_json_path = game_dir
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    fs::create_dir_all(version_json_path.parent().unwrap()).await?;
    let mut file = File::create(&version_json_path).await?;
    file.write_all(version_json_text.as_bytes()).await?;
    println!("已保存版本配置: {version_json_path:?}");

    // 4. 构建下载任务
    let mut tasks: Vec<DownloadTask> = Vec::new();

    let client_jar_path = game_dir
        .join("versions")
        .join(version)
        .join(format!("{version}.jar"));
    tasks.push(DownloadTask {
        url: version_json.downloads.client.url.clone(),
        path: client_jar_path,
        sha1: Some(version_json.downloads.client.sha1.clone()),
    });

    println!("[3/5] 分析 Libraries...");
    let library_root = game_dir.join("libraries");
    for lib in &version_json.libraries {
        if !library_allowed(lib.rules.as_deref()) {
            continue;
        }
        if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            if !valid_sha1(&artifact.sha1) {
                return Err(format!("游戏库 {} 的 SHA1 格式无效", lib.name).into());
            }
            tasks.push(DownloadTask {
                url: artifact.url.clone(),
                path: library_root.join(crate::launch::safe_library_path(&artifact.path)?),
                sha1: Some(artifact.sha1.clone()),
            });
        }
    }
    println!("Libraries 数量: {}", tasks.len() - 1);

    println!("[4/5] 下载 Asset Index...");
    let asset_index_text = net::get_text(&client, &version_json.asset_index.url).await?;
    let actual_index_sha1 = hex::encode(Sha1::digest(asset_index_text.as_bytes()));
    if !actual_index_sha1.eq_ignore_ascii_case(&version_json.asset_index.sha1) {
        return Err(format!(
            "Asset Index SHA1 不匹配 (期望: {}, 实际: {actual_index_sha1})",
            version_json.asset_index.sha1
        )
        .into());
    }
    let asset_index_json: AssetIndexJson = serde_json::from_str(&asset_index_text)?;
    let asset_count = asset_index_json.objects.len();
    println!("Asset 数量: {asset_count}");

    let asset_index_path = game_dir
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", version_json.asset_index.id));
    fs::create_dir_all(asset_index_path.parent().unwrap()).await?;
    let mut file = File::create(&asset_index_path).await?;
    file.write_all(asset_index_text.as_bytes()).await?;
    println!("已保存 Asset Index: {asset_index_path:?}");

    let objects_dir = game_dir.join("assets").join("objects");
    for obj in asset_index_json.objects.values() {
        let hash = &obj.hash;
        if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("资源哈希格式非法".into());
        }
        tasks.push(DownloadTask {
            url: format!(
                "https://resources.download.minecraft.net/{}/{}",
                &hash[..2],
                hash
            ),
            path: objects_dir.join(&hash[..2]).join(hash),
            sha1: Some(hash.clone()),
        });
    }

    println!("[5/5] 开始下载 (共 {} 个文件)...", tasks.len());
    download_all(tasks, concurrency, events.clone(), "下载中...").await?;

    // 旧版（pre-1.6）需要把资源对象铺平成 virtual/legacy 目录
    if matches!(
        version_json.assets.as_deref(),
        Some("pre-1.6") | Some("legacy")
    ) {
        println!("生成 legacy 虚拟资源目录...");
        if let Err(e) = materialize_virtual_assets(
            game_dir,
            &version_json.asset_index.id,
            &asset_index_json,
            &objects_dir,
        )
        .await
        {
            eprintln!("生成 legacy 资源失败（可能影响旧版声音/语言）: {e}");
        }
    }

    Ok(())
}

async fn materialize_virtual_assets(
    game_dir: &Path,
    index_id: &str,
    index: &AssetIndexJson,
    objects_dir: &Path,
) -> Result<(), String> {
    let virtual_dir = game_dir.join("assets").join("virtual").join(index_id);
    fs::create_dir_all(&virtual_dir)
        .await
        .map_err(|e| e.to_string())?;
    for (name, obj) in &index.objects {
        if obj.hash.len() < 2 {
            continue;
        }
        let src = objects_dir.join(&obj.hash[..2]).join(&obj.hash);
        let dst = virtual_dir.join(name);
        if dst.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        // 硬链接省空间；跨文件系统时回退到复制
        if fs::hard_link(&src, &dst).await.is_err() {
            let _ = fs::copy(&src, &dst).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn corrupted_existing_file_is_not_accepted() {
        let root = std::env::temp_dir().join(format!("download-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("file.jar");
        std::fs::write(&path, b"corrupt").unwrap();
        let (events, _rx) = mpsc::channel(10);
        let result = download_all(
            vec![DownloadTask {
                url: "invalid://test".into(),
                path,
                sha1: Some("0000000000000000000000000000000000000000".into()),
            }],
            1,
            events,
            "test",
        )
        .await;
        assert!(result.is_err(), "corrupted cached file must not be skipped");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn valid_existing_file_is_skipped_without_network() {
        let root = std::env::temp_dir().join(format!("download-valid-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("file.jar");
        std::fs::write(&path, b"abc").unwrap();
        let (events, _rx) = mpsc::channel(10);
        let result = download_all(
            vec![DownloadTask {
                url: "invalid://test".into(),
                path,
                sha1: Some("a9993e364706816aba3e25717850c26c9cd0d89d".into()),
            }],
            1,
            events,
            "test",
        )
        .await;
        assert_eq!(result.unwrap(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn installed_version_requires_valid_hashes_for_client_libraries_and_assets() {
        fn sha1_hex(bytes: &[u8]) -> String {
            hex::encode(Sha1::digest(bytes))
        }

        let root = std::env::temp_dir().join(format!("version-complete-{}", uuid::Uuid::new_v4()));
        let version = "test-version";
        assert!(!version_installation_is_complete("../outside", &root).await);
        let client = b"client";
        let library = b"library";
        let asset = b"asset";
        let asset_hash = sha1_hex(asset);
        let asset_index = format!(
            r#"{{"objects":{{"test":{{"hash":"{asset_hash}","size":{}}}}}}}"#,
            asset.len()
        );
        let asset_index_hash = sha1_hex(asset_index.as_bytes());
        let version_json = format!(
            r#"{{
                "id":"{version}",
                "assetIndex":{{"id":"test-index","url":"unused","sha1":"{asset_index_hash}"}},
                "mainClass":"example.Main",
                "downloads":{{"client":{{"url":"unused","sha1":"{}","size":{}}}}},
                "libraries":[{{"name":"example:library:1","downloads":{{"artifact":{{
                    "url":"unused","path":"example/library/1/library-1.jar","sha1":"{}","size":{}
                }}}}}}]
            }}"#,
            sha1_hex(client),
            client.len(),
            sha1_hex(library),
            library.len()
        );

        let version_dir = root.join("versions").join(version);
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::write(version_dir.join(format!("{version}.json")), version_json).unwrap();
        std::fs::write(version_dir.join(format!("{version}.jar")), client).unwrap();
        let library_path = root.join("libraries/example/library/1/library-1.jar");
        std::fs::create_dir_all(library_path.parent().unwrap()).unwrap();
        std::fs::write(&library_path, library).unwrap();
        let index_path = root.join("assets/indexes/test-index.json");
        std::fs::create_dir_all(index_path.parent().unwrap()).unwrap();
        std::fs::write(&index_path, &asset_index).unwrap();
        let asset_path = root
            .join("assets/objects")
            .join(&asset_hash[..2])
            .join(&asset_hash);
        std::fs::create_dir_all(asset_path.parent().unwrap()).unwrap();
        std::fs::write(&asset_path, asset).unwrap();

        assert!(version_installation_is_complete(version, &root).await);
        let client_path = version_dir.join(format!("{version}.jar"));
        std::fs::write(&client_path, b"damaged").unwrap();
        assert!(!version_installation_is_complete(version, &root).await);
        std::fs::write(&client_path, client).unwrap();

        std::fs::write(&library_path, b"damaged").unwrap();
        assert!(!version_installation_is_complete(version, &root).await);
        std::fs::write(&library_path, library).unwrap();

        std::fs::write(&asset_path, b"damaged").unwrap();
        assert!(!version_installation_is_complete(version, &root).await);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_rules_allow_and_disallow() {
        let allow_all = Rule {
            action: "allow".into(),
            ..Default::default()
        };
        let deny_other = Rule {
            action: "disallow".into(),
            os: Some(crate::launch::OsRestriction {
                name: Some(if crate::launch::get_os_name() == "windows" {
                    "linux".into()
                } else {
                    "windows".into()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(library_allowed(Some(&[allow_all.clone(), deny_other])));
        assert!(!library_allowed(Some(&[
            allow_all,
            Rule {
                action: "disallow".into(),
                os: Some(crate::launch::OsRestriction {
                    name: Some(crate::launch::get_os_name().into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ])));
    }

    #[test]
    fn version_info_display_uses_loader_tag() {
        let v = VersionInfo {
            id: "fabric-loader-0.16.14-1.21.1".into(),
            kind: "installed".into(),
            url: String::new(),
            release_time: String::new(),
            local: true,
            loader: Some("Fabric 0.16.14".into()),
            game_version: Some("1.21.1".into()),
        };
        assert_eq!(v.to_string(), "1.21.1 · Fabric 0.16.14");
    }
}
