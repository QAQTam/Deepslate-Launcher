mod auth;
mod download;
mod launch;

pub use auth::AuthMethod;

use md5::{Md5, Digest};
use reqwest;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use uuid::Uuid;

/// 获取默认根目录
fn get_default_root_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|p| p.join("Games").join("ObsidianLauncher"))
        .ok_or_else(|| "无法获取默认目录".to_string())
}

/// 从根目录计算实际游戏目录
/// version_isolation=false: root_dir/game/
/// version_isolation=true:  root_dir/  (每个版本的 versions/{id}/ 就是它的完整根目录)
fn resolve_game_dir(config: &AppConfig) -> PathBuf {
    if config.version_isolation {
        PathBuf::from(&config.root_dir)
    } else {
        PathBuf::from(&config.root_dir).join("game")
    }
}

/// 获取版本的实际目录（version_isolation=true 时为 root_dir/versions/{version}/）
fn get_version_root(config: &AppConfig, version: &str) -> PathBuf {
    if config.version_isolation {
        PathBuf::from(&config.root_dir).join("versions").join(version)
    } else {
        resolve_game_dir(config)
    }
}

/// 建立或确认版本的 saves 符号链接（实验级隔离）
/// 将版本目录下的 saves/ 链接到 root_dir/saves/{version}/
async fn ensure_saves_symlink(config: &AppConfig, version: &str) -> Result<(), String> {
    if !config.saves_isolation {
        return Ok(());
    }

    let root = PathBuf::from(&config.root_dir);
    let saves_target = root.join("saves").join(version);
    let version_root = get_version_root(config, version);
    let saves_link = version_root.join("saves");

    // 创建存档目标目录
    fs::create_dir_all(&saves_target).map_err(|e| e.to_string())?;

    // 如果已经是正确的符号链接，无需操作
    if saves_link.symlink_metadata().is_ok() {
        if let Ok(meta) = saves_link.symlink_metadata() {
            if meta.file_type().is_symlink() {
                // 检查链接目标是否正确
                if let Ok(target) = fs::read_link(&saves_link) {
                    if target == saves_target {
                        return Ok(()); // 已正确链接
                    }
                }
            }
            // 不是正确链接，删除
            if saves_link.is_dir() {
                fs::remove_dir_all(&saves_link).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(&saves_link).map_err(|e| e.to_string())?;
            }
        }
    }

    // 创建符号链接
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&saves_target, &saves_link)
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(&saves_target, &saves_link)
        .map_err(|e| e.to_string())?;

    println!("已建立 saves 隔离链接: {} -> {:?}", saves_link.display(), saves_target.display());
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadOpts {
    pub version: String,
    pub game_dir: PathBuf,
    pub concurrency: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LaunchOpts {
    pub version: String,
    pub game_dir: PathBuf,
    pub username: String,
    pub uuid: String,
    pub runtime_dir: PathBuf,
    pub auth_method: auth::AuthMethod,
    pub access_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserProfile {
    pub name: String,
    pub uuid: String,
    #[serde(default)]
    pub auth_method: auth::AuthMethod,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl UserProfile {
    pub fn is_online(&self) -> bool {
        matches!(self.auth_method, auth::AuthMethod::Microsoft)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileStore {
    pub profiles: Vec<UserProfile>,
    pub last_used: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub version: u32,
    pub root_dir: String,
    pub version_isolation: bool,   // true: 每个版本独立根目录; false: 统一 game/ 目录
    pub saves_isolation: bool,     // true: 每版本独立存档（实验级）；false: 统一存档
    pub last_profile: Option<String>,
    pub java_path: Option<String>,
    pub download_concurrency: usize,
    pub first_launch: bool,
    pub ui_mode: String,
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
            ui_mode: "gorgeous".to_string(),
        }
    }
}

/// 获取配置文件路径（可执行文件同目录）
fn get_config_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|_| "无法获取可执行文件路径".to_string())?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法获取可执行文件目录".to_string())
}

fn get_profiles_path() -> Result<PathBuf, String> {
    // 优先使用 config.json 同目录（v2+: root_dir/config/）
    let config_path = get_config_path()?.join("config.json");
    if config_path.exists() {
        if let Ok(config) = load_config() {
            if config.version >= 2 {
                return Ok(PathBuf::from(&config.root_dir).join("config").join("profiles.json"));
            }
        }
    }
    // 降级/旧版：使用 ~/.config/obsidian-launcher/
    dirs::config_dir()
        .map(|p| p.join("obsidian-launcher").join("profiles.json"))
        .ok_or_else(|| "无法获取配置目录".to_string())
}

fn load_profiles() -> Result<ProfileStore, String> {
    let path = get_profiles_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    } else {
        Ok(ProfileStore {
            profiles: Vec::new(),
            last_used: None,
        })
    }
}

fn save_profiles(store: &ProfileStore) -> Result<(), String> {
    let path = get_profiles_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

fn load_config() -> Result<AppConfig, String> {
    let path = get_config_path()?.join("config.json");
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    } else {
        Ok(AppConfig::default())
    }
}

fn save_config(config: &AppConfig) -> Result<(), String> {
    let path = get_config_path()?.join("config.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// 基于用户名生成离线 UUID（ deterministic）
fn uuid_from_name(name: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(name.as_bytes());
    let result = hasher.finalize();
    // MD5 是 16 字节，UUID 是 32 字符 hex + 4 个连字符
    let hash_str = hex::encode(result);
    format!(
        "{}-{}-{}-{}-{}",
        &hash_str[0..8],
        &hash_str[8..12],
        &hash_str[12..16],
        &hash_str[16..20],
        &hash_str[20..32]
    )
}

/// 生成随机 UUID
fn random_uuid() -> String {
    Uuid::new_v4().to_string()
}

#[tauri::command]
async fn download(app: AppHandle, opts: DownloadOpts) -> Result<(), String> {
    download::run(app, &opts.version, &opts.game_dir, opts.concurrency)
        .await
        .map_err(|e| e.to_string())
}

/// 准备版本存档隔离（建立符号链接）
/// 在启动/下载前调用
#[tauri::command]
async fn prepare_saves_isolation(version: String) -> Result<(), String> {
    let config = load_config()?;
    ensure_saves_symlink(&config, &version).await
}

/// 获取存档备份目录（不带版本名）
fn get_saves_backups_root(config: &AppConfig, version: &str) -> PathBuf {
    PathBuf::from(&config.root_dir).join("saves").join(format!("{}_backups", version))
}

/// 清理超过数量的旧备份
fn cleanup_old_backups(backup_dir: &Path, keep: usize) -> Result<(), String> {
    if !backup_dir.exists() {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(backup_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.len() <= keep {
        return Ok(());
    }

    // 按修改时间排序（旧的在前）
    entries.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());

    for entry in entries.iter().take(entries.len() - keep) {
        fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
        println!("已删除旧备份: {:?}", entry.path());
    }
    Ok(())
}

/// 创建存档备份
#[tauri::command]
async fn create_saves_backup(version: String) -> Result<String, String> {
    let config = load_config()?;
    let saves_dir = PathBuf::from(&config.root_dir).join("saves").join(&version);

    if !saves_dir.exists() {
        return Err("存档目录不存在，无法备份".to_string());
    }

    let backup_root = get_saves_backups_root(&config, &version);
    fs::create_dir_all(&backup_root).map_err(|e| e.to_string())?;

    // 时间戳命名
    let now = chrono_lite_timestamp();
    let backup_path = backup_root.join(&now);

    // 跨平台复制
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("rsync")
            .args(["-a", "--delete", saves_dir.to_str().unwrap(), backup_path.to_str().unwrap()])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("rsync 备份失败".to_string());
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("xcopy")
            .args([saves_dir.to_str().unwrap(), backup_path.to_str().unwrap(), "/E", "/I", "/Y", "/Q"])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("xcopy 备份失败".to_string());
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        copy_dir_recursive(&saves_dir, &backup_path)?;
    }

    // 清理旧备份，保留最近 5 个
    cleanup_old_backups(&backup_root, 5)?;

    println!("存档备份已创建: {:?}", backup_path);
    Ok(now)
}

/// 列出存档备份
#[tauri::command]
fn list_saves_backups(version: String) -> Result<Vec<String>, String> {
    let config = load_config()?;
    let backup_root = get_saves_backups_root(&config, &version);

    if !backup_root.exists() {
        return Ok(Vec::new());
    }

    let mut backups: Vec<String> = fs::read_dir(&backup_root)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    backups.sort_by(|a, b| b.cmp(a)); // 新的在前
    Ok(backups)
}

/// 恢复存档备份
#[tauri::command]
async fn restore_saves_backup(version: String, backup_name: String) -> Result<(), String> {
    let config = load_config()?;
    let backup_path = get_saves_backups_root(&config, &version).join(&backup_name);
    let saves_dir = PathBuf::from(&config.root_dir).join("saves").join(&version);

    if !backup_path.exists() {
        return Err("备份不存在".to_string());
    }

    // 先清空当前存档
    if saves_dir.exists() {
        for entry in fs::read_dir(&saves_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
    } else {
        fs::create_dir_all(&saves_dir).map_err(|e| e.to_string())?;
    }

    // 复制备份内容到存档目录
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("rsync")
            .args(["-a", "--delete", backup_path.to_str().unwrap(), saves_dir.parent().unwrap().to_str().unwrap()])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("rsync 恢复失败".to_string());
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("xcopy")
            .args([backup_path.to_str().unwrap(), saves_dir.to_str().unwrap(), "/E", "/I", "/Y", "/Q"])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("xcopy 恢复失败".to_string());
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        copy_dir_recursive(&backup_path, &saves_dir)?;
    }

    // 重建符号链接（如果启用了 saves_isolation）
    ensure_saves_symlink(&config, &version).await?;

    println!("存档已恢复: {}", backup_name);
    Ok(())
}

/// 删除存档备份
#[tauri::command]
fn delete_saves_backup(version: String, backup_name: String) -> Result<(), String> {
    let config = load_config()?;
    let backup_path = get_saves_backups_root(&config, &version).join(&backup_name);
    if backup_path.exists() {
        fs::remove_dir_all(&backup_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 简单的日期时间戳（纯 std::time 实现，跨平台）
fn chrono_lite_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 手动计算日期时间（平年，非负秒）
    let mut secs = now;
    let (year, month, day, hour, minute, second) = {
        const SECS_PER_MINUTE: u64 = 60;
        const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
        const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;
        const DAYS_PER_400Y: u64 = 365 * 400 + 97;
        const DAYS_PER_100Y: u64 = 365 * 100 + 24;
        const DAYS_PER_4Y: u64 = 365 * 4 + 1;

        // 1970-01-01 是周四，索引 0 = 周四
        const DAYS_FROM_1970: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        const DAYS_FROM_1970_LEAP: [i32; 12] = [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];

        let days = (secs / SECS_PER_DAY) as i64;
        secs %= SECS_PER_DAY;
        let hour = (secs / SECS_PER_HOUR) as u32;
        secs %= SECS_PER_HOUR;
        let minute = (secs / SECS_PER_MINUTE) as u32;
        let second = (secs % SECS_PER_MINUTE) as u32;

        // 计算年份（相对 1970）
        let mut year = 1970;
        let mut days_left = days;

        // 400 年周期
        let cyc400 = days_left / DAYS_PER_400Y as i64;
        days_left -= cyc400 * DAYS_PER_400Y as i64;
        year += (cyc400 * 400) as i32;

        // 100 年周期
        let cyc100 = days_left / DAYS_PER_100Y as i64;
        if cyc100 == 4 {
            // 最后一个 100 年周期（400 年周期的最后一天）
            days_left -= 3 * DAYS_PER_100Y as i64;
            year += 300;
        } else {
            days_left -= cyc100 * DAYS_PER_100Y as i64;
            year += (cyc100 * 100) as i32;
        }

        // 4 年周期
        let cyc4 = days_left / DAYS_PER_4Y as i64;
        if cyc4 == 25 {
            // 最后一个 4 年周期（100 年周期的最后一天）
            days_left -= 24 * DAYS_PER_4Y as i64;
            year += 96;
        } else {
            days_left -= cyc4 * DAYS_PER_4Y as i64;
            year += (cyc4 * 4) as i32;
        }

        // 年周期（1-4 年）
        let mut yday = days_left as i32;
        let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        if is_leap {
            yday += 1;
        }

        // 确定月份
        let (month_idx, mday) = if is_leap {
            let idx = DAYS_FROM_1970_LEAP.iter().position(|&d| yday < d).unwrap_or(12);
            (idx, yday - DAYS_FROM_1970_LEAP[idx - 1])
        } else {
            let idx = DAYS_FROM_1970.iter().position(|&d| yday < d).unwrap_or(12);
            (idx, yday - DAYS_FROM_1970[idx - 1])
        };

        (year, month_idx as u32, (mday + 1) as u32, hour, minute, second)
    };

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        year, month, day, hour, minute, second
    )
}

/// 递归复制目录（跨平台兜底）
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if src.is_dir() {
        fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        let mut entries = fs::read_dir(src).map_err(|e| e.to_string())?;
        while let Some(entry_result) = entries.next() {
            let entry = entry_result.map_err(|e| e.to_string())?;
            let src_path = entry.path();
            let file_name: std::ffi::OsString = entry.file_name();
            let dst_path = dst.join(&file_name);
            copy_dir_recursive(&src_path, &dst_path)?;
        }
    } else {
        fs::copy(src, dst).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn launch(app: AppHandle, opts: LaunchOpts) -> Result<(), String> {
    let opt = launch::LaunchOptions {
        version: opts.version,
        game_dir: opts.game_dir,
        username: opts.username,
        uuid: opts.uuid,
        runtime_dir: opts.runtime_dir,
        auth_method: opts.auth_method.clone(),
        access_token: opts.access_token.clone(),
    };
    launch::launch(app, opt).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_java() -> Result<Vec<launch::JavaInfo>, String> {
    launch::scan_java()
}

#[tauri::command]
fn scan_java_dir(dir: String) -> Result<Vec<launch::JavaInfo>, String> {
    launch::scan_java_dir(&PathBuf::from(dir))
}

#[tauri::command]
async fn kill_game() -> Result<(), String> {
    launch::kill_game().await
}

#[tauri::command]
async fn get_versions() -> Result<Vec<download::VersionInfo>, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| e.to_string())?;

    let manifest: download::Manifest = client
        .get(download::MANIFEST_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    Ok(manifest.versions)
}

/// 获取所有档案
#[tauri::command]
fn get_profiles() -> Result<ProfileStore, String> {
    load_profiles()
}

/// 创建新档案（使用用户名生成 deterministic UUID）
#[tauri::command]
fn create_profile(name: String) -> Result<UserProfile, String> {
    if name.trim().is_empty() {
        return Err("用户名不能为空".to_string());
    }
    let uuid = uuid_from_name(&name);
    let profile = UserProfile {
        name,
        uuid,
        auth_method: AuthMethod::Offline,
        access_token: None,
        refresh_token: None,
        expires_at: None,
    };

    let mut store = load_profiles()?;
    // 检查是否已存在同名档案
    if store.profiles.iter().any(|p| p.name == profile.name) {
        return Err("档案已存在".to_string());
    }
    store.profiles.push(profile.clone());
    store.last_used = Some(profile.name.clone());
    save_profiles(&store)?;

    Ok(profile)
}

/// Microsoft 在线登录
#[tauri::command]
async fn login_microsoft(app: AppHandle) -> Result<UserProfile, String> {
    let online_profile = auth::start_microsoft_login(app).await?;

    let profile = UserProfile {
        name: online_profile.username,
        uuid: online_profile.uuid,
        auth_method: auth::AuthMethod::Microsoft,
        access_token: Some(online_profile.access_token),
        refresh_token: Some(online_profile.refresh_token),
        expires_at: Some(online_profile.expires_at),
    };

    // 保存到 profiles
    let mut store = load_profiles()?;
    // 检查是否已存在（通过 uuid 判断是否是同一个 Microsoft 账户）
    if let Some(existing) = store.profiles.iter_mut().find(|p| p.uuid == profile.uuid) {
        // 更新现有档案
        existing.name = profile.name.clone();
        existing.auth_method = profile.auth_method.clone();
        existing.access_token = profile.access_token.clone();
        existing.refresh_token = profile.refresh_token.clone();
        existing.expires_at = profile.expires_at;
    } else {
        store.profiles.push(profile.clone());
    }
    store.last_used = Some(profile.name.clone());
    save_profiles(&store)?;

    println!("Microsoft 登录成功: {} ({})", profile.name, profile.uuid);
    Ok(profile)
}

/// 使用授权码完成 Microsoft 登录（不打开浏览器）
#[tauri::command]
async fn login_microsoft_with_code(app: AppHandle, code: String) -> Result<UserProfile, String> {
    let online_profile = auth::login_microsoft_with_code(app, code).await?;

    let profile = UserProfile {
        name: online_profile.username,
        uuid: online_profile.uuid,
        auth_method: auth::AuthMethod::Microsoft,
        access_token: Some(online_profile.access_token),
        refresh_token: Some(online_profile.refresh_token),
        expires_at: Some(online_profile.expires_at),
    };

    // 保存到 profiles
    let mut store = load_profiles()?;
    if let Some(existing) = store.profiles.iter_mut().find(|p| p.uuid == profile.uuid) {
        existing.name = profile.name.clone();
        existing.auth_method = profile.auth_method.clone();
        existing.access_token = profile.access_token.clone();
        existing.refresh_token = profile.refresh_token.clone();
        existing.expires_at = profile.expires_at;
    } else {
        store.profiles.push(profile.clone());
    }
    store.last_used = Some(profile.name.clone());
    save_profiles(&store)?;

    println!("Microsoft 登录成功: {} ({})", profile.name, profile.uuid);
    Ok(profile)
}

/// 获取 Microsoft 授权 URL
#[tauri::command]
fn get_microsoft_auth_url() -> String {
    auth::get_auth_url()
}

/// 使用系统默认浏览器打开 Microsoft 授权页面
#[tauri::command]
fn open_microsoft_auth() -> Result<(), String> {
    let auth_url = auth::get_auth_url();
    open::that(&auth_url).map_err(|e| format!("打开浏览器失败: {}", e))
}

/// 删除档案
#[tauri::command]
fn delete_profile(name: String) -> Result<(), String> {
    let mut store = load_profiles()?;
    store.profiles.retain(|p| p.name != name);
    if store.last_used.as_ref() == Some(&name) {
        store.last_used = None;
    }
    save_profiles(&store)
}

/// 重置 UUID（为当前档案生成新的随机 UUID）
#[tauri::command]
fn reset_profile_uuid(name: String) -> Result<UserProfile, String> {
    let mut store = load_profiles()?;
    let profile = store.profiles.iter_mut()
        .find(|p| p.name == name)
        .ok_or("档案不存在")?;

    profile.uuid = random_uuid();
    let result = profile.clone();
    save_profiles(&store)?;

    Ok(result)
}

/// 生成新的随机 UUID
#[tauri::command]
fn generate_uuid() -> String {
    random_uuid()
}

/// 基于用户名生成 UUID
#[tauri::command]
fn generate_uuid_from_name(name: String) -> String {
    uuid_from_name(&name)
}

/// 获取默认根目录
#[tauri::command]
fn get_root_dir() -> Result<String, String> {
    get_default_root_dir().map(|p| p.to_string_lossy().to_string())
}

/// 获取实际游戏目录（根据 config 解析）
#[tauri::command]
fn get_game_dir() -> Result<String, String> {
    let config = load_config()?;
    Ok(resolve_game_dir(&config).to_string_lossy().to_string())
}

/// 获取实际存档目录（始终为 root_dir/saves/）
#[tauri::command]
fn get_saves_dir() -> Result<String, String> {
    let config = load_config()?;
    Ok(PathBuf::from(&config.root_dir).join("saves").to_string_lossy().to_string())
}

/// 获取实际版本目录
/// isolation=false: root_dir/game/versions/
/// isolation=true:  root_dir/versions/
#[tauri::command]
fn get_versions_dir() -> Result<String, String> {
    let config = load_config()?;
    if config.version_isolation {
        Ok(PathBuf::from(&config.root_dir).join("versions").to_string_lossy().to_string())
    } else {
        Ok(PathBuf::from(&config.root_dir).join("game").join("versions").to_string_lossy().to_string())
    }
}

/// 获取应用配置
#[tauri::command]
fn get_config() -> Result<AppConfig, String> {
    let mut config = load_config()?;
    // first_launch 必须为 false 才视为已完成首次配置，否则强制重新配置
    if config.first_launch != false {
        config.first_launch = true;
    }
    Ok(config)
}

/// 保存应用配置（完成首次配置后 first_launch 设为 false）
#[tauri::command]
fn set_config(config: AppConfig) -> Result<(), String> {
    save_config(&config)
}

/// 删除用户配置（保留档案文件，仅重置 config.json）
#[tauri::command]
fn delete_user_config() -> Result<(), String> {
    // 保留 root_dir，只删除 config.json（重置为默认值）
    let config = AppConfig::default();
    save_config(&config)
}

/// 重置所有配置（删除 config.json 和 profiles.json，重新进入首次配置）
#[tauri::command]
fn reset_all_config() -> Result<(), String> {
    // 删除 config.json
    let config_path = get_config_path()?.join("config.json");
    if config_path.exists() {
        fs::remove_file(&config_path).map_err(|e| e.to_string())?;
    }
    // 删除 profiles.json（在 root_dir/config/ 或旧版 ~/.config/）
    let profiles_path = get_profiles_path()?;
    if profiles_path.exists() {
        fs::remove_file(&profiles_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 清空游戏目录（保留目录本身，删除所有内容）
#[tauri::command]
fn clear_game_files(game_dir: String) -> Result<(), String> {
    let path = PathBuf::from(&game_dir);
    if !path.exists() {
        return Ok(()); // 目录不存在就算成功
    }
    // 遍历删除所有内容
    for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
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

// ── Minecraft News API ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MinecraftNewsResponse {
    items: Vec<MinecraftNewsItem>,
    total: u64,
    page: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MinecraftNewsItem {
    title: String,
    description: String,
    url: String,
    date: String,
    image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MinecraftArticleDetail {
    title: String,
    description: String,
    author: String,
    date: String,
    image_url: Option<String>,
    url: String,
    images: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MinecraftSearchResponse {
    result: MinecraftSearchResult,
}

#[derive(Debug, Deserialize)]
struct MinecraftSearchResult {
    results: Vec<MinecraftArticleItem>,
    #[serde(default)]
    #[serde(rename = "numFound")]
    num_found: u64,
    #[serde(default)]
    page: u64,
}

#[derive(Debug, Deserialize)]
struct MinecraftArticleItem {
    title: String,
    #[serde(default)]
    description: String,
    url: String,
    time: i64,
    #[serde(default)]
    #[serde(rename = "image")]
    image_url: Option<String>,
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&#39;", "'")
     .replace("&#x27;", "'")
     .replace("&apos;", "'")
     .replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#x2F;", "/")
}

/// Fetches Minecraft news from the official minecraft.net search API.
#[tauri::command]
async fn fetch_minecraft_news(page: Option<u64>) -> Result<MinecraftNewsResponse, String> {
    let page = page.unwrap_or(1);
    let client = reqwest::Client::builder()
        .user_agent("Obsidian-Launcher/1.0")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let api_url = format!(
        "https://net-secondary.web.minecraft-services.net/api/v1.0/zh-cn/search?pageSize=24&sortType=Recent&category=News&newsOnly=true&geography=CN&page={}",
        page
    );
    let resp = client.get(&api_url).send().await
        .map_err(|e| format!("Failed to fetch news: {}", e))?;
    let json = resp.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let data: MinecraftSearchResponse = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse news: {}", e))?;

    let items: Vec<MinecraftNewsItem> = data.result.results.into_iter().filter_map(|a| {
        let title = decode_html_entities(a.title.trim());
        if title.is_empty() { return None; }
        let date = if a.time > 0 { timestamp_to_date(a.time) } else { String::new() };
        let desc = if a.description.trim().is_empty() {
            "阅读更多 →".to_string()
        } else {
            decode_html_entities(&a.description)
        };
        Some(MinecraftNewsItem { title, description: desc, url: a.url, date, image_url: a.image_url })
    }).collect();

    Ok(MinecraftNewsResponse {
        items,
        total: data.result.num_found,
        page: data.result.page,
    })
}

/// Fetches article detail (JSON-LD metadata) from a minecraft.net article page.
#[tauri::command]
async fn fetch_article_detail(url: String) -> Result<MinecraftArticleDetail, String> {
    let client = reqwest::Client::builder()
        .user_agent("Obsidian-Launcher/1.0")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let resp = client.get(&url).send().await
        .map_err(|e| format!("Failed to fetch article: {}", e))?;
    let html = resp.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Parse JSON-LD with serde for reliable metadata extraction
    let mut author = String::new();
    let mut title_ld = String::new();
    let mut date_ld = String::new();
    let mut desc_og = String::new();
    let mut image_og: Option<String> = None;

    if let Some(ld_start) = html.find(r#"type="application/ld+json""#) {
        let after_tag = &html[ld_start..];
        if let Some(script_end) = after_tag.find("</script>") {
            if let Some(json_start) = after_tag[..script_end].rfind('>') {
                let json_str = &after_tag[json_start+1..script_end];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(a) = val.get("author") {
                        author = a.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    }
                    if let Some(h) = val.get("headline").and_then(|v| v.as_str()) {
                        title_ld = h.to_string();
                    }
                    if let Some(d) = val.get("datePublished").and_then(|v| v.as_str()) {
                        date_ld = if d.len() > 10 { d[..10].to_string() } else { d.to_string() };
                    }
                }
            }
        }
    }

    // OG description
    if let Some(start) = html.find(r#"property="og:description" content=""#) {
        let slice = &html[start + 36..];
        if let Some(end) = slice.find('"') {
            desc_og = decode_html_entities(&slice[..end]);
        }
    }

    // OG image
    if let Some(start) = html.find(r#"property="og:image" content=""#) {
        let slice = &html[start + 31..];
        if let Some(end) = slice.find('"') {
            image_og = Some(decode_html_entities(&slice[..end]));
        }
    }

    // Extract all article body images (filter icons/avatars/logos)
    let mut images: Vec<String> = Vec::new();
    if let Some(art_start) = html.find(r#"class="article-text""#) {
        let footer = html[art_start..].find("<footer ").unwrap_or(html.len() - art_start);
        let article_area = &html[art_start..art_start + footer];
        let mut pos = 0;
        while let Some(img) = article_area[pos..].find("<img ") {
            let abs = pos + img;
            if let Some(end) = article_area[abs..].find('>') {
                let tag = &article_area[abs..abs + end];
                if let Some(s) = tag.find("src=\"") {
                    let ss = &tag[s + 5..];
                    if let Some(se) = ss.find('"') {
                        let src = &ss[..se];
                        let lower = src.to_lowercase();
                        // Filter out icons, avatars, logos, and small assets
                        if !lower.contains("icon") && !lower.contains("avatar")
                           && !lower.contains("logo") && !lower.contains("svg")
                           && !lower.contains("cursors") {
                            let full = if src.starts_with("http") { src.to_string() }
                                else if src.starts_with("//") { format!("https:{}", src) }
                                else { format!("https://www.minecraft.net{}", src) };
                            if !images.contains(&full) {
                                images.push(full);
                            }
                        }
                    }
                }
                pos = abs + end + 1;
            } else { break; }
        }
    }

    // Extract article body from SSR <article-text> with boundary check
    let mut body = String::new();
    if let Some(art_start) = html.find(r#"class="article-text""#) {
        let slice = &html[art_start..];
        // Find boundary: stop before footer or related-articles section
        let end_pos = [
            "<footer ",
            r#"class="article-sidebar""#,
            r#"class="related-articles""#,
        ].iter().filter_map(|m| slice.find(m)).min().unwrap_or(slice.len());
        let article_slice = &slice[..end_pos];

        // Extract <p>, <h2>, <h3>, <li> in document order
        let tags: &[(&str, &str, &str)] = &[
            ("<h2>", "</h2>", "## "),
            ("<h3>", "</h3>", "### "),
            ("<li>", "</li>", "- "),
            ("<p>",  "</p>",  ""),
        ];
        let mut pos = 0;
        while pos < article_slice.len() {
            // Find the earliest tag at or after pos
            let mut earliest: Option<(usize, &str, &str, &str)> = None;
            for (open, close, prefix) in tags {
                if let Some(start) = article_slice[pos..].find(open) {
                    let score = pos + start;
                    if earliest.map_or(true, |e| score < e.0) {
                        earliest = Some((score, open, close, prefix));
                    }
                }
            }
            if let Some((start, open, close, prefix)) = earliest {
                let after_open = start + open.len();
                if let Some(close_at) = article_slice[after_open..].find(close) {
                    let content_end = after_open + close_at;
                    let text = strip_html(&article_slice[after_open..content_end]);
                    if !text.is_empty() {
                        if !body.is_empty() { body.push('\n'); }
                        body.push_str(prefix);
                        body.push_str(&text);
                    }
                    pos = content_end + close.len();
                } else { break; }
            } else { break; }
        }
    }
    // Use extracted body, fallback to OG description
    let description = if !body.is_empty() {
        decode_html_entities(&body)
    } else {
        desc_og
    };

    Ok(MinecraftArticleDetail {
        title: decode_html_entities(&title_ld),
        description,
        author: decode_html_entities(&author),
        date: date_ld,
        image_url: image_og,
        url,
        images,
    })
}

fn strip_html(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in input.chars() {
        if c == '<' { in_tag = true; continue; }
        if c == '>' { in_tag = false; continue; }
        if !in_tag { out.push(c); }
    }
    decode_html_entities(&out.trim().to_string())
}


#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    open::that(&url).map_err(|e| format!("Failed to open URL: {}", e))
}

fn timestamp_to_date(ts: i64) -> String {
    // Howard Hinnant's civil_from_days — correct for all Gregorian dates
    let z = ts / 86400 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            download,
            launch,
            scan_java,
            scan_java_dir,
            kill_game,
            get_versions,
            get_profiles,
            create_profile,
            delete_profile,
            reset_profile_uuid,
            generate_uuid,
            generate_uuid_from_name,
            get_root_dir,
            get_game_dir,
            get_saves_dir,
            get_versions_dir,
            get_config,
            set_config,
            delete_user_config,
            reset_all_config,
            clear_game_files,
            prepare_saves_isolation,
            create_saves_backup,
            list_saves_backups,
            restore_saves_backup,
            delete_saves_backup,
            login_microsoft,
            login_microsoft_with_code,
            get_microsoft_auth_url,
            open_microsoft_auth,
            fetch_minecraft_news,
            fetch_article_detail,
            open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
