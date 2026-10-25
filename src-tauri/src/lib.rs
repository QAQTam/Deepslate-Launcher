//! Tauri 壳：只负责 IPC。
//! 全部业务逻辑在 `obsidian-launcher-core` crate 中，本文件把命令调用桥接到 core，
//! 并把 core 的事件 channel 转发为前端 Tauri 事件（事件名与载荷和旧版完全一致）。

use obsidian_launcher_core as launcher;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use launcher::events::CoreEvent;
use launcher::{AppConfig, AuthMethod, ProfileStore, UserProfile};

// ── IPC DTO ─────────────────────────────────────────

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
    pub auth_method: AuthMethod,
    pub access_token: Option<String>,
}

// ── 事件桥接 ─────────────────────────────────────────

/// 把 core 的事件 channel 桥接为 Tauri 前端事件。
/// 返回的 Sender 生命周期与命令一致：命令结束时 drop，桥接任务随之结束。
fn spawn_event_bridge(app: &AppHandle) -> mpsc::Sender<CoreEvent> {
    let (tx, mut rx) = mpsc::channel(64);
    let app = app.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CoreEvent::DownloadProgress(p) => {
                    let _ = app.emit("download-progress", p);
                }
                CoreEvent::AuthProgress(p) => {
                    let _ = app.emit("auth-progress", p);
                }
                CoreEvent::GameState(s) => {
                    let _ = app.emit("game-state", s);
                }
            }
        }
    });
    tx
}

// ── 游戏生命周期 ─────────────────────────────────────

#[tauri::command]
async fn download(app: AppHandle, opts: DownloadOpts) -> Result<(), String> {
    let events = spawn_event_bridge(&app);
    launcher::download::run(&opts.version, &opts.game_dir, opts.concurrency, events)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn launch(app: AppHandle, opts: LaunchOpts) -> Result<(), String> {
    let events = spawn_event_bridge(&app);
    let opt = launcher::launch::LaunchOptions {
        version: opts.version,
        game_dir: opts.game_dir,
        username: opts.username,
        uuid: opts.uuid,
        runtime_dir: opts.runtime_dir,
        auth_method: opts.auth_method,
        access_token: opts.access_token,
    };
    launcher::launch::launch(opt, events)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn scan_java() -> Result<Vec<launcher::launch::JavaInfo>, String> {
    launcher::launch::scan_java()
}

#[tauri::command]
fn scan_java_dir(dir: String) -> Result<Vec<launcher::launch::JavaInfo>, String> {
    launcher::launch::scan_java_dir(&PathBuf::from(dir))
}

#[tauri::command]
async fn kill_game() -> Result<(), String> {
    launcher::launch::kill_game().await
}

#[tauri::command]
async fn get_versions() -> Result<Vec<launcher::download::VersionInfo>, String> {
    launcher::download::get_versions().await
}

// ── 档案管理 ─────────────────────────────────────────

#[tauri::command]
fn get_profiles() -> Result<ProfileStore, String> {
    launcher::profiles::load()
}

#[tauri::command]
fn create_profile(name: String) -> Result<UserProfile, String> {
    launcher::profiles::create(name)
}

/// Tauri webview 无法监听授权回调 URL，自动登录不可用；
/// 前端捕获此错误后引导用户走"系统浏览器 + 手动粘贴 code"路径。
#[tauri::command]
async fn login_microsoft() -> Result<UserProfile, String> {
    Err("Tauri Webview 暂不支持自动获取 code，请复制回调 URL 手动粘贴".to_string())
}

#[tauri::command]
async fn login_microsoft_with_code(app: AppHandle, code: String) -> Result<UserProfile, String> {
    let events = spawn_event_bridge(&app);
    let online = launcher::auth::login_with_code(&code, &events).await?;
    launcher::profiles::upsert_online(online)
}

#[tauri::command]
fn get_microsoft_auth_url() -> String {
    launcher::auth::get_auth_url()
}

/// 使用系统默认浏览器打开 Microsoft 授权页面
#[tauri::command]
fn open_microsoft_auth() -> Result<(), String> {
    let auth_url = launcher::auth::get_auth_url();
    open::that(&auth_url).map_err(|e| format!("打开浏览器失败: {}", e))
}

#[tauri::command]
fn delete_profile(name: String) -> Result<(), String> {
    launcher::profiles::delete(&name)
}

#[tauri::command]
fn reset_profile_uuid(name: String) -> Result<UserProfile, String> {
    launcher::profiles::reset_uuid(&name)
}

#[tauri::command]
fn generate_uuid() -> String {
    launcher::profiles::random_uuid()
}

#[tauri::command]
fn generate_uuid_from_name(name: String) -> String {
    launcher::profiles::uuid_from_name(&name)
}

// ── 目录与配置 ───────────────────────────────────────

#[tauri::command]
fn get_root_dir() -> Result<String, String> {
    launcher::config::get_default_root_dir().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn get_game_dir() -> Result<String, String> {
    let config = launcher::config::load()?;
    Ok(launcher::config::resolve_game_dir(&config).to_string_lossy().to_string())
}

/// 实际存档目录（始终为 root_dir/saves/）
#[tauri::command]
fn get_saves_dir() -> Result<String, String> {
    let config = launcher::config::load()?;
    Ok(PathBuf::from(&config.root_dir).join("saves").to_string_lossy().to_string())
}

/// 实际版本目录
/// isolation=false: root_dir/game/versions/
/// isolation=true:  root_dir/versions/
#[tauri::command]
fn get_versions_dir() -> Result<String, String> {
    let config = launcher::config::load()?;
    if config.version_isolation {
        Ok(PathBuf::from(&config.root_dir).join("versions").to_string_lossy().to_string())
    } else {
        Ok(PathBuf::from(&config.root_dir)
            .join("game")
            .join("versions")
            .to_string_lossy()
            .to_string())
    }
}

#[tauri::command]
fn get_config() -> Result<AppConfig, String> {
    let mut config = launcher::config::load()?;
    // first_launch 必须为 false 才视为已完成首次配置，否则强制重新配置
    if config.first_launch != false {
        config.first_launch = true;
    }
    Ok(config)
}

#[tauri::command]
fn set_config(config: AppConfig) -> Result<(), String> {
    launcher::config::save(&config)
}

#[tauri::command]
fn delete_user_config() -> Result<(), String> {
    launcher::config::reset_to_default()
}

#[tauri::command]
fn reset_all_config() -> Result<(), String> {
    launcher::config::reset_all()
}

#[tauri::command]
fn clear_game_files(game_dir: String) -> Result<(), String> {
    launcher::config::clear_dir_contents(&PathBuf::from(game_dir))
}

// ── 存档备份 ─────────────────────────────────────────

#[tauri::command]
async fn prepare_saves_isolation(version: String) -> Result<(), String> {
    launcher::backup::prepare(&version).await
}

#[tauri::command]
async fn create_saves_backup(version: String) -> Result<String, String> {
    // rsync/xcopy 是阻塞调用，放 blocking 线程池避免占用 async runtime
    tauri::async_runtime::spawn_blocking(move || launcher::backup::create(&version))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn list_saves_backups(version: String) -> Result<Vec<String>, String> {
    launcher::backup::list(&version)
}

#[tauri::command]
async fn restore_saves_backup(version: String, backup_name: String) -> Result<(), String> {
    launcher::backup::restore(&version, &backup_name).await
}

#[tauri::command]
fn delete_saves_backup(version: String, backup_name: String) -> Result<(), String> {
    launcher::backup::delete(&version, &backup_name)
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
            open_microsoft_auth
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
