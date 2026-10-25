use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 获取默认根目录
pub fn get_default_root_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|p| p.join("Games").join("ObsidianLauncher"))
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
        PathBuf::from(&config.root_dir).join("versions").join(version)
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
    pub java_path: Option<String>,
    pub download_concurrency: usize,
    pub first_launch: bool,
    pub ui_mode: String,
    /// 启动游戏时弹出独立日志窗口（子进程渲染游戏 stdout/stderr）
    #[serde(default)]
    pub log_window: bool,
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
            java_path: None,
            download_concurrency: 32,
            first_launch: true,
            ui_mode: "dark".to_string(),
            log_window: false,
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

pub fn load() -> Result<AppConfig, String> {
    let path = config_path()?.join("config.json");
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    } else {
        Ok(AppConfig::default())
    }
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    let path = config_path()?.join("config.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// 删除用户配置（保留档案文件，仅重置 config.json 为默认值）
pub fn reset_to_default() -> Result<(), String> {
    save(&AppConfig::default())
}

/// 重置所有配置（删除 config.json 和 profiles.json，重新进入首次配置）
pub fn reset_all() -> Result<(), String> {
    let config_json = config_path()?.join("config.json");
    if config_json.exists() {
        fs::remove_file(&config_json).map_err(|e| e.to_string())?;
    }
    let profiles_json = crate::profiles::path()?;
    if profiles_json.exists() {
        fs::remove_file(&profiles_json).map_err(|e| e.to_string())?;
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
