//! 版本解析与游戏启动。
//!
//! 职责：
//! - 沿 `inheritsFrom` 加载并合并整条版本链（原版 ↔ Fabric 等加载器）
//! - 求值 Mojang 的 `rules`（操作系统 / 架构 / 功能开关）与参数占位符
//! - 下载缺失的库 / natives，组装 classpath
//! - 生成可检查的 [`LaunchPlan`]，再由 [`launch`] 真正拉起进程
//!
//! 设计要点：参数构造（[`prepare`]）与进程管理（[`launch`]）分离，
//! 便于测试、排查「启动命令到底长什么样」，也便于 GUI 未来展示命令。

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::sync::mpsc;
mod natives;
use natives::extract_natives;

use crate::auth::AuthMethod;
use crate::branding;
use crate::download::{self, DownloadTask};
use crate::events::{CoreEvent, GameState};
use crate::game_options::{self, GameDefaults};
use crate::net;

static GAME_PROCESS: Mutex<Option<u32>> = Mutex::new(None);
static GAME_HANDLE: std::sync::LazyLock<tokio::sync::Mutex<Option<Child>>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));
static BRIDGE_COMMANDS: std::sync::LazyLock<
    tokio::sync::Mutex<Option<(String, mpsc::Sender<crate::bridge::BridgeCommand>)>>,
> = std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));

/// Ask the managed client to save and exit through Minecraft's normal lifecycle.
pub async fn quit_managed_game() -> Result<(), String> {
    let sender = BRIDGE_COMMANDS
        .lock()
        .await
        .as_ref()
        .map(|(_, sender)| sender.clone())
        .ok_or("没有受管游戏会话")?;
    sender
        .send(crate::bridge::BridgeCommand::Quit)
        .await
        .map_err(|_| "Bridge 会话已断开".into())
}

/// Push the launcher-owned friend state into the active game for this account.
pub async fn sync_managed_friends(
    profile_id: &str,
    snapshot: crate::friends::FriendSnapshot,
) -> Result<bool, String> {
    let profile_id = uuid::Uuid::parse_str(profile_id)
        .map_err(|_| "档案 UUID 无效，无法同步好友数据".to_string())?
        .hyphenated()
        .to_string();
    let session = BRIDGE_COMMANDS.lock().await.clone();
    let Some((active_profile, sender)) = session else {
        return Ok(false);
    };
    if active_profile != profile_id {
        return Ok(false);
    }
    sender
        .send(crate::bridge::BridgeCommand::SyncFriends {
            profile_id: active_profile,
            snapshot,
        })
        .await
        .map_err(|_| "Bridge 好友同步通道已断开".to_string())?;
    Ok(true)
}

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest.json";
const LAUNCHER_NAME: &str = "Deepslate Launcher";
const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct LaunchOptions {
    pub version: String,
    /// 文件目录：versions/libraries/assets/natives 的共享根
    pub game_dir: PathBuf,
    /// 实例目录（--gameDir）：saves/options/logs 落盘位置。
    /// 版本隔离时为 root/versions/{version}，否则与 game_dir 相同
    pub instance_dir: PathBuf,
    /// 地图库配置；持锁直到游戏退出，防止并发修改 live 地图。
    pub map_config: Option<crate::config::AppConfig>,
    pub username: String,
    pub uuid: String,
    pub runtime_dir: PathBuf,
    pub auth_method: AuthMethod,
    pub access_token: Option<String>,
    /// 缺失库的并发下载数（通常为 AppConfig.download_concurrency）
    pub download_concurrency: usize,
    /// 启动后直接进入存档 / 服务器（26.3 的 quickPlay 参数）
    pub quick_play: Option<QuickPlayTarget>,
    /// Bridge 管理的地图；不可同时使用原版 Quick Play。
    pub managed_world: Option<crate::bridge::ManagedTarget>,
    /// 标题界面左下角追加的启动器名称（None = 关闭）
    pub branding: Option<String>,
    /// 首次启动写入中文 / 跳过辅助功能引导
    pub apply_game_defaults: bool,
    // 26.3 launch switches
    pub disable_multiplayer: bool,
    pub disable_chat: bool,
    pub fullscreen: bool,
    pub window_width: u32,
    pub window_height: u32,
    pub graphics_backend: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickPlayTarget {
    /// 存档文件夹名
    Singleplayer(String),
    /// host 或 host:port
    Multiplayer(String),
    /// Realms id
    Realms(String),
}

#[derive(Debug, Clone)]
pub struct JvmTuning {
    /// `default` leaves the JDK-selected collector unchanged; `zgc` enables Java 25+ ZGC.
    pub gc: String,
    /// Zero keeps the JVM's ergonomic max heap.
    pub max_memory_mb: u32,
    /// Java 25+ compact object headers.
    pub compact_object_headers: bool,
}

fn apply_enhanced_jvm_tuning(
    args: &mut Vec<String>,
    tuning: &JvmTuning,
    java_major: Option<u32>,
) -> Result<(), String> {
    if !matches!(
        tuning.max_memory_mb,
        0 | 2048 | 4096 | 6144 | 8192 | 12288 | 16384 | 24576 | 32768
    ) {
        return Err("增强模式最大堆内存配置无效".into());
    }
    if !matches!(tuning.gc.as_str(), "default" | "zgc") {
        return Err("增强模式 GC 配置无效".into());
    }
    if tuning.gc == "zgc" && !java_major.is_some_and(|major| major >= 25) {
        return Err("ZGC 方案要求已识别的 Java 25 或更高版本".into());
    }
    if tuning.compact_object_headers && !java_major.is_some_and(|major| major >= 25) {
        return Err("紧凑对象头要求已识别的 Java 25 或更高版本".into());
    }

    if tuning.max_memory_mb != 0 {
        args.retain(|arg| !arg.starts_with("-Xmx"));
        args.push(format!("-Xmx{}M", tuning.max_memory_mb));
    }
    if tuning.gc == "zgc" {
        args.retain(|arg| {
            !matches!(
                arg.as_str(),
                "-XX:+UseG1GC" | "-XX:-UseG1GC" | "-XX:+UseZGC" | "-XX:-UseZGC"
            )
        });
        args.push("-XX:+UseZGC".into());
    }
    if tuning.compact_object_headers {
        args.retain(|arg| {
            !matches!(
                arg.as_str(),
                "-XX:+UseCompactObjectHeaders" | "-XX:-UseCompactObjectHeaders"
            )
        });
        args.push("-XX:+UseCompactObjectHeaders".into());
    }
    Ok(())
}

fn append_quick_play_args(args: &mut Vec<String>, target: Option<&QuickPlayTarget>) {
    let Some(target) = target else {
        return;
    };
    match target {
        QuickPlayTarget::Singleplayer(id) if !id.trim().is_empty() => {
            args.push("--quickPlaySingleplayer".to_string());
            args.push(id.clone());
        }
        QuickPlayTarget::Multiplayer(address) if !address.trim().is_empty() => {
            args.push("--quickPlayMultiplayer".to_string());
            args.push(address.trim().to_string());
        }
        QuickPlayTarget::Realms(id) if !id.trim().is_empty() => {
            args.push("--quickPlayRealms".to_string());
            args.push(id.trim().to_string());
        }
        _ => {}
    }
}

fn append_launch_switches(
    args: &mut Vec<String>,
    disable_multiplayer: bool,
    disable_chat: bool,
    fullscreen: bool,
    width: u32,
    height: u32,
    graphics_backend: &str,
) {
    if disable_multiplayer {
        args.push("--disableMultiplayer".to_string());
    }
    if disable_chat {
        args.push("--disableChat".to_string());
    }
    if fullscreen {
        args.push("--fullscreen".to_string());
    }
    if (320..=7680).contains(&width) {
        args.push("--width".to_string());
        args.push(width.to_string());
    }
    if (240..=4320).contains(&height) {
        args.push("--height".to_string());
        args.push(height.to_string());
    }
    if matches!(graphics_backend, "opengl" | "vulkan") {
        args.push("--graphicsBackend".to_string());
        args.push(graphics_backend.to_string());
    }
}

// ---------------------------------------------------------------------------
// version JSON 模型
// ---------------------------------------------------------------------------

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

#[derive(Deserialize, Debug, Clone, Default)]
pub struct VersionJson {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "mainClass", default)]
    pub main_class: String,
    #[serde(rename = "assetIndex", default)]
    pub asset_index: Option<AssetIndex>,
    #[serde(rename = "inheritsFrom", default)]
    pub inherits_from: Option<String>,
    #[serde(default)]
    pub downloads: Option<Downloads>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(rename = "minimumLauncherVersion", default)]
    pub minimum_launcher_version: Option<u32>,
    #[serde(rename = "minecraftArguments", default)]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    /// 显式指定使用哪个版本的客户端 jar（如旧版 Forge）
    #[serde(rename = "jar", default)]
    pub jar: Option<String>,
    /// "release" / "snapshot" / ...
    #[serde(rename = "type", default)]
    pub version_type: Option<String>,
    /// 旧版资源索引代号（"pre-1.6" / "legacy" 等）
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersion>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct JavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(rename = "majorVersion", default)]
    pub major_version: Option<u32>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Option<Vec<GameArgument>>,
    #[serde(default)]
    pub jvm: Option<Vec<JvmArgument>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum GameArgument {
    String(String),
    Rule(RuleArgument),
}

#[derive(Deserialize, Debug, Clone)]
pub struct RuleArgument {
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub value: serde_json::Value,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum JvmArgument {
    String(String),
    Rule(RuleArgument),
}

/// Mojang 的规则条目。`rules` 按顺序求值，**最后一条命中的规则生效**。
#[derive(Deserialize, Debug, Clone, Default)]
pub struct Rule {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRestriction>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct OsRestriction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    /// 形如 `^10\\.` 的正则；当前只做「存在即忽略」处理（极少数库才会用到）
    #[serde(default)]
    pub version: Option<String>,
}

/// 启动器自身支持的 feature 开关。默认全部为 false（与原版启动器一致）。
pub type FeatureSet = HashMap<String, bool>;

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AssetIndex {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Downloads {
    #[serde(default)]
    pub client: Option<ClientDownload>,
    #[serde(rename = "client_mappings", default)]
    pub client_mappings: Option<Artifact>,
    #[serde(default)]
    pub server: Option<Artifact>,
    #[serde(rename = "server_mappings", default)]
    pub server_mappings: Option<Artifact>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ClientDownload {
    pub url: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Artifact {
    pub url: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    /// maven 仓库根地址（Fabric / 旧版 Forge 的坐标格式库）
    #[serde(default)]
    pub url: Option<String>,
    /// 坐标格式库附带的顶层 sha1（Fabric profile JSON 的写法）
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    #[serde(rename = "natives", default)]
    pub natives: Option<HashMap<String, String>>,
    #[serde(rename = "extract", default)]
    pub extract: Option<ExtractRule>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ExtractRule {
    #[serde(default)]
    pub exclude: Vec<String>,
}

// ---------------------------------------------------------------------------
// 平台与规则
// ---------------------------------------------------------------------------

pub fn get_os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Mojang 规则里的 `os.arch` 取值（注意不是 Rust 的 target_arch 名称）。
fn get_mojang_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "unknown"
    }
}

/// 旧版 natives 分类器里 `${arch}` 的替换值（32 / 64）。
fn natives_arch_bits() -> &'static str {
    if cfg!(target_arch = "x86") {
        "32"
    } else {
        "64"
    }
}

fn rule_matches(rule: &Rule, features: &FeatureSet) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if name != get_os_name() {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if arch != get_mojang_arch() {
                return false;
            }
        }
        // os.version 是 Windows 版本正则；Linux/macOS 上不参与匹配。
        // 目前没有必须依赖它的库，忽略以避免引入正则依赖。
    }
    if let Some(required) = &rule.features {
        for (key, expected) in required {
            let actual = features.get(key).copied().unwrap_or(false);
            if actual != *expected {
                return false;
            }
        }
    }
    true
}

/// 按 Mojang 语义求值一组规则：无规则 = 允许；否则逐条匹配，最后一条命中者决定结果。
pub fn rules_allow(rules: Option<&[Rule]>, features: &FeatureSet) -> bool {
    let Some(rules) = rules else {
        return true;
    };
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if !rule_matches(rule, features) {
            continue;
        }
        allowed = rule.action == "allow";
    }
    allowed
}

// ---------------------------------------------------------------------------
// 参数占位符
// ---------------------------------------------------------------------------

/// 一次启动需要的所有占位符取值。
pub struct ArgContext {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
    pub version_type: String,
    pub version_name: String,
    pub game_directory: String,
    pub assets_root: String,
    pub game_assets: String,
    pub assets_index_name: String,
    pub natives_directory: String,
    pub launcher_name: String,
    pub launcher_version: String,
    pub classpath: String,
    pub classpath_separator: String,
    pub library_directory: String,
    pub user_properties: String,
}

impl ArgContext {
    fn substitute(&self, input: &str) -> String {
        let pairs: [(&str, &str); 24] = [
            ("${auth_player_name}", &self.username),
            ("${auth_uuid}", &self.uuid),
            ("${auth_access_token}", &self.access_token),
            ("${auth_session}", &self.access_token),
            ("${clientid}", ""),
            ("${auth_xuid}", ""),
            ("${user_type}", &self.user_type),
            ("${version_type}", &self.version_type),
            ("${version_name}", &self.version_name),
            ("${game_directory}", &self.game_directory),
            ("${assets_root}", &self.assets_root),
            ("${game_assets}", &self.game_assets),
            ("${assets_index_name}", &self.assets_index_name),
            ("${natives_directory}", &self.natives_directory),
            ("${launcher_name}", &self.launcher_name),
            ("${launcher_version}", &self.launcher_version),
            ("${classpath}", &self.classpath),
            ("${classpath_separator}", &self.classpath_separator),
            ("${library_directory}", &self.library_directory),
            ("${user_properties}", &self.user_properties),
            ("${quickPlayPath}", ""),
            ("${quickPlaySingleplayer}", ""),
            ("${quickPlayMultiplayer}", ""),
            ("${quickPlayRealms}", ""),
        ];
        let mut out = input.to_string();
        for (key, value) in pairs {
            if out.contains(key) {
                out = out.replace(key, value);
            }
        }
        out
    }
}

fn push_argument(value: &serde_json::Value, ctx: &ArgContext, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(ctx.substitute(s)),
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    out.push(ctx.substitute(s));
                }
            }
        }
        _ => {}
    }
}

fn expand_game_arguments(
    json: &VersionJson,
    ctx: &ArgContext,
    features: &FeatureSet,
) -> Vec<String> {
    let mut out = Vec::new();
    let Some(arguments) = &json.arguments else {
        return out;
    };
    let Some(game) = &arguments.game else {
        return out;
    };
    for arg in game {
        match arg {
            GameArgument::String(s) => out.push(ctx.substitute(s)),
            GameArgument::Rule(rule) => {
                if rules_allow(Some(&rule.rules), features) {
                    push_argument(&rule.value, ctx, &mut out);
                }
            }
        }
    }
    out
}

fn expand_jvm_arguments(
    json: &VersionJson,
    ctx: &ArgContext,
    features: &FeatureSet,
) -> Vec<String> {
    let mut out = Vec::new();
    let Some(arguments) = &json.arguments else {
        return out;
    };
    let Some(jvm) = &arguments.jvm else {
        return out;
    };
    for arg in jvm {
        match arg {
            JvmArgument::String(s) => out.push(ctx.substitute(s)),
            JvmArgument::Rule(rule) => {
                if rules_allow(Some(&rule.rules), features) {
                    push_argument(&rule.value, ctx, &mut out);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Java 扫描
// ---------------------------------------------------------------------------

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

    let search_paths = if cfg!(windows) {
        vec![
            PathBuf::from(r"C:\Program Files\Java"),
            PathBuf::from(r"C:\Program Files (x86)\Java"),
            PathBuf::from(r"C:\Program Files\Eclipse Adoptium"),
            PathBuf::from(r"C:\Program Files\Eclipse Foundation"),
            PathBuf::from(r"C:\Program Files\Amazon Corretto"),
            PathBuf::from(r"C:\Program Files\Microsoft"),
            PathBuf::from(r"C:\Program Files\Zulu"),
            dirs::data_dir()
                .map(|d| d.join("Programs").join("Eclipse Adoptium"))
                .unwrap_or_default(),
        ]
    } else {
        vec![
            PathBuf::from("/usr/lib/jvm"),
            PathBuf::from("/usr/java"),
            PathBuf::from("/opt/java"),
            PathBuf::from("/opt/jdk"),
            PathBuf::from("/Library/Java/JavaVirtualMachines"),
            dirs::home_dir()
                .map(|h| h.join(".sdkman/candidates/java"))
                .unwrap_or_default(),
            dirs::home_dir()
                .map(|h| h.join(".local/share/applications"))
                .unwrap_or_default(),
        ]
    };

    for base_path in search_paths {
        if !base_path.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&base_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                // macOS 的 JDK 是 bundle：.../Contents/Home/bin/java
                let home = if path.join("Contents").join("Home").exists() {
                    path.join("Contents").join("Home")
                } else {
                    path
                };
                let java_bin = home.join("bin").join(java_cmd);
                if java_bin.exists() {
                    if let Ok(output) = Command::new(&java_bin).arg("-version").output() {
                        let version_str = String::from_utf8_lossy(&output.stderr).to_string();
                        java_versions.push(JavaInfo {
                            path: home.to_string_lossy().to_string(),
                            version: extract_java_version(&version_str),
                        });
                    }
                }
            }
        }
    }

    java_versions.sort_by_key(|j| std::cmp::Reverse(parse_java_version(&j.version)));
    Ok(java_versions)
}

/// 扫描指定目录，寻找 Java 运行时。
pub fn scan_java_dir(dir: &Path) -> Result<Vec<JavaInfo>, String> {
    let java_cmd = if cfg!(windows) { "java.exe" } else { "java" };
    if !dir.exists() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    let mut java_versions = Vec::new();
    scan_java_dir_recursive(dir, java_cmd, 0, &mut java_versions);
    java_versions.sort_by_key(|j| std::cmp::Reverse(parse_java_version(&j.version)));
    Ok(java_versions)
}

fn scan_java_dir_recursive(dir: &Path, java_cmd: &str, depth: u32, results: &mut Vec<JavaInfo>) {
    if depth > 8 {
        return;
    }
    let home = if dir.join("Contents").join("Home").exists() {
        dir.join("Contents").join("Home")
    } else {
        dir.to_path_buf()
    };
    let java_bin = home.join("bin").join(java_cmd);
    if java_bin.exists() {
        if let Ok(output) = Command::new(&java_bin).arg("-version").output() {
            let version_str = String::from_utf8_lossy(&output.stderr).to_string();
            results.push(JavaInfo {
                path: home.to_string_lossy().to_string(),
                version: extract_java_version(&version_str),
            });
        }
        return; // 找到则不再深入（避免重复）
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.')
                    || name == "node_modules"
                    || name == "target"
                    || name == "Contents"
                {
                    continue;
                }
            }
            scan_java_dir_recursive(&path, java_cmd, depth + 1, results);
        }
    }
}

fn extract_java_version(version_output: &str) -> String {
    // 例：openjdk version "21.0.2" 2024-01-16
    for line in version_output.lines() {
        if line.contains("version") {
            if let Some(start) = line.find('"') {
                let rest = &line[start + 1..];
                if let Some(end) = rest.find('"') {
                    return rest[..end].to_string();
                }
            }
        }
    }
    "Unknown".to_string()
}

fn parse_java_version(version: &str) -> (u32, u32, u32) {
    let parts: Vec<&str> = version.split('.').collect();
    match parts.as_slice() {
        [major] => (major.parse().unwrap_or(0), 0, 0),
        [major, minor] => (major.parse().unwrap_or(0), minor.parse().unwrap_or(0), 0),
        [major, minor, patch, ..] => (
            major.parse().unwrap_or(0),
            minor.parse().unwrap_or(0),
            patch.split('-').next().unwrap_or("0").parse().unwrap_or(0),
        ),
        [] => (0, 0, 0),
    }
}

// ---------------------------------------------------------------------------
// 进程控制
// ---------------------------------------------------------------------------

pub async fn kill_game() -> Result<(), String> {
    let pid = {
        let pid_guard = GAME_PROCESS.lock().map_err(|e| e.to_string())?;
        pid_guard.ok_or("没有运行中的游戏")?
    };

    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    // 进程状态由 launch() 的 wait() 负责清理并上报 GameState::Stopped，
    // 这里不要提前清空 GAME_HANDLE，否则 wait 分支会误报「进程丢失」。
    Ok(())
}

// ---------------------------------------------------------------------------
// 版本链加载与合并
// ---------------------------------------------------------------------------

/// 库去重 key：group:artifact + classifier + 是否 natives 库。
/// 同名基础库与 natives 变体（如旧版 LWJGL）都能保留。
fn library_dedup_key(lib: &Library) -> String {
    let parts: Vec<&str> = lib.name.split(':').collect();
    let base = if parts.len() >= 2 {
        format!("{}:{}", parts[0], parts[1])
    } else {
        lib.name.clone()
    };
    let classifier = parts.get(3).copied().unwrap_or("");
    let native = if lib.natives.is_some() { "n" } else { "" };
    format!("{}:{}:{}", base, classifier, native)
}

/// 按 HMCL GameInstanceManifest.merge 的语义合并继承链上的两个版本 JSON：
/// 标量字段子版本非空才覆盖；libraries 子版本在前并按 name 去重；
/// arguments 父版本在前、子版本追加；minecraftArguments 子版本整体替换。
pub fn merge_version_json(parent: &VersionJson, child: &VersionJson) -> VersionJson {
    let mut merged = parent.clone();

    merged.id = child.id.clone();
    merged.main_class = child.main_class.clone();
    if child.asset_index.is_some() {
        merged.asset_index = child.asset_index.clone();
    }
    if child.downloads.is_some() {
        merged.downloads = child.downloads.clone();
    }
    merged.minimum_launcher_version = merged
        .minimum_launcher_version
        .max(child.minimum_launcher_version);
    if child.minecraft_arguments.is_some() {
        merged.minecraft_arguments = child.minecraft_arguments.clone();
    }
    if child.jar.is_some() {
        merged.jar = child.jar.clone();
    }
    if child.version_type.is_some() {
        merged.version_type = child.version_type.clone();
    }
    if child.assets.is_some() {
        merged.assets = child.assets.clone();
    }
    if child.java_version.is_some() {
        merged.java_version = child.java_version.clone();
    }
    merged.inherits_from = None;

    // libraries：子版本在前，按 group:artifact 去重（子版本优先），父版本未重复的追加在后。
    let mut libs: Vec<Library> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for lib in child.libraries.iter().chain(parent.libraries.iter()) {
        if seen.insert(library_dedup_key(lib)) {
            libs.push(lib.clone());
        }
    }
    merged.libraries = libs;

    // arguments：父版本在前，子版本追加在后（HMCL Arguments.merge 语义）
    match (&parent.arguments, &child.arguments) {
        (None, None) => merged.arguments = None,
        _ => {
            let mut game = parent
                .arguments
                .as_ref()
                .and_then(|a| a.game.clone())
                .unwrap_or_default();
            if let Some(child_game) = child.arguments.as_ref().and_then(|a| a.game.as_ref()) {
                game.extend(child_game.iter().cloned());
            }
            let mut jvm = parent
                .arguments
                .as_ref()
                .and_then(|a| a.jvm.clone())
                .unwrap_or_default();
            if let Some(child_jvm) = child.arguments.as_ref().and_then(|a| a.jvm.as_ref()) {
                jvm.extend(child_jvm.iter().cloned());
            }
            merged.arguments = Some(Arguments {
                game: Some(game),
                jvm: Some(jvm),
            });
        }
    }

    merged
}

/// 加载单个版本的 JSON：本地磁盘优先（加载器版本只存在于本地），官方 manifest 兜底。
/// 从网络获取的 JSON 会写回磁盘，供下次离线启动。
async fn load_version_json(
    client: &reqwest::Client,
    game_dir: &Path,
    manifest: &mut Option<Manifest>,
    id: &str,
) -> Result<VersionJson, String> {
    validate_version_id(id)?;
    let local = game_dir
        .join("versions")
        .join(id)
        .join(format!("{id}.json"));
    if local.exists() {
        if let Ok(text) = fs::read_to_string(&local).await {
            match serde_json::from_str(&text) {
                Ok(json) => return Ok(json),
                Err(e) => println!("本地版本 JSON 解析失败，回退到官方 manifest: {e}"),
            }
        }
    }

    if manifest.is_none() {
        println!("获取版本列表...");
        *manifest = Some(net::get_json(client, MANIFEST_URL).await?);
    }
    let url = manifest
        .as_ref()
        .and_then(|m| m.versions.iter().find(|v| v.id == id))
        .ok_or_else(|| format!("版本 {id} 的 JSON 不存在（本地与官方 manifest 均未找到）"))?
        .url
        .clone();
    let text = net::get_text(client, &url).await?;
    let json: VersionJson =
        serde_json::from_str(&text).map_err(|e| format!("解析版本 JSON 失败 ({id}): {e}"))?;

    fs::create_dir_all(local.parent().unwrap_or(game_dir))
        .await
        .map_err(|e| e.to_string())?;
    let mut file = File::create(&local).await.map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    Ok(json)
}

/// 沿 `inheritsFrom` 加载整条链，返回「子 → 父」顺序。
async fn load_version_chain(
    client: &reqwest::Client,
    game_dir: &Path,
    version: &str,
) -> Result<Vec<VersionJson>, String> {
    let mut manifest: Option<Manifest> = None;
    let mut chain: Vec<VersionJson> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = version.to_string();
    loop {
        if !seen.insert(current.clone()) {
            return Err(format!("版本继承链出现循环: {current}"));
        }
        let json = load_version_json(client, game_dir, &mut manifest, &current).await?;
        match json.inherits_from.clone() {
            Some(parent) => {
                println!("继承版本: {parent}");
                chain.push(json);
                current = parent;
            }
            None => {
                chain.push(json);
                break;
            }
        }
    }
    Ok(chain)
}

pub fn merge_chain(chain: Vec<VersionJson>) -> VersionJson {
    chain
        .into_iter()
        .rev()
        .reduce(|parent, child| merge_version_json(&parent, &child))
        .expect("版本链非空")
}

// ---------------------------------------------------------------------------
// maven 坐标
// ---------------------------------------------------------------------------

/// maven 坐标 `group:artifact:version[:classifier][@ext]` 展开为仓库相对路径
pub fn maven_artifact_path(name: &str) -> Option<String> {
    let (coords, ext) = match name.split_once('@') {
        Some((c, e)) => (c, e),
        None => (name, "jar"),
    };
    let parts: Vec<&str> = coords.split(':').collect();
    fn safe_component(s: &str) -> bool {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+'))
    }
    if parts.len() < 3
        || parts.len() > 4
        || !safe_component(ext)
        || parts.iter().any(|p| !safe_component(p))
        || parts[0].split('.').any(|p| p.is_empty() || p == "..")
    {
        return None;
    }
    let (group, artifact, version) = (parts[0], parts[1], parts[2]);
    let file = match parts.get(3) {
        Some(classifier) => format!("{artifact}-{version}-{classifier}.{ext}"),
        None => format!("{artifact}-{version}.{ext}"),
    };
    Some(format!(
        "{}/{}/{}/{}",
        group.replace('.', "/"),
        artifact,
        version,
        file
    ))
}

pub fn validate_version_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+'))
    {
        return Err(format!("非法版本标识: {id:?}"));
    }
    Ok(())
}

pub fn safe_library_path(path: &str) -> Result<&Path, String> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains(':')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("非法库路径: {path:?}"));
    }
    let path = Path::new(path);
    if !path
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err("非法库路径".to_string());
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// 启动计划
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LaunchPlan {
    pub java: PathBuf,
    pub jvm_args: Vec<String>,
    pub main_class: String,
    pub game_args: Vec<String>,
    pub classpath: Vec<PathBuf>,
    pub natives_dir: PathBuf,
    pub instance_dir: PathBuf,
    pub version_id: String,
    pub version_type: String,
    pub java_major_version: Option<u32>,
}

impl std::fmt::Debug for LaunchPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchPlan")
            .field("java", &self.java)
            .field("main_class", &self.main_class)
            .field("version_id", &self.version_id)
            .field("arguments", &"[已隐藏]")
            .finish_non_exhaustive()
    }
}

impl LaunchPlan {
    fn quote(arg: &str) -> String {
        if arg.is_empty() {
            "\"\"".to_string()
        } else if arg.contains(' ') || arg.contains('"') {
            format!("\"{}\"", arg.replace('"', "\\\""))
        } else {
            arg.to_string()
        }
    }

    /// 日志展示使用的安全摘要；参数可能包含令牌，不输出参数值。
    pub fn command_line(&self) -> String {
        format!(
            "{} [JVM 参数已隐藏] {} [游戏参数已隐藏]",
            Self::quote(&self.java.to_string_lossy()),
            Self::quote(&self.main_class)
        )
    }
}

fn classpath_string(paths: &[PathBuf]) -> (String, String) {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let joined = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(separator);
    (joined, separator.to_string())
}

/// 构造一次启动所需的全部参数与文件布局（会下载缺失的库/natives）。
pub async fn prepare(
    opt: &LaunchOptions,
    events: &mpsc::Sender<CoreEvent>,
) -> Result<LaunchPlan, String> {
    validate_version_id(&opt.version)?;
    let client = net::client()?;
    let features: FeatureSet = FeatureSet::new();

    // 1. 版本链（本地优先，官方 manifest 兜底）
    let chain = load_version_chain(&client, &opt.game_dir, &opt.version).await?;

    // jar 候选：显式 jar 字段优先，其次沿继承链回溯（加载器版本目录通常没有自己的 jar）
    let mut jar_candidates: Vec<String> = Vec::new();
    for json in &chain {
        if let Some(jar) = &json.jar {
            if !jar_candidates.contains(jar) {
                jar_candidates.push(jar.clone());
            }
        }
    }
    for json in &chain {
        if !jar_candidates.contains(&json.id) {
            jar_candidates.push(json.id.clone());
        }
    }

    let full = merge_chain(chain);

    // 2. 组装 classpath 并处理 natives
    let mut classpath: Vec<PathBuf> = Vec::new();
    let natives_dir = opt.game_dir.join("natives").join(&opt.version);
    let _ = fs::remove_dir_all(&natives_dir).await;
    fs::create_dir_all(&natives_dir)
        .await
        .map_err(|e| format!("创建 natives 目录失败: {e}"))?;
    let library_dir = opt.game_dir.join("libraries");

    println!("分析 Libraries...");

    // 第一遍：只解析出「要下载什么」和「classpath 顺序」，下载阶段统一并发。
    struct PendingNative {
        jar: PathBuf,
        extract: Option<ExtractRule>,
        name: String,
    }
    let mut downloads: Vec<DownloadTask> = Vec::new();
    let mut pending_natives: Vec<PendingNative> = Vec::new();

    for lib in &full.libraries {
        if !rules_allow(lib.rules.as_deref(), &features) {
            continue;
        }

        // 旧格式 natives：下载分类器 jar 并解压到 natives 目录，不进普通 classpath。
        if let Some(natives) = &lib.natives {
            let Some(classifier) = natives
                .get(get_os_name())
                .map(|c| c.replace("${arch}", natives_arch_bits()))
            else {
                continue;
            };
            let native = if let Some(artifact) = lib
                .downloads
                .as_ref()
                .and_then(|d| d.classifiers.as_ref())
                .and_then(|c| c.get(&classifier))
            {
                let path_str = artifact
                    .path
                    .clone()
                    .or_else(|| maven_artifact_path(&format!("{}:{classifier}", lib.name)))
                    .ok_or_else(|| format!("natives 缺少路径: {}", lib.name))?;
                (
                    artifact.url.clone(),
                    library_dir.join(safe_library_path(&path_str)?),
                    Some(artifact.sha1.clone()).filter(|s| !s.is_empty()),
                )
            } else if let (Some(url), Some(rel)) = (
                &lib.url,
                maven_artifact_path(&format!("{}:{classifier}", lib.name)),
            ) {
                (
                    format!("{}/{}", url.trim_end_matches('/'), rel),
                    library_dir.join(&rel),
                    lib.sha1.clone().filter(|s| !s.is_empty()),
                )
            } else {
                continue;
            };
            downloads.push(DownloadTask {
                url: native.0,
                path: native.1.clone(),
                sha1: native.2,
            });
            pending_natives.push(PendingNative {
                jar: native.1,
                extract: lib.extract.clone(),
                name: lib.name.clone(),
            });
            continue;
        }

        // 普通库：官方格式 artifact，或 Fabric/旧 Forge 的 maven 坐标格式。
        let (url, path, sha1) =
            if let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
                let path_str = artifact
                    .path
                    .clone()
                    .or_else(|| maven_artifact_path(&lib.name))
                    .ok_or_else(|| format!("库缺少路径: {}", lib.name))?;
                (
                    artifact.url.clone(),
                    library_dir.join(safe_library_path(&path_str)?),
                    Some(artifact.sha1.clone()).filter(|s| !s.is_empty()),
                )
            } else if let (Some(url), Some(rel)) = (&lib.url, maven_artifact_path(&lib.name)) {
                (
                    format!("{}/{}", url.trim_end_matches('/'), rel),
                    library_dir.join(&rel),
                    lib.sha1.clone().filter(|s| !s.is_empty()),
                )
            } else {
                // 极少数库只提供 natives，或坐标无法解析：跳过而不是让启动失败
                continue;
            };

        downloads.push(DownloadTask {
            url,
            path: path.clone(),
            sha1,
        });
        if !classpath.iter().any(|p| p == &path) {
            classpath.push(path);
        }
    }

    if !downloads.is_empty() {
        download::download_all(
            downloads,
            opt.download_concurrency.max(1),
            events.clone(),
            "正在校验/下载游戏库...",
        )
        .await?;
    }

    for native in pending_natives {
        extract_natives(&native.jar, &natives_dir, native.extract.as_ref())
            .map_err(|e| format!("解压 natives 失败 ({}): {e}", native.name))?;
    }

    // 3. 游戏 jar：加载器版本通常没有自己的 jar，沿候选链回溯到父版本
    let game_jar = jar_candidates
        .iter()
        .map(|id| {
            opt.game_dir
                .join("versions")
                .join(id)
                .join(format!("{id}.jar"))
        })
        .find(|p| p.exists())
        .ok_or_else(|| {
            format!(
                "游戏客户端不存在: versions/{}/*.jar，请先下载基础版本",
                jar_candidates.first().cloned().unwrap_or_default()
            )
        })?;
    println!("游戏 JAR: {}", game_jar.display());
    if !classpath.iter().any(|p| p == &game_jar) {
        classpath.push(game_jar.clone());
    }

    // 2.5 标题界面左下角品牌（vanilla 生效；Fabric 的 Knot 会忽略该覆盖）
    if let Some(brand) = opt.branding.as_deref().filter(|b| !b.trim().is_empty()) {
        match branding::write_overlay(&game_jar, &opt.instance_dir, brand) {
            Ok(dir) => branding::insert_branding_classpath(&mut classpath, dir),
            Err(e) => println!("启动器名称注入失败（不影响启动）: {e}"),
        }
    }

    // 4. 占位符上下文
    let (classpath_joined, classpath_separator) = classpath_string(&classpath);
    let instance_dir_str = opt.instance_dir.to_string_lossy().to_string();
    let assets_dir = opt.game_dir.join("assets");
    let assets_dir_str = assets_dir.to_string_lossy().to_string();
    let asset_index_id = full
        .asset_index
        .as_ref()
        .map(|a| a.id.clone())
        .unwrap_or_else(|| "legacy".to_string());
    let is_legacy_assets = matches!(full.assets.as_deref(), Some("pre-1.6") | Some("legacy"));
    let game_assets = if is_legacy_assets {
        assets_dir.join("virtual").join("legacy")
    } else {
        assets_dir.clone()
    };

    let (access_token, user_type) = match &opt.auth_method {
        AuthMethod::Microsoft => (
            opt.access_token
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| "0".to_string()),
            "msa".to_string(),
        ),
        AuthMethod::Offline => ("0".to_string(), "legacy".to_string()),
    };
    let version_name = if full.id.is_empty() {
        opt.version.clone()
    } else {
        full.id.clone()
    };
    let version_type = full
        .version_type
        .clone()
        .unwrap_or_else(|| "release".to_string());

    let ctx = ArgContext {
        username: opt.username.clone(),
        uuid: opt.uuid.clone(),
        access_token,
        user_type,
        version_type: version_type.clone(),
        version_name,
        game_directory: instance_dir_str,
        assets_root: assets_dir_str,
        game_assets: game_assets.to_string_lossy().to_string(),
        assets_index_name: asset_index_id,
        natives_directory: natives_dir.to_string_lossy().to_string(),
        launcher_name: LAUNCHER_NAME.to_string(),
        launcher_version: LAUNCHER_VERSION.to_string(),
        classpath: classpath_joined.clone(),
        classpath_separator,
        library_directory: library_dir.to_string_lossy().to_string(),
        user_properties: "{}".to_string(),
    };

    // 5. JVM / 游戏参数
    let mut jvm_args = expand_jvm_arguments(&full, &ctx, &features);
    let has_library_path = jvm_args
        .iter()
        .any(|a| a == "-Djava.library.path" || a.starts_with("-Djava.library.path="));
    if !has_library_path {
        jvm_args.push(format!("-Djava.library.path={}", natives_dir.display()));
    }
    if !jvm_args.iter().any(|a| a == "-cp" || a == "-classpath") {
        jvm_args.push("-cp".to_string());
        jvm_args.push(classpath_joined.clone());
    }

    let has_modern_game_args = full
        .arguments
        .as_ref()
        .and_then(|a| a.game.as_ref())
        .map(|g| !g.is_empty())
        .unwrap_or(false);

    let mut game_args = if has_modern_game_args {
        expand_game_arguments(&full, &ctx, &features)
    } else if let Some(legacy) = &full.minecraft_arguments {
        legacy
            .split_whitespace()
            .map(|part| ctx.substitute(part))
            .collect()
    } else {
        // 兜底：极简原版参数（正常版本 JSON 不会走到这里）
        vec![
            "--username".into(),
            ctx.username.clone(),
            "--version".into(),
            ctx.version_name.clone(),
            "--gameDir".into(),
            ctx.game_directory.clone(),
            "--assetsDir".into(),
            ctx.assets_root.clone(),
            "--assetIndex".into(),
            ctx.assets_index_name.clone(),
            "--uuid".into(),
            ctx.uuid.clone(),
            "--accessToken".into(),
            ctx.access_token.clone(),
            "--userType".into(),
            ctx.user_type.clone(),
            "--versionType".into(),
            ctx.version_type.clone(),
        ]
    };
    // 现代参数里没有 --userProperties 的需求；旧版可能读取它，补上无害。
    if !game_args.iter().any(|a| a == "--userProperties") {
        game_args.push("--userProperties".to_string());
        game_args.push(ctx.user_properties.clone());
    }
    append_quick_play_args(&mut game_args, opt.quick_play.as_ref());
    append_launch_switches(
        &mut game_args,
        opt.disable_multiplayer,
        opt.disable_chat,
        opt.fullscreen,
        opt.window_width,
        opt.window_height,
        &opt.graphics_backend,
    );

    Ok(LaunchPlan {
        java: opt
            .runtime_dir
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" }),
        jvm_args,
        main_class: full.main_class.clone(),
        game_args,
        classpath,
        natives_dir,
        instance_dir: opt.instance_dir.clone(),
        version_id: opt.version.clone(),
        version_type,
        java_major_version: full.java_version.and_then(|v| v.major_version),
    })
}

// ---------------------------------------------------------------------------
// 启动
// ---------------------------------------------------------------------------

pub async fn launch(
    opt: LaunchOptions,
    events: mpsc::Sender<CoreEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    if opt.managed_world.is_some() && opt.quick_play.is_some() {
        return Err("受管会话不能同时使用 Quick Play".into());
    }
    if opt.managed_world.is_some() && opt.map_config.is_none() {
        return Err("受管会话需要地图库配置".into());
    }
    if let Some(crate::bridge::ManagedTarget::JoinServer(address)) = &opt.managed_world {
        if opt.disable_multiplayer
            || address.trim().is_empty()
            || address.len() > 255
            || address.chars().any(char::is_control)
        {
            return Err("多人游戏已禁用或服务器地址无效".into());
        }
    }
    let _map_guard = match &opt.map_config {
        Some(config) => Some(crate::maps::lock_library(config)?),
        None => None,
    };
    let _ = events
        .send(CoreEvent::GameState(GameState::Launching {
            version: opt.version.clone(),
        }))
        .await;

    fs::create_dir_all(&opt.instance_dir).await?;
    if let (Some(config), Some(guard), Some(QuickPlayTarget::Singleplayer(id))) =
        (&opt.map_config, &_map_guard, &opt.quick_play)
    {
        crate::maps::backup_for_launch(config, id, guard)
            .map_err(|e| format!("启动前备份地图失败，已阻止启动: {e}"))?;
    }
    if let (Some(config), Some(guard), Some(crate::bridge::ManagedTarget::Open(id))) =
        (&opt.map_config, &_map_guard, &opt.managed_world)
    {
        crate::maps::map_dir(config, id)?;
        crate::maps::backup_for_launch(config, id, guard)
            .map_err(|e| format!("启动前备份地图失败，已阻止启动: {e}"))?;
    }
    if opt.apply_game_defaults {
        if let Err(e) = game_options::ensure_defaults(&opt.instance_dir, &GameDefaults::default()) {
            eprintln!("写入中文/引导默认值失败（不影响启动）: {e}");
        }
    }
    if opt.managed_world.is_some() {
        let config = opt.map_config.as_ref().ok_or("受管会话缺少配置")?;
        game_options::set_language(&opt.instance_dir, &config.enhanced_language)?;
    }

    let mut plan = prepare(&opt, &events).await?;

    if opt.managed_world.is_some() {
        let installed = crate::enhanced::load_installed(opt.map_config.as_ref().unwrap())?;
        if opt.version != installed.lock.version_id {
            return Err("受管会话使用的 Fabric 版本与组件锁不一致".into());
        }
        for item in crate::mods::list(&opt.instance_dir.join("mods")) {
            if item.enabled && matches!(item.id.as_str(), "fabric-api" | "deepslate-bridge") {
                return Err(format!("增强实例中存在重复的受管模组: {}", item.file_name).into());
            }
        }
        let paths =
            [installed.api_jar, installed.bridge_jar].map(|path| std::fs::canonicalize(path));
        let [api, bridge] = paths;
        let api = api?;
        let bridge = bridge?;
        let (value, separator) = classpath_string(&[api.clone(), bridge.clone()]);
        if [api, bridge]
            .iter()
            .any(|p| p.to_string_lossy().contains(&separator))
        {
            return Err("组件路径包含系统路径分隔符，无法启动 Fabric".into());
        }
        plan.jvm_args.push(format!("-Dfabric.addMods={value}"));
    }

    if !plan.java.exists() {
        let _ = events
            .send(CoreEvent::GameState(GameState::Stopped { exit_code: -1 }))
            .await;
        return Err(format!(
            "Java 不存在: {}（请在启动页选择有效的 Java 运行时）",
            plan.java.display()
        )
        .into());
    }

    // Java 版本下限检查（例如 1.21 需要 Java 21），避免启动后得到晦涩的
    // UnsupportedClassVersionError。
    if let (Some(required), Some(actual)) =
        (plan.java_major_version, detect_java_major(&opt.runtime_dir))
    {
        if actual < required {
            let _ = events
                .send(CoreEvent::GameState(GameState::Stopped { exit_code: -1 }))
                .await;
            return Err(format!(
                "该版本需要 Java {required} 或更高，当前选择的是 Java {actual}（{}）",
                opt.runtime_dir.display()
            )
            .into());
        }
    }

    if opt.managed_world.is_some() {
        let config = opt.map_config.as_ref().ok_or("受管会话缺少配置")?;
        let tuning = JvmTuning {
            gc: config.enhanced_gc.clone(),
            max_memory_mb: config.enhanced_max_memory_mb,
            compact_object_headers: config.enhanced_compact_object_headers,
        };
        apply_enhanced_jvm_tuning(
            &mut plan.jvm_args,
            &tuning,
            detect_java_major(&opt.runtime_dir),
        )?;
    }

    println!("\n启动命令:");
    println!("{}", plan.command_line());
    println!();

    let bridge = if opt.managed_world.is_some() {
        let cache = crate::runtime::paths(opt.map_config.as_ref().unwrap()).cache_dir;
        Some(crate::bridge::BridgeListener::bind_for_profile(&cache, &opt.uuid).await?)
    } else {
        None
    };
    if let Some(crate::bridge::ManagedTarget::Create {
        id,
        name,
        seed,
        mode,
        difficulty,
        preset,
        ..
    }) = &opt.managed_world
    {
        if name.trim().is_empty()
            || name.chars().count() > 64
            || !matches!(mode.as_str(), "survival" | "creative")
            || !matches!(difficulty.as_str(), "peaceful" | "easy" | "normal" | "hard")
            || !matches!(preset.as_str(), "normal" | "flat")
        {
            return Err("新世界参数无效".into());
        }
        crate::maps::begin_client_creation(
            opt.map_config.as_ref().unwrap(),
            id,
            name,
            &crate::maps::GeneratorInfo {
                kind: preset.clone(),
                seed: seed.clone(),
            },
        )?;
    }
    let mut command = tokio::process::Command::new(&plan.java);
    command
        .args(&plan.jvm_args)
        .arg(&plan.main_class)
        .args(&plan.game_args)
        .current_dir(&plan.instance_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(opt.managed_world.is_none());
    if let Some(listener) = &bridge {
        command.env("DEEPSLATE_BRIDGE_SESSION", listener.description_path());
    }
    #[cfg(unix)]
    let inherited_guard = if let Some(guard) = &_map_guard {
        use std::os::fd::AsRawFd;
        let cloned = crate::maps::duplicate_inheritable_lock(guard)?;
        let fd = cloned.as_raw_fd();
        // Only the child clears CLOEXEC, avoiding descriptor leakage into unrelated subprocesses.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Some(cloned)
    } else {
        None
    };
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 Java 失败 ({}): {e}", plan.java.display()))?;
    #[cfg(unix)]
    drop(inherited_guard);

    // 逐行转发游戏输出（stdout/stderr）
    if let Some(stdout) = child.stdout.take() {
        let events = events.clone();
        let secret = opt.access_token.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = events
                    .send(CoreEvent::GameLog {
                        is_stderr: false,
                        line: redact_game_log(&line, secret.as_deref()),
                    })
                    .await;
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let events = events.clone();
        let secret = opt.access_token.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = events
                    .send(CoreEvent::GameLog {
                        is_stderr: true,
                        line: redact_game_log(&line, secret.as_deref()),
                    })
                    .await;
            }
        });
    }

    let pid = child.id().ok_or("无法获取进程 ID")?;
    {
        let mut pid_guard = GAME_PROCESS.lock().map_err(|e| e.to_string())?;
        *pid_guard = Some(pid);
    }
    {
        let mut handle_guard = GAME_HANDLE.lock().await;
        *handle_guard = Some(child);
    }
    println!("游戏进程 PID: {pid}");

    let _ = events
        .send(CoreEvent::GameState(GameState::Running { pid }))
        .await;

    let entered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let bridge_task = match (bridge, opt.managed_world.clone()) {
        (Some(listener), Some(target)) => {
            let events = events.clone();
            let entered = entered.clone();
            let (sender, receiver) = mpsc::channel(16);
            let active_profile = uuid::Uuid::parse_str(&opt.uuid)
                .map_err(|_| "档案 UUID 无效，无法接入好友同步".to_string())?
                .hyphenated()
                .to_string();
            *BRIDGE_COMMANDS.lock().await = Some((active_profile, sender));
            Some(tokio::spawn(async move {
                let result = crate::bridge::run_world_session_with_commands(
                    listener,
                    &target,
                    &events,
                    receiver,
                    Some(entered),
                )
                .await;
                if let Err(message) = &result {
                    let _ = events
                        .send(CoreEvent::BridgeState {
                            state: "failed".into(),
                            message: Some(message.clone()),
                        })
                        .await;
                }
                result
            }))
        }
        _ => None,
    };

    // 等待进程结束 - 先取出 handle 避免跨 await 持有锁
    let exit_status = {
        let mut handle_guard = GAME_HANDLE.lock().await;
        match handle_guard.take() {
            Some(mut child) => {
                drop(handle_guard);
                child.wait().await
            }
            None => return Err("游戏进程丢失".into()),
        }?
    };
    let bridge_result = if let Some(mut task) = bridge_task {
        // Allow the last WorldClosed event to drain after the process exits.
        match tokio::time::timeout(std::time::Duration::from_secs(2), &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(format!("Bridge 会话任务失败: {e}")),
            Err(_) => {
                task.abort();
                Err("游戏退出时 Bridge 会话未完成".into())
            }
        }
    } else {
        Ok(())
    };
    *BRIDGE_COMMANDS.lock().await = None;

    {
        let mut pid_guard = GAME_PROCESS.lock().map_err(|e| e.to_string())?;
        *pid_guard = None;
    }

    let exit_code = exit_status.code().unwrap_or(-1);
    let _ = events
        .send(CoreEvent::GameState(GameState::Stopped { exit_code }))
        .await;

    if let Some(crate::bridge::ManagedTarget::Create { id, .. }) = &opt.managed_world {
        if exit_status.success()
            && bridge_result.is_ok()
            && entered.load(std::sync::atomic::Ordering::SeqCst)
        {
            crate::maps::finish_client_creation(opt.map_config.as_ref().unwrap(), id)
                .map_err(|e| format!("世界已创建但尚待恢复: {e}"))?;
        } else {
            return Err("世界创建未确认完成；创建记录和可能产生的数据已保留".into());
        }
    }

    if !exit_status.success() {
        return Err(format!("游戏进程退出码: {exit_code}").into());
    }
    bridge_result?;

    Ok(())
}

fn redact_game_log(line: &str, secret: Option<&str>) -> String {
    match secret.filter(|s| !s.is_empty() && *s != "0") {
        Some(secret) => line.replace(secret, "[已隐藏]"),
        None => line.to_string(),
    }
}

/// 从 JDK 的 `release` 文件读取 major 版本（1.8 → 8）。
fn detect_java_major(java_home: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(java_home.join("release")).ok()?;
    parse_java_major_release(&text)
}

fn parse_java_major_release(text: &str) -> Option<u32> {
    let raw = text
        .lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))?
        .trim()
        .trim_matches('"');
    let mut parts = raw.split(['.', '_', '-']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_does_not_expose_access_token() {
        let plan = LaunchPlan {
            java: "java".into(),
            jvm_args: vec![],
            main_class: "Main".into(),
            game_args: vec!["--accessToken".into(), "secret-token".into()],
            classpath: vec![],
            natives_dir: "n".into(),
            instance_dir: "i".into(),
            version_id: "v".into(),
            version_type: "release".into(),
            java_major_version: None,
        };
        assert!(!plan.command_line().contains("secret-token"));
    }

    #[test]
    fn java_launch_arguments_are_redacted_without_token_in_debug() {
        let plan = LaunchPlan {
            java: "java".into(),
            jvm_args: vec!["-Dsecret=token123".into()],
            main_class: "Main".into(),
            game_args: vec!["--accessToken=token123".into()],
            classpath: vec![],
            natives_dir: "n".into(),
            instance_dir: "i".into(),
            version_id: "v".into(),
            version_type: "release".into(),
            java_major_version: None,
        };
        assert!(!format!("{plan:?}").contains("token123"));
    }

    #[test]
    fn game_output_redacts_access_token() {
        assert_eq!(
            redact_game_log("access=secret, secret", Some("secret")),
            "access=[已隐藏], [已隐藏]"
        );
        assert_eq!(redact_game_log("abc", None), "abc");
    }

    #[test]
    fn reject_untrusted_version_and_maven_path_components() {
        assert!(validate_version_id("../outside").is_err());
        assert!(validate_version_id("/tmp/outside").is_err());
        assert!(validate_version_id("fabric-loader-0.16.14-1.21.1").is_ok());
        assert!(maven_artifact_path("group:../../outside:1").is_none());
        assert!(maven_artifact_path("group:artifact:1@../../x").is_none());
        assert!(safe_library_path("../outside.jar").is_err());
        assert!(safe_library_path("a/../../outside.jar").is_err());
        assert!(safe_library_path("group/artifact/1/a.jar").is_ok());
    }

    #[test]
    fn natives_reject_path_traversal() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("native-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let file = std::fs::File::create(root.join("native.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../outside.so", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"marker").unwrap();
        zip.finish().unwrap();
        assert!(extract_natives(&root.join("native.jar"), &root.join("natives"), None).is_err());
        assert!(!root.join("outside.so").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn natives_reject_symlink_in_destination() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("native-link-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("natives")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("natives/link")).unwrap();
        let file = std::fs::File::create(root.join("native.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("link/test.so", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"marker").unwrap();
        zip.finish().unwrap();
        assert!(extract_natives(&root.join("native.jar"), &root.join("natives"), None).is_err());
        assert!(!root.join("outside/test.so").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn natives_reject_symlink_destination_root() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("native-root-link-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("outside")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("natives")).unwrap();
        let file = std::fs::File::create(root.join("native.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("test.so", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"marker").unwrap();
        zip.finish().unwrap();
        assert!(extract_natives(&root.join("native.jar"), &root.join("natives"), None).is_err());
        assert!(!root.join("outside/test.so").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn natives_do_not_overwrite_hardlinked_files_outside_destination() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("native-hardlink-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("natives")).unwrap();
        std::fs::write(root.join("outside.so"), b"original").unwrap();
        std::fs::hard_link(root.join("outside.so"), root.join("natives/test.so")).unwrap();
        let file = std::fs::File::create(root.join("native.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("test.so", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"marker").unwrap();
        zip.finish().unwrap();
        let _ = extract_natives(&root.join("native.jar"), &root.join("natives"), None);
        assert_eq!(std::fs::read(root.join("outside.so")).unwrap(), b"original");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn natives_can_replace_a_previous_regular_library() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("native-replace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("natives")).unwrap();
        std::fs::write(root.join("natives/test.so"), b"previous").unwrap();
        let file = std::fs::File::create(root.join("native.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("test.so", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"new").unwrap();
        zip.finish().unwrap();
        extract_natives(&root.join("native.jar"), &root.join("natives"), None).unwrap();
        assert_eq!(std::fs::read(root.join("natives/test.so")).unwrap(), b"new");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn natives_reject_excessive_uncompressed_size() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("native-size-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let file = std::fs::File::create(root.join("native.jar")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(
            "large.so",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..129 {
            zip.write_all(&chunk).unwrap();
        }
        zip.finish().unwrap();
        assert!(extract_natives(&root.join("native.jar"), &root.join("natives"), None).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn lib(name: &str) -> Library {
        Library {
            name: name.to_string(),
            ..Default::default()
        }
    }

    fn vjson(id: &str, libs: Vec<Library>, arguments: Option<Arguments>) -> VersionJson {
        VersionJson {
            id: id.to_string(),
            main_class: format!("Main_{id}"),
            libraries: libs,
            arguments,
            ..Default::default()
        }
    }

    fn str_args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ctx() -> ArgContext {
        ArgContext {
            username: "Steve".into(),
            uuid: "uuid-1".into(),
            access_token: "token-1".into(),
            user_type: "legacy".into(),
            version_type: "release".into(),
            version_name: "1.21.1".into(),
            game_directory: "/game".into(),
            assets_root: "/game/assets".into(),
            game_assets: "/game/assets".into(),
            assets_index_name: "17".into(),
            natives_directory: "/game/natives/v".into(),
            launcher_name: "Deepslate Launcher".into(),
            launcher_version: "0.1.0".into(),
            classpath: "/libs/a.jar:/libs/b.jar".into(),
            classpath_separator: ":".into(),
            library_directory: "/game/libraries".into(),
            user_properties: "{}".into(),
        }
    }

    #[test]
    fn merge_unions_libraries_child_priority() {
        let parent = vjson("1.21.1", vec![lib("a:b:1"), lib("c:d:1")], None);
        let child = vjson("fabric-loader", vec![lib("c:d:2"), lib("e:f:1")], None);
        let merged = merge_version_json(&parent, &child);

        let names: Vec<&str> = merged.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["c:d:2", "e:f:1", "a:b:1"]);
        assert_eq!(merged.main_class, "Main_fabric-loader");
        assert!(merged.inherits_from.is_none());
    }

    #[test]
    fn merge_concatenates_arguments_parent_first() {
        let parent_args = Some(Arguments {
            game: Some(vec![GameArgument::String("--parent-game".into())]),
            jvm: Some(vec![JvmArgument::String("-Dparent".into())]),
        });
        let child_args = Some(Arguments {
            game: Some(vec![GameArgument::String("--child-game".into())]),
            jvm: Some(vec![JvmArgument::String("-Dchild".into())]),
        });
        let parent = vjson("1.21.1", vec![], parent_args);
        let child = vjson("fabric-loader", vec![], child_args);
        let merged = merge_version_json(&parent, &child);

        let args = merged.arguments.expect("arguments 合并后应存在");
        let game: Vec<String> = args
            .game
            .expect("game 应存在")
            .iter()
            .filter_map(|a| match a {
                GameArgument::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(game, str_args(&["--parent-game", "--child-game"]));

        let jvm: Vec<String> = args
            .jvm
            .expect("jvm 应存在")
            .iter()
            .filter_map(|a| match a {
                JvmArgument::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(jvm, str_args(&["-Dparent", "-Dchild"]));
    }

    #[test]
    fn merge_keeps_parent_fields_when_child_absent() {
        let mut parent = vjson("1.21.1", vec![lib("a:b:1")], None);
        parent.asset_index = Some(AssetIndex {
            id: "17".to_string(),
            url: "https://example.com/17.json".to_string(),
        });
        parent.downloads = Some(Downloads {
            client: Some(ClientDownload {
                url: "https://example.com/client.jar".to_string(),
                sha1: "abc".to_string(),
                size: 1,
            }),
            ..Default::default()
        });
        parent.minecraft_arguments = Some("${auth_player_name}".to_string());
        parent.jar = Some("1.21.1".to_string());
        parent.version_type = Some("release".to_string());
        parent.assets = Some("17".to_string());
        let child = vjson("fabric-loader", vec![lib("c:d:1")], None);

        let merged = merge_version_json(&parent, &child);
        assert_eq!(merged.asset_index.as_ref().unwrap().id, "17");
        assert!(merged.downloads.is_some());
        assert_eq!(
            merged.minecraft_arguments.as_deref(),
            Some("${auth_player_name}")
        );
        assert_eq!(merged.jar.as_deref(), Some("1.21.1"));
        assert_eq!(merged.version_type.as_deref(), Some("release"));
        assert_eq!(merged.assets.as_deref(), Some("17"));
    }

    #[test]
    fn merge_replaces_minecraft_arguments_when_child_has_own() {
        let mut parent = vjson("1.12.2", vec![], None);
        parent.minecraft_arguments = Some("${auth_player_name} --parent".to_string());
        let mut child = vjson("forge-1.12.2", vec![], None);
        child.minecraft_arguments = Some("${auth_player_name} --child --tweakClass".to_string());

        let merged = merge_version_json(&parent, &child);
        assert_eq!(
            merged.minecraft_arguments.as_deref(),
            Some("${auth_player_name} --child --tweakClass")
        );
    }

    #[test]
    fn merge_keeps_base_and_native_variants_of_same_artifact() {
        let base = lib("org.lwjgl.lwjgl:lwjgl:2.9.4-nightly-20150209");
        let mut native = lib("org.lwjgl.lwjgl:lwjgl:2.9.4-nightly-20150209");
        native.natives = Some(HashMap::from([(
            "linux".to_string(),
            "natives-linux".to_string(),
        )]));
        let parent = vjson("1.12.2", vec![base, native], None);
        let child = vjson("forge-1.12.2", vec![], None);

        let merged = merge_version_json(&parent, &child);
        assert_eq!(merged.libraries.len(), 2);
        assert!(merged.libraries.iter().any(|l| l.natives.is_some()));
        assert!(merged.libraries.iter().any(|l| l.natives.is_none()));
    }

    #[test]
    fn maven_path_expansion() {
        assert_eq!(
            maven_artifact_path("org.ow2.asm:asm:9.8").as_deref(),
            Some("org/ow2/asm/asm/9.8/asm-9.8.jar")
        );
        assert_eq!(
            maven_artifact_path("net.fabricmc:intermediary:1.21.1").as_deref(),
            Some("net/fabricmc/intermediary/1.21.1/intermediary-1.21.1.jar")
        );
        assert_eq!(
            maven_artifact_path("net.minecraft:client:1.21.1:slim").as_deref(),
            Some("net/minecraft/client/1.21.1/client-1.21.1-slim.jar")
        );
        assert_eq!(
            maven_artifact_path("de.oceanlabs.mcp:mcp_config:1.21.1@zip").as_deref(),
            Some("de/oceanlabs/mcp/mcp_config/1.21.1/mcp_config-1.21.1.zip")
        );
        assert_eq!(
            maven_artifact_path("g:a:1:natives-linux@jar").as_deref(),
            Some("g/a/1/a-1-natives-linux.jar")
        );
        assert!(maven_artifact_path("not-a-gav").is_none());
        assert!(maven_artifact_path("g:a").is_none());
        assert!(maven_artifact_path("g::1").is_none());
    }

    #[test]
    fn merge_chain_via_reduce_matches_layered_merge() {
        let grandparent = vjson("1.21.1", vec![lib("a:b:1"), lib("c:d:1")], None);
        let mut middle = vjson("intermediary", vec![lib("c:d:2")], None);
        middle.inherits_from = Some("1.21.1".to_string());
        let mut child = vjson("fabric-loader", vec![lib("e:f:1")], None);
        child.inherits_from = Some("intermediary".to_string());

        let chain = vec![child, middle, grandparent];
        let merged = merge_chain(chain);
        let names: Vec<&str> = merged.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["e:f:1", "c:d:2", "a:b:1"]);
        assert_eq!(merged.id, "fabric-loader");
    }

    // ---- 规则求值 ----

    #[test]
    fn rules_absent_means_allowed() {
        let features = FeatureSet::new();
        assert!(rules_allow(None, &features));
        assert!(rules_allow(Some(&[]), &features));
    }

    #[test]
    fn rules_last_match_wins() {
        let features = FeatureSet::new();
        // 原版 JSON 里常见的写法：[allow 全部, disallow 其它 OS]
        let allow_all_deny_other = [
            Rule {
                action: "allow".into(),
                ..Default::default()
            },
            Rule {
                action: "disallow".into(),
                os: Some(OsRestriction {
                    name: Some("windows".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ];
        if get_os_name() != "windows" {
            assert!(rules_allow(Some(&allow_all_deny_other), &features));
        } else {
            assert!(!rules_allow(Some(&allow_all_deny_other), &features));
        }

        // [allow 当前 OS, disallow 当前 OS]：后者命中 → 拒绝
        let allow_then_deny_self = [
            Rule {
                action: "allow".into(),
                os: Some(OsRestriction {
                    name: Some(get_os_name().into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Rule {
                action: "disallow".into(),
                os: Some(OsRestriction {
                    name: Some(get_os_name().into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ];
        assert!(!rules_allow(Some(&allow_then_deny_self), &features));
    }

    #[test]
    fn rules_other_os_only_is_denied() {
        let other = if get_os_name() == "windows" {
            "linux"
        } else {
            "windows"
        };
        let rules = [Rule {
            action: "allow".into(),
            os: Some(OsRestriction {
                name: Some(other.into()),
                ..Default::default()
            }),
            ..Default::default()
        }];
        assert!(!rules_allow(Some(&rules), &FeatureSet::new()));
    }

    #[test]
    fn rules_features_gate_optional_arguments() {
        let mut features = FeatureSet::new();
        let demo_rule = Rule {
            action: "allow".into(),
            features: Some(HashMap::from([("is_demo_user".to_string(), true)])),
            ..Default::default()
        };
        assert!(!rules_allow(
            Some(std::slice::from_ref(&demo_rule)),
            &features
        ));
        features.insert("is_demo_user".to_string(), true);
        assert!(rules_allow(
            Some(std::slice::from_ref(&demo_rule)),
            &features
        ));
    }

    // ---- 参数展开 ----

    #[test]
    fn expand_game_arguments_substitutes_and_respects_features() {
        let mut json = vjson("1.21.1", vec![], None);
        json.arguments = Some(Arguments {
            game: Some(vec![
                GameArgument::String("--username".into()),
                GameArgument::String("${auth_player_name}".into()),
                GameArgument::String("--accessToken".into()),
                GameArgument::String("${auth_access_token}".into()),
                GameArgument::String("--userType".into()),
                GameArgument::String("${user_type}".into()),
                GameArgument::Rule(RuleArgument {
                    rules: vec![Rule {
                        action: "allow".into(),
                        features: Some(HashMap::from([(
                            "has_custom_resolution".to_string(),
                            true,
                        )])),
                        ..Default::default()
                    }],
                    value: serde_json::json!(["--width", "${resolution_width}"]),
                }),
            ]),
            jvm: None,
        });

        let args = expand_game_arguments(&json, &ctx(), &FeatureSet::new());
        assert_eq!(
            args,
            str_args(&[
                "--username",
                "Steve",
                "--accessToken",
                "token-1",
                "--userType",
                "legacy"
            ])
        );
    }

    #[test]
    fn expand_jvm_arguments_substitutes_classpath_and_natives() {
        let mut json = vjson("1.21.1", vec![], None);
        json.arguments = Some(Arguments {
            game: None,
            jvm: Some(vec![
                JvmArgument::String("-Djava.library.path=${natives_directory}".into()),
                JvmArgument::String("-cp".into()),
                JvmArgument::String("${classpath}".into()),
                JvmArgument::Rule(RuleArgument {
                    rules: vec![Rule {
                        action: "allow".into(),
                        os: Some(OsRestriction {
                            name: Some("windows".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    value: serde_json::json!("-DonWindows=true"),
                }),
            ]),
        });

        let args = expand_jvm_arguments(&json, &ctx(), &FeatureSet::new());
        assert_eq!(args[0], "-Djava.library.path=/game/natives/v");
        assert_eq!(args[1], "-cp");
        assert_eq!(args[2], "/libs/a.jar:/libs/b.jar");
        if get_os_name() == "windows" {
            assert!(args.contains(&"-DonWindows=true".to_string()));
        } else {
            assert!(!args.contains(&"-DonWindows=true".to_string()));
        }
    }

    #[test]
    fn legacy_minecraft_arguments_with_placeholders() {
        // 只验证占位符替换，不启动进程
        let json = vjson("1.12.2", vec![], None);
        let mut json = json;
        json.minecraft_arguments =
            Some("--username ${auth_player_name} --gameDir ${game_directory}".into());
        let ctx = ctx();
        let args: Vec<String> = json
            .minecraft_arguments
            .as_ref()
            .unwrap()
            .split_whitespace()
            .map(|p| ctx.substitute(p))
            .collect();
        assert_eq!(
            args,
            str_args(&["--username", "Steve", "--gameDir", "/game"])
        );
    }

    #[test]
    fn parses_java_release_major() {
        assert_eq!(
            parse_java_major_release("JAVA_VERSION=\"1.8.0_402\"\n"),
            Some(8)
        );
        assert_eq!(
            parse_java_major_release("JAVA_VERSION=\"21.0.2\"\n"),
            Some(21)
        );
        assert_eq!(parse_java_major_release("JAVA_VERSION=\"25\"\n"), Some(25));
        assert_eq!(
            parse_java_major_release("JAVA_VERSION=\"17.0.1+12\"\n"),
            Some(17)
        );
        assert_eq!(parse_java_major_release("no version here"), None);
    }

    #[test]
    fn appends_quick_play_arguments() {
        let mut args = Vec::new();
        append_quick_play_args(
            &mut args,
            Some(&QuickPlayTarget::Singleplayer("my world".to_string())),
        );
        assert_eq!(args, str_args(&["--quickPlaySingleplayer", "my world"]));

        let mut args = Vec::new();
        append_quick_play_args(
            &mut args,
            Some(&QuickPlayTarget::Multiplayer("127.0.0.1:25565".to_string())),
        );
        assert_eq!(
            args,
            str_args(&["--quickPlayMultiplayer", "127.0.0.1:25565"])
        );

        let mut args = vec!["--username".to_string()];
        append_quick_play_args(&mut args, Some(&QuickPlayTarget::Multiplayer("   ".into())));
        assert_eq!(args, str_args(&["--username"]));
        append_quick_play_args(&mut args, None);
        assert_eq!(args, str_args(&["--username"]));
    }

    #[test]
    fn appends_launch_switches() {
        let mut args = Vec::new();
        append_launch_switches(&mut args, true, true, true, 1920, 1080, "vulkan");
        assert_eq!(
            args,
            str_args(&[
                "--disableMultiplayer",
                "--disableChat",
                "--fullscreen",
                "--width",
                "1920",
                "--height",
                "1080",
                "--graphicsBackend",
                "vulkan",
            ])
        );

        let mut args = Vec::new();
        append_launch_switches(&mut args, false, false, false, 0, 0, "default");
        assert!(args.is_empty());
    }

    /// 端到端（需网络）：真实 Fabric profile JSON 与官方 1.21.1 JSON 的反序列化 + 合并。
    /// `cargo test -p deepslate-core -- --ignored`
    #[test]
    #[ignore = "需要网络访问 meta.fabricmc.net 与 piston-meta.mojang.com"]
    fn real_fabric_profile_json_parses_and_merges() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = net::client().unwrap();
            let mut manifest: Option<Manifest> = None;
            let parent = load_version_json(
                &client,
                std::path::Path::new("/tmp/deepslate-merge-test"),
                &mut manifest,
                "1.21.1",
            )
            .await
            .unwrap();

            let child_text = net::get_text(
                &client,
                "https://meta.fabricmc.net/v2/versions/loader/1.21.1/0.16.14/profile/json",
            )
            .await
            .unwrap();
            let child: VersionJson = serde_json::from_str(&child_text).unwrap();

            assert_eq!(child.inherits_from.as_deref(), Some("1.21.1"));
            assert!(child.asset_index.is_none());
            assert!(child.downloads.is_none());
            assert!(child.libraries.iter().all(|l| l.url.is_some()));

            let merged = merge_version_json(&parent, &child);
            assert_eq!(
                merged.main_class,
                "net.fabricmc.loader.impl.launch.knot.KnotClient"
            );
            assert!(merged.asset_index.is_some());
            assert!(merged.libraries.iter().any(|l| l.url.is_some()));
            assert!(merged.libraries.iter().any(|l| l.downloads.is_some()));
            for l in merged.libraries.iter().filter(|l| l.url.is_some()) {
                assert!(
                    maven_artifact_path(&l.name).is_some(),
                    "无法展开: {}",
                    l.name
                );
            }
            let jvm = merged.arguments.as_ref().unwrap().jvm.as_ref().unwrap();
            let last = jvm.last();
            assert!(matches!(last, Some(JvmArgument::String(s)) if s.contains("FabricMcEmu")));
        });
    }
}
