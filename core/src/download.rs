use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Semaphore};
use tokio::time::Duration;

use crate::events::CoreEvent;

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
    /// "release" | "snapshot" | "old_beta" | "old_alpha"
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

impl std::fmt::Display for VersionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}

/// 获取全部版本列表（版本清单）
pub async fn get_versions() -> Result<Vec<VersionInfo>, String> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| e.to_string())?;

    let manifest: Manifest = client
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    Ok(manifest.versions)
}

#[derive(Deserialize, Debug)]
struct VersionJson {
    #[serde(rename = "assetIndex")]
    asset_index: AssetIndex,
    #[serde(rename = "mainClass")]
    main_class: String,
    downloads: Downloads,
    libraries: Vec<Library>,
}

#[derive(Deserialize, Debug)]
struct AssetIndex {
    id: String,
    url: String,
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
struct Rule {
    action: String,
    os: Option<Os>,
}

#[derive(Deserialize, Debug)]
struct Os {
    name: Option<String>,
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

#[derive(Debug)]
struct DownloadTask {
    url: String,
    path: PathBuf,
    sha1: String,
}

pub async fn run(
    version: &str,
    game_dir: &PathBuf,
    concurrency: usize,
    events: mpsc::Sender<CoreEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()?;

    // 发送初始进度
    let _ = events
        .send(CoreEvent::DownloadProgress(DownloadProgress {
            current: 0,
            total: 100,
            percent: 0,
            current_file: "正在获取版本信息...".to_string(),
        }))
        .await;

    // 1. 下载 version manifest
    println!("[1/5] 获取版本列表...");
    let manifest: Manifest = client
        .get(MANIFEST_URL)
        .send()
        .await?
        .json()
        .await?;
    println!("最新版本: {}", manifest.latest.release);

    // 2. 找到目标版本
    let version_url = manifest
        .versions
        .iter()
        .find(|v| v.id == version)
        .ok_or_else(|| format!("版本 {} 不存在", version))?
        .url
        .clone();
    println!("版本 URL: {}", version_url);

    // 3. 下载版本 JSON
    println!("[2/5] 下载版本配置...");
    let version_response = client.get(&version_url).send().await?;
    let version_json_text = version_response.text().await?;
    let version_json: VersionJson = serde_json::from_str(&version_json_text)?;
    println!("主类: {}", version_json.main_class);

    // 保存 version.json
    let version_json_path = game_dir.join("versions").join(version).join(format!("{}.json", version));
    fs::create_dir_all(version_json_path.parent().unwrap()).await?;
    let mut file = File::create(&version_json_path).await?;
    file.write_all(version_json_text.as_bytes()).await?;
    println!("已保存版本配置: {:?}", version_json_path);

    // 4. 构建下载任务
    let mut tasks = Vec::new();

    // Client JAR
    let client_jar_path = game_dir.join("versions").join(version).join(format!("{}.jar", version));
    tasks.push(DownloadTask {
        url: version_json.downloads.client.url.clone(),
        path: client_jar_path,
        sha1: version_json.downloads.client.sha1.clone(),
    });

    // Libraries
    println!("[3/5] 分析 Libraries...");
    for lib in &version_json.libraries {
        // 检查规则
        if let Some(rules) = &lib.rules {
            let allowed = rules.iter().any(|r| {
                r.action == "allow"
                    && r.os
                        .as_ref()
                        .map_or(true, |os| {
                            os.name
                                .as_ref()
                                .map_or(true, |n| n == "windows" || n == "osx" || n == "linux")
                        })
            });
            if !allowed {
                continue;
            }
        }

        if let Some(downloads) = &lib.downloads {
            if let Some(artifact) = &downloads.artifact {
                let path = game_dir.join("libraries").join(&artifact.path);
                tasks.push(DownloadTask {
                    url: artifact.url.clone(),
                    path,
                    sha1: artifact.sha1.clone(),
                });
            }
        }
    }
    println!("Libraries 数量: {}", tasks.len() - 1);

    // Asset Index
    println!("[4/5] 下载 Asset Index...");
    let asset_index_response = client.get(&version_json.asset_index.url).send().await?;
    let asset_index_text = asset_index_response.text().await?;
    let asset_index_json: AssetIndexJson = serde_json::from_str(&asset_index_text)?;
    let asset_count = asset_index_json.objects.len();
    println!("Asset 数量: {}", asset_count);

    // 保存 asset index JSON（游戏需要这个来定位资源）
    let asset_index_path = game_dir
        .join("assets")
        .join("indexes")
        .join(format!("{}.json", version_json.asset_index.id));
    fs::create_dir_all(asset_index_path.parent().unwrap()).await?;
    let mut file = File::create(&asset_index_path).await?;
    file.write_all(asset_index_text.as_bytes()).await?;
    println!("已保存 Asset Index: {:?}", asset_index_path);

    for (_name, obj) in asset_index_json.objects {
        let hash = &obj.hash;
        let path = game_dir.join("assets").join("objects").join(&hash[..2]).join(hash);
        tasks.push(DownloadTask {
            url: format!("https://resources.download.minecraft.net/{}/{}", &hash[..2], hash),
            path,
            sha1: hash.clone(),
        });
    }

    // 5. 开始下载
    println!("[5/5] 开始下载 (共 {} 个文件)...", tasks.len());

    // 创建进度条
    let m = MultiProgress::new();
    let pb = m.add(ProgressBar::new(tasks.len() as u64));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40}] {pos}/{len} ({percent}%) {msg}")
            .unwrap()
            .progress_chars("#>-"),
    );

    // 过滤已存在的文件
    let total_count = tasks.len();
    let mut tasks_to_download: Vec<DownloadTask> = Vec::new();
    for task in tasks {
        if !task.path.exists() {
            tasks_to_download.push(task);
        } else {
            pb.inc(1);
        }
    }

    let remaining = tasks_to_download.len();
    if remaining == 0 {
        println!("\n所有文件已存在，无需下载！");
        let _ = events
            .send(CoreEvent::DownloadProgress(DownloadProgress {
                current: total_count,
                total: total_count,
                percent: 100,
                current_file: "所有文件已存在".to_string(),
            }))
            .await;
        return Ok(());
    }

    println!("需下载: {} 个文件", remaining);
    pb.set_message("下载中...");

    // 并发下载（信号量限制实际并发数，避免文件句柄耗尽）
    let client = std::sync::Arc::new(client);
    let total = tasks_to_download.len();
    let downloaded = std::sync::atomic::AtomicUsize::new(0);

    // 每个并发约需 2 个文件句柄（网络响应体 + 磁盘写入），限制为 concurrency / 2
    // 但最多不超过 512，避免极端情况下耗尽 Linux 默认 1024 上限
    let max_concurrent_writes = (concurrency / 2).clamp(1, 512);
    let semaphore = std::sync::Arc::new(Semaphore::new(max_concurrent_writes));

    stream::iter(tasks_to_download)
        .map(|task| {
            let client = client.clone();
            let pb = pb.clone();
            let downloaded = &downloaded;
            let events = events.clone();
            let total = total;
            let sem = semaphore.clone();
            async move {
                // 信号量控制实际并发数
                let _permit = sem.acquire().await.unwrap();
                download_file(&client, task, &pb).await;
                let done = downloaded.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let progress = DownloadProgress {
                    current: done + 1,
                    total,
                    percent: ((done + 1) as f64 / total as f64 * 100.0) as u32,
                    current_file: "下载中...".to_string(),
                };
                let _ = events.send(CoreEvent::DownloadProgress(progress)).await;
            }
        })
        .buffer_unordered(concurrency)
        .collect::<()>()
        .await;

    pb.finish_with_message("完成！");
    let _ = events
        .send(CoreEvent::DownloadProgress(DownloadProgress {
            current: total,
            total,
            percent: 100,
            current_file: "下载完成".to_string(),
        }))
        .await;

    Ok(())
}

async fn download_file(client: &Client, task: DownloadTask, pb: &ProgressBar) {
    // 确保目录存在
    if let Some(parent) = task.path.parent() {
        if let Err(e) = fs::create_dir_all(parent).await {
            eprintln!("创建目录失败: {:?} - {}", parent, e);
            return;
        }
    }

    // 重试下载
    let mut retries = 3;
    let bytes = loop {
        match client.get(&task.url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.bytes().await {
                        Ok(b) => break b,
                        Err(_e) if retries > 0 => {
                            retries -= 1;
                            eprintln!("\n读取响应失败 (重试 {}): {}", retries, task.url);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                        Err(e) => {
                            eprintln!("\n读取响应失败: {} - {}", task.url, e);
                            return;
                        }
                    }
                } else if retries > 0 {
                    retries -= 1;
                    eprintln!("\n下载失败 HTTP {} (重试 {}): {}", response.status(), retries, task.url);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                } else {
                    eprintln!("\n下载失败 (HTTP {}): {}", response.status(), task.url);
                    return;
                }
            }
            Err(_e) if retries > 0 => {
                retries -= 1;
                eprintln!("\n请求失败 (重试 {}): {}", retries, task.url);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                eprintln!("\n请求失败: {} - {}", task.url, e);
                return;
            }
        }
    };

    // 校验 SHA1
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    let hash = hex::encode(result);

    if !hash.eq_ignore_ascii_case(&task.sha1) {
        eprintln!("\nSHA1 不匹配: {} (期望: {}, 实际: {})", task.path.display(), task.sha1, hash);
        return;
    }

    // 写入文件
    match File::create(&task.path).await {
        Ok(mut f) => {
            if let Err(e) = f.write_all(&bytes).await {
                eprintln!("\n写入文件失败: {:?} - {}", task.path, e);
                return;
            }
        }
        Err(e) => {
            eprintln!("\n创建文件失败: {:?} - {}", task.path, e);
            return;
        }
    }

    pb.inc(1);
}
