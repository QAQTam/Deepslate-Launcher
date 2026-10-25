use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::sync::mpsc;
use zip::ZipArchive;

use crate::auth::AuthMethod;
use crate::events::{CoreEvent, GameState};

static GAME_PROCESS: Mutex<Option<u32>> = Mutex::new(None);
static GAME_HANDLE: std::sync::LazyLock<tokio::sync::Mutex<Option<Child>>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest.json";

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub version: String,
    pub game_dir: PathBuf,
    pub username: String,
    pub uuid: String,
    pub runtime_dir: PathBuf,
    pub auth_method: AuthMethod,
    pub access_token: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct Manifest {
    latest: Latest,
    versions: Vec<VersionInfo>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct Latest {
    release: String,
}

#[derive(Deserialize, Debug)]
struct VersionInfo {
    id: String,
    url: String,
}

#[derive(Deserialize, Debug, Clone)]
struct VersionJson {
    id: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    #[serde(rename = "assetIndex")]
    asset_index: AssetIndex,
    #[serde(rename = "inheritsFrom")]
    inherits_from: Option<String>,
    downloads: Downloads,
    libraries: Vec<Library>,
    #[serde(rename = "minimumLauncherVersion")]
    minimum_launcher_version: Option<u32>,
    #[serde(rename = "minecraftArguments")]
    minecraft_arguments: Option<String>,
    arguments: Option<Arguments>,
}

#[derive(Deserialize, Debug, Clone)]
struct Arguments {
    game: Option<Vec<GameArgument>>,
    jvm: Option<Vec<JvmArgument>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
#[allow(dead_code)]
enum GameArgument {
    String(String),
    Rule(RuleArgument),
}

#[derive(Deserialize, Debug, Clone)]
struct RuleArgument {
    rules: Vec<Rule>,
    value: serde_json::Value,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum JvmArgument {
    String(String),
    Rule(RuleArgument),
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct Rule {
    action: String,
    os: Option<OsRestriction>,
    features: Option<HashMap<String, bool>>,
}

#[derive(Deserialize, Debug, Clone)]
struct OsRestriction {
    name: Option<String>,
    arch: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct AssetIndex {
    id: String,
    url: String,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct Downloads {
    client: ClientDownload,
    #[serde(rename = "client_mappings")]
    client_mappings: Option<Artifact>,
    server: Option<Artifact>,
    #[serde(rename = "server_mappings")]
    server_mappings: Option<Artifact>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct ClientDownload {
    url: String,
    sha1: String,
    size: u64,
}

#[derive(Deserialize, Debug, Clone)]
struct Artifact {
    url: String,
    sha1: String,
    #[allow(dead_code)]
    size: u64,
    path: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct Library {
    name: String,
    downloads: Option<LibraryDownloads>,
    rules: Option<Vec<Rule>>,
    #[serde(rename = "natives")]
    natives: Option<HashMap<String, String>>,
    #[serde(rename = "extract")]
    extract: Option<ExtractRule>,
}

#[derive(Deserialize, Debug, Clone)]
struct LibraryDownloads {
    artifact: Option<Artifact>,
    classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Deserialize, Debug, Clone)]
struct ExtractRule {
    exclude: Vec<String>,
}

fn get_os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        return "windows";
    } else if cfg!(target_os = "macos") {
        return "osx";
    } else {
        return "linux";
    }
}

fn get_os_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        return "x86_64";
    } else if cfg!(target_arch = "aarch64") {
        return "aarch64";
    } else {
        return "x86";
    }
}

#[derive(Debug, serde::Serialize, Clone, PartialEq)]
pub struct JavaInfo {
    pub path: String,
    pub version: String,
}

impl std::fmt::Display for JavaInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Java {} — {}", self.version, self.path)
    }
}

pub fn scan_java() -> Result<Vec<JavaInfo>, String> {
    let mut java_versions = Vec::new();
    let java_cmd = if cfg!(windows) { "java.exe" } else { "java" };

    // 常见 Java 路径
    let search_paths = if cfg!(windows) {
        vec![
            PathBuf::from(r"C:\Program Files\Java"),
            PathBuf::from(r"C:\Program Files (x86)\Java"),
            PathBuf::from(r"C:\Program Files\Eclipse Adoptium"),
            PathBuf::from(r"C:\Program Files\Eclipse Foundation"),
            PathBuf::from(r"C:\Program Files\Amazon Corretto"),
            PathBuf::from(r"C:\Program Files\Microsoft"),
            PathBuf::from(r"C:\Program Files\Zulu"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/lib/jvm"),
            PathBuf::from("/usr/java"),
            PathBuf::from("/opt/java"),
            PathBuf::from("/opt/jdk"),
            PathBuf::from("/Library/Java/JavaVirtualMachines"),
            dirs::home_dir().map(|h| h.join(".sdkman/candidates/java")).unwrap_or_default(),
            dirs::home_dir().map(|h| h.join(".local/share/applications")).unwrap_or_default(),
        ]
    };

    for base_path in search_paths {
        if !base_path.exists() {
            continue;
        }

        if let Ok(entries) = std::fs::read_dir(&base_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                let java_bin = path.join("bin").join(java_cmd);
                if java_bin.exists() {
                    if let Ok(output) = Command::new(&java_bin).arg("-version").output() {
                        let version_str = if cfg!(windows) {
                            String::from_utf8_lossy(&output.stderr).to_string()
                        } else {
                            String::from_utf8_lossy(&output.stderr).to_string()
                        };
                        let version = extract_java_version(&version_str);
                        java_versions.push(JavaInfo {
                            path: path.to_string_lossy().to_string(),
                            version,
                        });
                    }
                }
            }
        }
    }

    // 按版本号排序（新版在前）
    java_versions.sort_by(|a, b| {
        let a_ver = parse_java_version(&a.version);
        let b_ver = parse_java_version(&b.version);
        b_ver.cmp(&a_ver)
    });

    Ok(java_versions)
}

/// 扫描指定目录，寻找 Java 运行时
/// 支持直接目录（如 jdk-21.0.2）或嵌套目录（如 .sdkman/candidates/java/jdk-21）
pub fn scan_java_dir(dir: &PathBuf) -> Result<Vec<JavaInfo>, String> {
    let mut java_versions = Vec::new();
    let java_cmd = if cfg!(windows) { "java.exe" } else { "java" };

    if !dir.exists() {
        return Err(format!("目录不存在: {}", dir.display()));
    }

    scan_java_dir_recursive(dir, &java_cmd, &mut java_versions)?;

    // 按版本号排序（新版在前）
    java_versions.sort_by(|a, b| {
        let a_ver = parse_java_version(&a.version);
        let b_ver = parse_java_version(&b.version);
        b_ver.cmp(&a_ver)
    });

    Ok(java_versions)
}

fn scan_java_dir_recursive(dir: &PathBuf, java_cmd: &str, results: &mut Vec<JavaInfo>) -> Result<(), String> {
    let java_bin = dir.join("bin").join(java_cmd);
    if java_bin.exists() {
        if let Ok(output) = Command::new(&java_bin).arg("-version").output() {
            let version_str = String::from_utf8_lossy(&output.stderr).to_string();
            let version = extract_java_version(&version_str);
            results.push(JavaInfo {
                path: dir.to_string_lossy().to_string(),
                version,
            });
        }
        return Ok(()); // 找到则不再深入（避免重复）
    }

    // 递归扫描子目录（限制深度避免过深搜索）
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // 避免扫描系统隐藏目录
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') || name == "node_modules" || name == "target" {
                        continue;
                    }
                }
                scan_java_dir_recursive(&path, java_cmd, results)?;
            }
        }
    }

    Ok(())
}

fn extract_java_version(version_output: &str) -> String {
    // 尝试从 version string 中提取版本号
    for line in version_output.lines() {
        if line.contains("version") {
            if let Some(start) = line.find("\"") {
                let rest = &line[start + 1..];
                if let Some(end) = rest.find("\"") {
                    return rest[..end].to_string();
                }
            }
        }
    }
    "Unknown".to_string()
}

fn parse_java_version(version: &str) -> (u32, u32, u32) {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() >= 3 {
        let major: u32 = parts[0].parse().unwrap_or(0);
        let minor: u32 = parts[1].parse().unwrap_or(0);
        let patch: u32 = parts[2].split('-').next().unwrap_or("0").parse().unwrap_or(0);
        (major, minor, patch)
    } else if parts.len() == 2 {
        let major: u32 = parts[0].parse().unwrap_or(0);
        let minor: u32 = parts[1].parse().unwrap_or(0);
        (major, minor, 0)
    } else {
        (0, 0, 0)
    }
}

pub async fn kill_game() -> Result<(), String> {
    // 先获取 PID
    let pid = {
        let pid_guard = GAME_PROCESS.lock().map_err(|e| e.to_string())?;
        pid_guard.ok_or("没有运行中的游戏")?
    };

    // 使用系统命令终止进程
    #[cfg(windows)]
    {
        Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        Command::new("kill").args(["-9", &pid.to_string()]).spawn()
            .map_err(|e| e.to_string())?;
    }

    // 清理状态
    {
        let mut handle_guard = GAME_HANDLE.lock().await;
        *handle_guard = None;
    }
    {
        let mut pid_guard = GAME_PROCESS.lock().map_err(|e| e.to_string())?;
        *pid_guard = None;
    }

    Ok(())
}

fn check_rule(rule: &Rule) -> bool {
    // 默认拒绝
    let action = rule.action.as_str();
    if action != "allow" {
        return false;
    }

    // 检查 OS 限制
    if let Some(os) = &rule.os {
        let os_name = get_os_name();
        let os_arch = get_os_arch();

        if let Some(name) = &os.name {
            if name != os_name && name != "windows" && name != "osx" && name != "linux" {
                return false;
            }
            if name == "osx" && os_name != "osx" {
                return false;
            }
            if name == "windows" && os_name != "windows" {
                return false;
            }
            if name == "linux" && os_name != "linux" {
                return false;
            }
        }

        if let Some(arch) = &os.arch {
            if arch == "x86" && (os_arch != "x86_64" && os_arch != "x86") {
                return false;
            }
        }
    }

    true
}

#[allow(dead_code)]
fn apply_rules(args: &[GameArgument]) -> Vec<String> {
    let mut result = Vec::new();
    for arg in args {
        match arg {
            GameArgument::String(s) => result.push(s.clone()),
            GameArgument::Rule(r) => {
                if r.rules.iter().all(|rule| check_rule(rule)) {
                    match &r.value {
                        serde_json::Value::String(s) => result.push(s.clone()),
                        serde_json::Value::Array(arr) => {
                            for item in arr {
                                if let Some(s) = item.as_str() {
                                    result.push(s.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    result
}

pub async fn launch(
    opt: LaunchOptions,
    events: mpsc::Sender<CoreEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()?;

    // 发送游戏启动事件
    let _ = events
        .send(CoreEvent::GameState(GameState::Launching {
            version: opt.version.clone(),
        }))
        .await;

    // 1. 获取 version manifest
    println!("获取版本列表...");
    let manifest: Manifest = client.get(MANIFEST_URL).send().await?.json().await?;

    // 2. 找到版本 URL
    let version_url = manifest
        .versions
        .iter()
        .find(|v| v.id == opt.version)
        .ok_or_else(|| format!("版本 {} 不存在", opt.version))?
        .url
        .clone();

    // 3. 下载版本 JSON
    println!("下载版本配置...");
    let version_json_text = client.get(&version_url).send().await?.text().await?;
    let version_json: VersionJson = serde_json::from_str(&version_json_text)?;

    // 如果有 inheritsFrom，需要先加载父版本
    let full_version_json = if let Some(parent_id) = &version_json.inherits_from {
        println!("继承版本: {}", parent_id);
        let parent_url = manifest
            .versions
            .iter()
            .find(|v| v.id == *parent_id)
            .ok_or_else(|| format!("父版本 {} 不存在", parent_id))?
            .url
            .clone();
        let parent_json_text = client.get(&parent_url).send().await?.text().await?;
        let parent_json: VersionJson = serde_json::from_str(&parent_json_text)?;

        // 合并：父版本的字段被版本自身的字段覆盖
        merge_version_json(&parent_json, &version_json)
    } else {
        version_json
    };

    // 4. 保存版本 JSON
    let version_json_path = opt.game_dir.join("versions").join(&opt.version).join(format!("{}.json", opt.version));
    fs::create_dir_all(version_json_path.parent().unwrap()).await?;
    let mut file = File::create(&version_json_path).await?;
    file.write_all(version_json_text.as_bytes()).await?;

    // 5. 分析并下载 libraries
    let mut classpath = Vec::new();
    let natives_dir = opt.game_dir.join("natives").join(&opt.version);
    fs::create_dir_all(&natives_dir).await?;

    println!("分析 Libraries...");
    for lib in &full_version_json.libraries {
        // 检查规则
        if let Some(rules) = &lib.rules {
            if !rules.iter().all(|r| check_rule(r)) {
                continue;
            }
        }

        let lib_path = opt.game_dir.join("libraries");

        // 处理 natives
        if let Some(natives) = &lib.natives {
            let os_name = get_os_name();
            if let Some(native_key) = natives.get(os_name) {
                // 下载 classifier native
                if let Some(downloads) = &lib.downloads {
                    if let Some(classifiers) = &downloads.classifiers {
                        if let Some(artifact) = classifiers.get(native_key) {
                            let path_str = artifact.path.as_ref().ok_or("Missing path in classifier artifact")?;
                            let native_jar_path = lib_path.join(path_str);
                            if !native_jar_path.exists() {
                                download_artifact(&client, &artifact.url, &native_jar_path, &artifact.sha1).await?;
                            }
                            // 提取 native
                            extract_natives(&native_jar_path, &natives_dir, lib.extract.as_ref()).await?;
                            classpath.push(native_jar_path);
                            continue;
                        }
                    }
                }
            }
        }

        // 普通 artifact
        if let Some(downloads) = &lib.downloads {
            if let Some(artifact) = &downloads.artifact {
                let path_str = artifact.path.as_ref().ok_or("Missing path in artifact")?;
                let artifact_path = lib_path.join(path_str);
                if !artifact_path.exists() {
                    download_artifact(&client, &artifact.url, &artifact_path, &artifact.sha1).await?;
                }
                classpath.push(artifact_path);
            }
        }
    }

    // 6. 添加游戏 jar（假设已通过 download 命令下载）
    let game_jar = opt.game_dir.join("versions").join(&opt.version).join(format!("{}.jar", opt.version));
    if !game_jar.exists() {
        return Err(format!("游戏客户端不存在: {:?}，请先下载版本", game_jar).into());
    }
    classpath.push(game_jar);

    // 7. 构建 JVM 参数
    let mut jvm_args = Vec::new();

    // Java 25 不需要额外的 --add-exports 和 --add-opens

    // 替换占位符
    let natives_dir_str = natives_dir.to_string_lossy().to_string();
    let classpath_str = classpath
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(if cfg!(windows) { ";" } else { ":" });
    let game_dir_str = opt.game_dir.to_string_lossy().to_string();
    let assets_dir = opt.game_dir.join("assets");
    let assets_dir_str = assets_dir.to_string_lossy().to_string();

    // 处理 arguments.jvm
    if let Some(arguments) = &full_version_json.arguments {
        if let Some(jvm_args_list) = &arguments.jvm {
            for arg in jvm_args_list {
                match arg {
                    JvmArgument::String(s) => {
                        let s = s
                            .replace("${natives_directory}", &natives_dir_str)
                            .replace("${classpath}", &classpath_str)
                            .replace("${launcher_name}", "Obsidian Launcher")
                            .replace("${launcher_version}", "0.1.0");
                        jvm_args.push(s);
                    }
                    JvmArgument::Rule(r) => {
                        if r.rules.iter().all(|rule| check_rule(rule)) {
                            match &r.value {
                                serde_json::Value::String(s) => {
                                    let s = s
                                        .replace("${natives_directory}", &natives_dir_str)
                                        .replace("${classpath}", &classpath_str)
                                        .replace("${launcher_name}", "Obsidian Launcher")
                                        .replace("${launcher_version}", "0.1.0");
                                    jvm_args.push(s);
                                }
                                serde_json::Value::Array(arr) => {
                                    for item in arr {
                                        if let Some(s) = item.as_str() {
                                            let s = s
                                                .replace("${natives_directory}", &natives_dir_str)
                                                .replace("${classpath}", &classpath_str)
                                                .replace("${launcher_name}", "Obsidian Launcher")
                                                .replace("${launcher_version}", "0.1.0");
                                            jvm_args.push(s);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // --- GhastLoader Mod Loader ---
    // 将 GhastLoader Java Agent 注入 JVM 参数，实现事件总线式 mod 加载
    {
        let loader_jar = opt.game_dir.join("ghastloader").join("ghastloader.jar");
        if loader_jar.exists() {
            jvm_args.push("-javaagent".to_string());
            jvm_args.push(loader_jar.to_string_lossy().to_string());
            println!("[GhastLoader] Agent injected: {}", loader_jar.display());
        }
    }

    // 8. 构建游戏参数
    let mut game_args = Vec::new();
    game_args.push("--username".to_string());
    game_args.push(opt.username.clone());
    game_args.push("--version".to_string());
    game_args.push(opt.version.clone());
    game_args.push("--gameDir".to_string());
    game_args.push(game_dir_str.clone());
    game_args.push("--assetsDir".to_string());
    game_args.push(assets_dir_str.clone());
    game_args.push("--assetIndex".to_string());
    game_args.push(full_version_json.asset_index.id.clone());
    game_args.push("--uuid".to_string());
    game_args.push(opt.uuid.clone());
    game_args.push("--accessToken".to_string());

    // 根据认证方式选择 token
    let (access_token, user_type) = match &opt.auth_method {
        AuthMethod::Microsoft => (
            opt.access_token.clone().unwrap_or_else(|| "offline".to_string()),
            "microsoft".to_string(),
        ),
        AuthMethod::Offline => ("offline".to_string(), "mojang".to_string()),
    };
    game_args.push(access_token.clone());
    game_args.push("--userType".to_string());
    game_args.push(user_type.clone());
    game_args.push("--versionType".to_string());
    game_args.push("release".to_string());

    // 处理 minecraftArguments (旧版格式)
    if let Some(mc_args) = &full_version_json.minecraft_arguments {
        for part in mc_args.split_whitespace() {
            let part = part
                .replace("${auth_player_name}", &opt.username)
                .replace("${auth_uuid}", &opt.uuid)
                .replace("${auth_access_token}", &access_token)
                .replace("${game_directory}", &game_dir_str)
                .replace("${assets_root}", &assets_dir_str)
                .replace("${assets_index_name}", &full_version_json.asset_index.id)
                .replace("${version_name}", &opt.version)
                .replace("${version_type}", "release")
                .replace("${user_type}", &user_type);
            game_args.push(part);
        }
    }

    // 9. 执行 java 命令
    let java_path = opt.runtime_dir.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });

    println!("\n启动命令:");
    println!("{} {}", java_path.display(), jvm_args.join(" "));
    println!("主类: {}", full_version_json.main_class);
    println!("游戏参数: {}\n", game_args.join(" "));

    let child = tokio::process::Command::new(&java_path)
        .args(&jvm_args)
        .arg("-cp")
        .arg(&classpath_str)
        .arg(&full_version_json.main_class)
        .args(&game_args)
        .current_dir(&opt.game_dir)
        .spawn()?;

    // 保存进程 ID 和 Handle
    let pid = child.id().ok_or("无法获取进程 ID")?;
    {
        let mut pid_guard = GAME_PROCESS.lock().map_err(|e| e.to_string())?;
        *pid_guard = Some(pid);
    }
    {
        let mut handle_guard = GAME_HANDLE.lock().await;
        *handle_guard = Some(child);
    }
    println!("游戏进程 PID: {}", pid);

    // 发送游戏运行中事件
    let _ = events.send(CoreEvent::GameState(GameState::Running { pid })).await;

    // 等待进程结束 - 先取出 handle 避免跨 await 持有锁
    let exit_status = {
        let mut handle_guard = GAME_HANDLE.lock().await;
        match handle_guard.take() {
            Some(mut child) => {
                // 释放锁后再 await
                drop(handle_guard);
                child.wait().await
            }
            None => return Err("游戏进程丢失".into()),
        }?
    };

    // 游戏退出后清除状态
    {
        let mut pid_guard = GAME_PROCESS.lock().map_err(|e| e.to_string())?;
        *pid_guard = None;
    }
    {
        let mut handle_guard = GAME_HANDLE.lock().await;
        *handle_guard = None;
    }

    // 发送游戏退出事件
    let _ = events
        .send(CoreEvent::GameState(GameState::Stopped {
            exit_code: exit_status.code().unwrap_or(-1),
        }))
        .await;

    if !exit_status.success() {
        return Err(format!("游戏进程退出码: {:?}", exit_status.code()).into());
    }

    Ok(())
}

fn merge_version_json(parent: &VersionJson, child: &VersionJson) -> VersionJson {
    let mut merged = parent.clone();

    // child 的字段覆盖 parent
    merged.id = child.id.clone();
    merged.main_class = child.main_class.clone();
    merged.asset_index = child.asset_index.clone();
    merged.downloads = child.downloads.clone();
    merged.minimum_launcher_version = child.minimum_launcher_version;

    // 合并 libraries (child 的 libraries 覆盖 parent 的)
    merged.libraries = child.libraries.clone();

    // arguments 合并
    if let Some(child_args) = &child.arguments {
        if let Some(parent_args) = &merged.arguments {
            let mut merged_args = parent_args.clone();
            if let Some(child_game) = &child_args.game {
                merged_args.game = Some(child_game.clone());
            }
            if let Some(child_jvm) = &child_args.jvm {
                merged_args.jvm = Some(child_jvm.clone());
            }
            merged.arguments = Some(merged_args);
        }
    } else {
        merged.arguments = child.arguments.clone();
    }

    merged.minecraft_arguments = child.minecraft_arguments.clone();

    merged
}

async fn download_artifact(client: &reqwest::Client, url: &str, path: &PathBuf, expected_sha1: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = client.get(url).send().await?.bytes().await?;

    // 校验 SHA1
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    let hash = hex::encode(result);

    if !hash.eq_ignore_ascii_case(expected_sha1) {
        return Err(format!("SHA1 不匹配: {} (期望: {}, 实际: {})", path.display(), expected_sha1, hash).into());
    }

    fs::create_dir_all(path.parent().unwrap()).await?;
    let mut file = File::create(path).await?;
    file.write_all(&bytes).await?;

    Ok(())
}

async fn extract_natives(jar_path: &PathBuf, dest_dir: &PathBuf, extract_rule: Option<&ExtractRule>) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(jar_path)?;
    let mut archive = ZipArchive::new(file)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = dest_dir.join(file.name());

        // 检查是否在 exclude 列表中
        if let Some(rule) = extract_rule {
            if rule.exclude.iter().any(|ex| file.name().starts_with(ex)) {
                continue;
            }
        }

        // 只提取 .dll, .so, .dylib 文件
        let name = file.name();
        if !name.contains('.') {
            continue;
        }
        let ext = name.rsplit('.').next().unwrap_or("");
        if ext != "dll" && ext != "so" && ext != "dylib" {
            continue;
        }

        std::fs::create_dir_all(outpath.parent().unwrap())?;
        let mut outfile = std::fs::File::create(&outpath)?;
        std::io::copy(&mut file, &mut outfile)?;
    }

    Ok(())
}
