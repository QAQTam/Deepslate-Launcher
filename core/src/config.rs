use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 获取默认根目录（优先使用深板岩 DeepslateLauncher，兼容旧版 ObsidianLauncher）
pub fn get_default_root_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|p| {
            let primary = p.join("Games").join("DeepslateLauncher");
            let legacy = p.join("Games").join("ObsidianLauncher");
            if !primary.exists() && legacy.exists() {
                legacy
            } else {
                primary
            }
        })
        .ok_or_else(|| "无法获取默认目录".to_string())
}

/// 从根目录计算实际游戏目录
/// version_isolation=false: root_dir/game/
/// version_isolation=true:  root_dir/  (每个版本的 versions/{id}/ 就是它的完整根目录)
pub fn resolve_game_dir(config: &AppConfig) -> PathBuf {
    if config.version_isolation {
        PathBuf::from(&config.root_dir)
    } else {
        PathBuf::from(&config.root_dir).join("game")
    }
}

/// 获取版本的实际目录（version_isolation=true 时为 root_dir/versions/{version}/）
pub fn get_version_root(config: &AppConfig, version: &str) -> PathBuf {
    if config.version_isolation {
        PathBuf::from(&config.root_dir)
            .join("versions")
            .join(version)
    } else {
        resolve_game_dir(config)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub version: u32,
    pub root_dir: String,
    pub version_isolation: bool, // true: 每个版本独立根目录; false: 统一 game/ 目录
    pub saves_isolation: bool,   // true: 每版本独立存档（实验级）；false: 统一存档
    pub last_profile: Option<String>,
    /// Last selected game version, including locally installed loaders.
    #[serde(default)]
    pub last_version: Option<String>,
    pub java_path: Option<String>,
    pub download_concurrency: usize,
    pub first_launch: bool,
    /// None denotes configurations created before onboarding existed.
    #[serde(default)]
    pub onboarding_version: Option<u8>,
    pub ui_mode: String,
    /// 启动游戏时弹出独立日志窗口（子进程渲染游戏 stdout/stderr）
    #[serde(default)]
    pub log_window: bool,
    /// 增强体验仅在完整的受管会话实现就绪后由启动入口启用。
    #[serde(default)]
    pub enhanced_experience: bool,
    /// 标题界面左下角显示启动器名称（vanilla 有效；Fabric 受 Knot 限制暂不生效）
    #[serde(default = "default_true")]
    pub brand_title: bool,
    #[serde(default = "default_brand_text")]
    pub brand_text: String,
    /// 首次进入实例时写入中文 / 跳过辅助功能引导（只补缺失键）
    #[serde(default = "default_true")]
    pub apply_game_defaults: bool,
    /// 最近快速加入的服务器地址
    #[serde(default)]
    pub recent_servers: Vec<String>,
    /// 增强体验的命名服务器收藏（与最近连接分开保存）。
    #[serde(default)]
    pub saved_servers: Vec<SavedServer>,
    /// 增强实例启动时写入的语言代码。
    #[serde(default = "default_enhanced_language")]
    pub enhanced_language: String,
    /// 增强实例 GC 策略：default 或 zgc。
    #[serde(default = "default_enhanced_gc")]
    pub enhanced_gc: String,
    /// 增强实例最大堆大小，0 表示由 JVM 自动决定。
    #[serde(default)]
    pub enhanced_max_memory_mb: u32,
    /// Java 25+ 的紧凑对象头选项。
    #[serde(default)]
    pub enhanced_compact_object_headers: bool,
    /// 去中心化好友网络的监听端口（DL2 邀请码里公布）。
    #[serde(default = "default_friend_port")]
    pub friend_port: u16,
    /// 好友网络对外公布地址（host:port）。留空时使用本机探测到的局域网地址。
    #[serde(default)]
    pub friend_advertise: String,
    /// 本地服务端端口
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    #[serde(default = "default_server_memory")]
    pub server_memory_mb: u32,
    #[serde(default = "default_true")]
    pub server_online_mode: bool,
    /// 兼容旧配置；实际授权凭据是 `<root>/eula.txt`（见 `eula` 模块）。
    #[serde(default)]
    pub server_eula_accepted: bool,
    // 高级启动开关（26.3 参数）
    #[serde(default)]
    pub disable_multiplayer: bool,
    #[serde(default)]
    pub disable_chat: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default = "default_window_width")]
    pub window_width: u32,
    #[serde(default = "default_window_height")]
    pub window_height: u32,
    #[serde(default = "default_graphics_backend")]
    pub graphics_backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedServer {
    pub name: String,
    pub address: String,
}

fn default_true() -> bool {
    true
}

fn default_brand_text() -> String {
    "Deepslate".to_string()
}

fn default_server_port() -> u16 {
    25565
}

fn default_friend_port() -> u16 {
    27731
}

fn default_server_memory() -> u32 {
    2048
}

fn default_window_width() -> u32 {
    1280
}

fn default_window_height() -> u32 {
    720
}

fn default_graphics_backend() -> String {
    "default".to_string()
}

fn default_enhanced_language() -> String {
    "zh_cn".to_string()
}

fn default_enhanced_gc() -> String {
    "default".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 2,
            root_dir: get_default_root_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string()),
            version_isolation: false,
            saves_isolation: false,
            last_profile: None,
            last_version: None,
            java_path: None,
            download_concurrency: 32,
            first_launch: true,
            onboarding_version: None,
            ui_mode: "dark".to_string(),
            log_window: false,
            enhanced_experience: false,
            brand_title: true,
            brand_text: default_brand_text(),
            apply_game_defaults: true,
            recent_servers: Vec::new(),
            saved_servers: Vec::new(),
            enhanced_language: default_enhanced_language(),
            enhanced_gc: default_enhanced_gc(),
            enhanced_max_memory_mb: 0,
            enhanced_compact_object_headers: false,
            friend_port: default_friend_port(),
            friend_advertise: String::new(),
            server_port: default_server_port(),
            server_memory_mb: default_server_memory(),
            server_online_mode: true,
            server_eula_accepted: false,
            disable_multiplayer: false,
            disable_chat: false,
            fullscreen: false,
            window_width: default_window_width(),
            window_height: default_window_height(),
            graphics_backend: default_graphics_backend(),
        }
    }
}

/// 配置文件路径（可执行文件同目录）
pub fn config_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|_| "无法获取可执行文件路径".to_string())?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法获取可执行文件目录".to_string())
}

/// 检查是否存在配置文件（config.toml 或旧版 config.json）
pub fn exists() -> bool {
    config_path()
        .map(|p| p.join("config.toml").exists() || p.join("config.json").exists())
        .unwrap_or(false)
}

/// 返回当前生效或预期的配置文件路径（优先 config.toml，降级兼容 config.json）
pub fn config_file_path() -> Result<PathBuf, String> {
    let base = config_path()?;
    let toml_path = base.join("config.toml");
    if toml_path.exists() {
        return Ok(toml_path);
    }
    let json_path = base.join("config.json");
    if json_path.exists() {
        return Ok(json_path);
    }
    Ok(toml_path)
}

pub fn load() -> Result<AppConfig, String> {
    let base = config_path()?;
    let toml_path = base.join("config.toml");
    let json_path = base.join("config.json");

    if toml_path.exists() {
        let content = fs::read_to_string(&toml_path).map_err(|e| e.to_string())?;
        toml::from_str(&content).map_err(|e| format!("解析 config.toml 失败: {e}"))
    } else if json_path.exists() {
        // 平滑迁移：读取旧版 config.json，并在内存反序列化后落盘为 config.toml
        let content = fs::read_to_string(&json_path).map_err(|e| e.to_string())?;
        let config: AppConfig =
            serde_json::from_str(&content).map_err(|e| format!("解析旧版 config.json 失败: {e}"))?;
        let _ = save(&config);
        Ok(config)
    } else {
        Ok(AppConfig::default())
    }
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    let path = config_path()?.join("config.toml");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content =
        toml::to_string_pretty(config).map_err(|e| format!("序列化 config.toml 失败: {e}"))?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// 删除用户配置（保留档案文件，仅重置 config.toml 为默认值）
pub fn reset_to_default() -> Result<(), String> {
    let base = config_path()?;
    let json_path = base.join("config.json");
    if json_path.exists() {
        let _ = fs::remove_file(&json_path);
    }
    save(&AppConfig::default())
}

/// 重置所有配置（删除 config.toml、旧版 config.json 和 profiles.json，重新进入首次配置）
pub fn reset_all() -> Result<(), String> {
    let base = config_path()?;
    let toml_path = base.join("config.toml");
    let json_path = base.join("config.json");
    let profiles_json = crate::profiles::path()?;
    if profiles_json.exists() {
        fs::remove_file(&profiles_json).map_err(|e| e.to_string())?;
    }
    if toml_path.exists() {
        fs::remove_file(&toml_path).map_err(|e| e.to_string())?;
    }
    if json_path.exists() {
        fs::remove_file(&json_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 清空目录内容（保留目录本身）
pub fn clear_dir_contents(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(()); // 目录不存在就算成功
    }
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            fs::remove_dir_all(&entry_path).map_err(|e| e.to_string())?;
        } else {
            fs::remove_file(&entry_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod security_tests {
    #[test]
    fn new_server_defaults_to_online_mode() {
        assert!(super::AppConfig::default().server_online_mode);
    }

    #[test]
    fn old_configs_without_last_version_still_load() {
        let mut value = serde_json::to_value(super::AppConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("last_version");
        let config: super::AppConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.last_version, None);
    }

    #[test]
    fn old_configs_default_enhanced_experience_to_disabled() {
        let mut value = serde_json::to_value(super::AppConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("enhanced_experience");
        let config: super::AppConfig = serde_json::from_value(value).unwrap();
        assert!(!config.enhanced_experience);
    }

    #[test]
    fn old_configs_without_friend_network_fields_still_load() {
        let mut value = serde_json::to_value(super::AppConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("friend_port");
        value.as_object_mut().unwrap().remove("friend_advertise");
        let config: super::AppConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.friend_port, super::default_friend_port());
        assert!(config.friend_advertise.is_empty());
    }

    #[test]
    fn toml_roundtrip() {
        let original = super::AppConfig::default();
        let encoded = toml::to_string_pretty(&original).unwrap();
        let decoded: super::AppConfig = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.version, original.version);
        assert_eq!(decoded.root_dir, original.root_dir);
        assert_eq!(decoded.ui_mode, original.ui_mode);
        assert_eq!(decoded.server_port, original.server_port);
    }

    #[test]
    fn json_migrates_to_toml_struct() {
        let original = super::AppConfig::default();
        let json_str = serde_json::to_string(&original).unwrap();
        let decoded_from_json: super::AppConfig = serde_json::from_str(&json_str).unwrap();
        let toml_str = toml::to_string_pretty(&decoded_from_json).unwrap();
        let decoded_from_toml: super::AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(decoded_from_toml.root_dir, original.root_dir);
    }
}
