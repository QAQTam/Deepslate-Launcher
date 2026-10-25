//! 深板岩启动器（Deepslate Launcher）— iced 前端。
//!
//! 只消费 `deepslate-core`：数据加载用 Task::perform，
//! 长任务（下载/启动）通过 core 的 CoreEvent 事件总线回流（见 `events` 模块）。
//! 页面切换/卡片入场动画基于 iced 内置 Animation + Float 变换。

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use iced::{
    Element, Length, Size, Subscription, Task, Vector,
    animation::Animation,
    futures::{SinkExt, Stream},
    widget::{Space, button, checkbox, column, container, float, pick_list, progress_bar, row, rule, scrollable, text, text_input},
};
use tokio::sync::{broadcast, mpsc};

use launcher::events::{CoreEvent, GameState};
use launcher::{AppConfig, ProfileStore, UserProfile};

use deepslate_core as launcher;

mod style;

mod log_window;

// ── 事件总线：core (mpsc 单消费者) → iced 订阅 (broadcast 可重订阅) ────────

static EVENT_BUS: LazyLock<broadcast::Sender<CoreEvent>> =
    LazyLock::new(|| broadcast::channel(256).0);

/// 取一个发给 core 的进度事件 sender；首次调用时启动转发线程
fn core_event_sender() -> mpsc::Sender<CoreEvent> {
    static SENDER: LazyLock<mpsc::Sender<CoreEvent>> = LazyLock::new(|| {
        let (tx, mut rx) = mpsc::channel(64);
        std::thread::Builder::new()
            .name("core-events".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build core-events runtime");
                rt.block_on(async {
                    while let Some(event) = rx.recv().await {
                        let _ = EVENT_BUS.send(event);
                    }
                });
            })
            .expect("spawn core-events thread");
        tx
    });
    SENDER.clone()
}

// ── 应用状态 ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Launch,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Home => "首页",
            Page::Launch => "启动",
            Page::Settings => "设置",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionTab {
    Release,
    Snapshot,
    Old,
}

impl VersionTab {
    fn label(self) -> &'static str {
        match self {
            VersionTab::Release => "正式版",
            VersionTab::Snapshot => "测试版",
            VersionTab::Old => "历史版",
        }
    }
}

struct App {
    page: Page,
    config: AppConfig,
    versions: Vec<launcher::download::VersionInfo>,
    /// 当前 tab 的版本缓存：只在版本列表加载 / tab 切换时重算（refilter），
    /// 避免动画帧里每帧克隆整个列表
    filtered: Vec<launcher::download::VersionInfo>,
    version_tab: VersionTab,
    selected_version: Option<launcher::download::VersionInfo>,
    profiles: Vec<UserProfile>,
    selected_profile: Option<UserProfile>,
    new_profile_name: String,
    java_list: Vec<launcher::launch::JavaInfo>,
    selected_java: Option<launcher::launch::JavaInfo>,
    status: String,
    downloading: bool,
    progress: Option<launcher::download::DownloadProgress>,
    launching: bool,
    game_running: bool,
    // 亮/暗主题（持久化在 config.ui_mode: "light"/"dark"）
    dark_mode: bool,
    // 页面入场动画（0 → 1），now 为最近一次时钟，驱动插值
    page_anim: Animation<bool>,
    now: Instant,
    // Microsoft 登录（浏览器授权 + 粘贴回调 URL）
    ms_code_input: String,
    ms_logging_in: bool,
    // 设置页：存档备份（针对当前选中版本）
    backups: Vec<String>,
    pending_restore: Option<String>,
    pending_delete: Option<String>,
    // 设置页危险区二次确认
    confirm_reset: bool,
    // 独立日志窗口子进程的写入端（None = 未启用或窗口已关闭）
    log_writer: Option<mpsc::Sender<String>>,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let config = launcher::config::load().unwrap_or_default();
        let now = Instant::now();
        // 启动即播放入场动画
        let mut page_anim = Animation::new(false)
            .duration(Duration::from_millis(400))
            .easing(iced::animation::Easing::EaseOutCubic);
        page_anim.go_mut(true, now);
        let app = Self {
            page: Page::Launch,
            status: format!("根目录: {}", config.root_dir),
            dark_mode: config.ui_mode != "light",
            page_anim,
            now,
            config,
            versions: Vec::new(),
            filtered: Vec::new(),
            version_tab: VersionTab::Release,
            selected_version: None,
            profiles: Vec::new(),
            selected_profile: None,
            new_profile_name: String::new(),
            java_list: Vec::new(),
            selected_java: None,
            downloading: false,
            progress: None,
            launching: false,
            game_running: false,
            ms_code_input: String::new(),
            ms_logging_in: false,
            backups: Vec::new(),
            pending_restore: None,
            pending_delete: None,
            confirm_reset: false,
            log_writer: None,
        };

        let load = Task::batch([
            Task::perform(launcher::download::get_versions(), Message::VersionsLoaded),
            Task::perform(async { launcher::profiles::load() }, Message::ProfilesLoaded),
            Task::perform(async { scan_java_async().await }, Message::JavaLoaded),
        ]);

        (app, load)
    }

    /// 重算当前 tab 的版本缓存，并把选择默认到该分类最新一个（列表为空则保留原选择）。
    /// 仅在版本列表加载 / tab 切换时调用。
    fn refilter(&mut self) {
        self.filtered = self
            .versions
            .iter()
            .filter(|v| match self.version_tab {
                VersionTab::Release => v.kind == "release",
                VersionTab::Snapshot => v.kind == "snapshot",
                VersionTab::Old => v.kind == "old_alpha" || v.kind == "old_beta",
            })
            .cloned()
            .collect();
        self.selected_version = self
            .filtered
            .first()
            .cloned()
            .or_else(|| self.selected_version.take());
    }
}

// ── 消息 ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    /// 动画时钟滴答
    Tick(Instant),
    /// 亮/暗主题切换
    ToggleDarkMode(bool),
    /// 启动游戏时打开独立日志窗口
    ToggleLogWindow(bool),
    VersionsLoaded(Result<Vec<launcher::download::VersionInfo>, String>),
    ProfilesLoaded(Result<ProfileStore, String>),
    JavaLoaded(Vec<launcher::launch::JavaInfo>),
    TabChanged(VersionTab),
    VersionSelected(launcher::download::VersionInfo),
    ProfileSelected(UserProfile),
    JavaSelected(launcher::launch::JavaInfo),
    NewProfileNameChanged(String),
    CreateProfile,
    ProfileCreated(Result<UserProfile, String>),
    RescanJava,
    DownloadPressed,
    DownloadFinished(Result<(), String>),
    LaunchPressed,
    LaunchFinished(Result<(), String>),
    KillPressed,
    KillFinished(Result<(), String>),
    // Microsoft 登录
    MsAuthPressed,
    MsAuthOpened(Result<(), String>),
    MsCodeChanged(String),
    MsLoginPressed,
    MsLoginFinished(Result<UserProfile, String>),
    // 设置
    RootDirChanged(String),
    ToggleVersionIsolation(bool),
    ToggleSavesIsolation(bool),
    ConcurrencySelected(usize),
    SaveSettings,
    SettingsSaved(Result<(), String>),
    ResetAllPressed,
    ResetAllDone(Result<(), String>),
    // 存档备份
    BackupCreatePressed,
    BackupCreated(Result<String, String>),
    BackupLoad(String),
    BackupsLoaded(Result<Vec<String>, String>),
    BackupRestorePressed(String),
    BackupRestored(Result<(), String>),
    BackupDeletePressed(String),
    BackupDeleted(Result<(), String>),
    CoreEvent(CoreEvent),
}

// ── 更新逻辑 ─────────────────────────────────────────────

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => {
            app.page = page;
            app.pending_restore = None;
            app.pending_delete = None;
            app.confirm_reset = false;
            // 重播页面入场动画
            app.now = Instant::now();
            app.page_anim = Animation::new(false)
                .duration(Duration::from_millis(300))
                .easing(iced::animation::Easing::EaseOutCubic);
            app.page_anim.go_mut(true, app.now);
            let backup_task = if page == Page::Settings {
                match &app.selected_version {
                    Some(v) => load_backups(v.id.clone()),
                    None => Task::none(),
                }
            } else {
                Task::none()
            };
            backup_task
        }
        Message::Tick(t) => {
            app.now = t;
            Task::none()
        }
        Message::ToggleDarkMode(dark) => {
            app.dark_mode = dark;
            app.config.ui_mode = if dark { "dark".to_string() } else { "light".to_string() };
            let config = app.config.clone();
            Task::perform(async move { launcher::config::save(&config) }, Message::SettingsSaved)
        }
        Message::ToggleLogWindow(on) => {
            app.config.log_window = on;
            let config = app.config.clone();
            Task::perform(async move { launcher::config::save(&config) }, Message::SettingsSaved)
        }
        Message::VersionsLoaded(Ok(versions)) => {
            app.versions = versions;
            app.refilter();
            Task::none()
        }
        Message::VersionsLoaded(Err(e)) => {
            app.status = format!("加载版本列表失败: {}", e);
            Task::none()
        }
        Message::ProfilesLoaded(Ok(store)) => {
            app.profiles = store.profiles.clone();
            app.selected_profile = store
                .last_used
                .as_ref()
                .and_then(|name| app.profiles.iter().find(|p| &p.name == name))
                .cloned()
                .or_else(|| app.profiles.first().cloned());
            Task::none()
        }
        Message::ProfilesLoaded(Err(e)) => {
            app.status = format!("加载档案失败: {}", e);
            Task::none()
        }
        Message::JavaLoaded(javas) => {
            app.java_list = javas;
            // 优先配置里指定的 java，否则用扫描到的第一个（scan 已按版本倒序）
            app.selected_java = app
                .config
                .java_path
                .as_ref()
                .and_then(|p| app.java_list.iter().find(|j| &j.path == p))
                .cloned()
                .or_else(|| app.java_list.first().cloned());
            if app.selected_java.is_none() {
                app.status = "未找到 Java 运行时，请安装 Java 或检查路径".to_string();
            }
            Task::none()
        }
        Message::TabChanged(tab) => {
            app.version_tab = tab;
            app.refilter();
            Task::none()
        }
        Message::VersionSelected(v) => {
            app.selected_version = Some(v);
            Task::none()
        }
        Message::ProfileSelected(p) => {
            app.selected_profile = Some(p);
            Task::none()
        }
        Message::JavaSelected(j) => {
            app.selected_java = Some(j);
            Task::none()
        }
        Message::NewProfileNameChanged(name) => {
            app.new_profile_name = name;
            Task::none()
        }
        Message::CreateProfile => {
            let name = app.new_profile_name.trim().to_string();
            Task::perform(async move { launcher::profiles::create(name) }, Message::ProfileCreated)
        }
        Message::ProfileCreated(Ok(profile)) => {
            app.status = format!("已创建档案: {}", profile.name);
            app.new_profile_name.clear();
            app.profiles.push(profile.clone());
            app.selected_profile = Some(profile);
            Task::none()
        }
        Message::ProfileCreated(Err(e)) => {
            app.status = format!("创建档案失败: {}", e);
            Task::none()
        }
        Message::RescanJava => {
            Task::perform(async { scan_java_async().await }, Message::JavaLoaded)
        }
        Message::DownloadPressed => {
            let Some(version) = app.selected_version.clone() else {
                app.status = "请先选择一个版本".to_string();
                return Task::none();
            };
            let game_dir = launcher::config::resolve_game_dir(&app.config);
            let concurrency = app.config.download_concurrency;
            let saves_isolation = app.config.saves_isolation;
            let events = core_event_sender();

            app.downloading = true;
            app.progress = None;
            app.status = format!("正在下载 {} ...", version.id);
            Task::perform(
                async move {
                    if saves_isolation {
                        launcher::backup::prepare(&version.id).await?;
                    }
                    launcher::download::run(&version.id, &game_dir, concurrency, events)
                        .await
                        .map_err(|e| e.to_string())
                },
                Message::DownloadFinished,
            )
        }
        Message::DownloadFinished(Ok(())) => {
            app.downloading = false;
            app.status = format!(
                "版本 {} 下载完成",
                app.selected_version
                    .as_ref()
                    .map(|v| v.id.as_str())
                    .unwrap_or("")
            );
            Task::none()
        }
        Message::DownloadFinished(Err(e)) => {
            app.downloading = false;
            app.status = format!("下载失败: {}", e);
            Task::none()
        }
        Message::LaunchPressed => {
            let Some(version) = app.selected_version.clone() else {
                app.status = "请先选择一个版本".to_string();
                return Task::none();
            };
            let Some(profile) = app.selected_profile.clone() else {
                app.status = "请先选择或创建档案".to_string();
                return Task::none();
            };
            let Some(java) = app.selected_java.clone() else {
                app.status = "请先选择 Java 运行时".to_string();
                return Task::none();
            };

            let saves_isolation = app.config.saves_isolation;
            let events = core_event_sender();

            // 启用日志模式时拉起独立日志窗口（复用已打开的窗口）
            if app.config.log_window && app.log_writer.is_none() {
                app.log_writer = spawn_log_window(app.dark_mode);
                if app.log_writer.is_some() {
                    app.status = format!("正在启动 {} ...（日志窗口已打开）", version.id);
                }
            }

            let opts = launcher::launch::LaunchOptions {
                version: version.id.clone(),
                game_dir: launcher::config::resolve_game_dir(&app.config),
                instance_dir: launcher::config::get_version_root(&app.config, &version.id),
                username: profile.name,
                uuid: profile.uuid,
                runtime_dir: std::path::PathBuf::from(java.path),
                auth_method: profile.auth_method,
                access_token: profile.access_token,
            };

            app.launching = true;
            app.game_running = true;
            app.status = format!("正在启动 {} ...", version.id);
            Task::perform(
                async move {
                    if saves_isolation {
                        launcher::backup::prepare(&opts.version).await?;
                    }
                    launcher::launch::launch(opts, events)
                        .await
                        .map_err(|e| e.to_string())
                },
                Message::LaunchFinished,
            )
        }
        Message::LaunchFinished(Ok(())) => {
            app.launching = false;
            app.game_running = false;
            app.status = "游戏已退出".to_string();
            Task::none()
        }
        Message::LaunchFinished(Err(e)) => {
            app.launching = false;
            app.game_running = false;
            app.status = format!("启动失败: {}", e);
            Task::none()
        }
        Message::KillPressed => {
            app.status = "正在终止游戏...".to_string();
            Task::perform(launcher::launch::kill_game(), Message::KillFinished)
        }
        Message::KillFinished(Ok(())) => {
            app.game_running = false;
            app.status = "游戏已终止".to_string();
            Task::none()
        }
        Message::KillFinished(Err(e)) => {
            app.status = format!("终止失败: {}", e);
            Task::none()
        }
        Message::MsAuthPressed => {
            app.status = "已打开浏览器，请在浏览器中完成 Microsoft 授权...".to_string();
            let url = launcher::auth::get_auth_url();
            Task::perform(
                async move { open::that(url).map_err(|e| e.to_string()) },
                Message::MsAuthOpened,
            )
        }
        Message::MsAuthOpened(Ok(())) => Task::none(),
        Message::MsAuthOpened(Err(e)) => {
            app.status = format!("打开浏览器失败: {}", e);
            Task::none()
        }
        Message::MsCodeChanged(code) => {
            app.ms_code_input = code;
            Task::none()
        }
        Message::MsLoginPressed => {
            let Some(code) = extract_auth_code(&app.ms_code_input) else {
                app.status = "未在输入中找到 code（可粘贴完整回调 URL 或仅 code 值）".to_string();
                return Task::none();
            };
            let events = core_event_sender();
            app.ms_logging_in = true;
            app.status = "正在完成 Microsoft 登录...".to_string();
            Task::perform(
                async move {
                    let online = launcher::auth::login_with_code(&code, &events).await?;
                    launcher::profiles::upsert_online(online)
                },
                Message::MsLoginFinished,
            )
        }
        Message::MsLoginFinished(Ok(profile)) => {
            app.ms_logging_in = false;
            app.ms_code_input.clear();
            let existing = app.profiles.iter().position(|p| p.uuid == profile.uuid);
            if let Some(i) = existing {
                app.profiles[i] = profile.clone();
            } else {
                app.profiles.push(profile.clone());
            }
            app.selected_profile = Some(profile.clone());
            app.status = format!("Microsoft 登录成功: {}", profile.name);
            Task::none()
        }
        Message::MsLoginFinished(Err(e)) => {
            app.ms_logging_in = false;
            app.status = format!("登录失败: {}", e);
            Task::none()
        }
        Message::RootDirChanged(dir) => {
            app.config.root_dir = dir;
            Task::none()
        }
        Message::ToggleVersionIsolation(on) => {
            app.config.version_isolation = on;
            Task::none()
        }
        Message::ToggleSavesIsolation(on) => {
            app.config.saves_isolation = on;
            Task::none()
        }
        Message::ConcurrencySelected(n) => {
            app.config.download_concurrency = n;
            Task::none()
        }
        Message::SaveSettings => {
            let mut config = app.config.clone();
            if let Some(java) = &app.selected_java {
                config.java_path = Some(java.path.clone());
            }
            if let Some(profile) = &app.selected_profile {
                config.last_profile = Some(profile.name.clone());
            }
            Task::perform(async move { launcher::config::save(&config) }, Message::SettingsSaved)
        }
        Message::SettingsSaved(Ok(())) => {
            app.status = "设置已保存".to_string();
            Task::none()
        }
        Message::SettingsSaved(Err(e)) => {
            app.status = format!("保存设置失败: {}", e);
            Task::none()
        }
        Message::ResetAllPressed => {
            if app.confirm_reset {
                app.confirm_reset = false;
                Task::perform(async { launcher::config::reset_all() }, Message::ResetAllDone)
            } else {
                app.confirm_reset = true;
                app.status = "再次点击红色按钮才会真正重置".to_string();
                Task::none()
            }
        }
        Message::ResetAllDone(Ok(())) => {
            app.config = AppConfig::default();
            app.profiles.clear();
            app.selected_profile = None;
            app.status = "所有配置已重置".to_string();
            Task::perform(async { launcher::profiles::load() }, Message::ProfilesLoaded)
        }
        Message::ResetAllDone(Err(e)) => {
            app.status = format!("重置失败: {}", e);
            Task::none()
        }
        Message::BackupCreatePressed => {
            let Some(version) = app.selected_version.as_ref().map(|v| v.id.clone()) else {
                app.status = "请先在「启动」页选择一个版本".to_string();
                return Task::none();
            };
            if app.game_running || app.launching {
                app.status = "游戏运行中，不建议备份存档".to_string();
                return Task::none();
            }
            app.status = "正在创建存档备份...".to_string();
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || launcher::backup::create(&version))
                        .await
                        .map_err(|e| e.to_string())?
                },
                Message::BackupCreated,
            )
        }
        Message::BackupCreated(Ok(name)) => {
            app.status = format!("备份已创建: {}", name);
            match app.selected_version.clone() {
                Some(v) => load_backups(v.id),
                None => Task::none(),
            }
        }
        Message::BackupCreated(Err(e)) => {
            app.status = format!("创建备份失败: {}", e);
            Task::none()
        }
        Message::BackupLoad(version) => load_backups(version),
        Message::BackupsLoaded(Ok(backups)) => {
            app.backups = backups;
            Task::none()
        }
        Message::BackupsLoaded(Err(e)) => {
            app.status = format!("加载备份列表失败: {}", e);
            Task::none()
        }
        Message::BackupRestorePressed(name) => {
            if app.pending_restore.as_deref() == Some(name.as_str()) {
                // 二次确认通过，执行恢复
                app.pending_restore = None;
                let Some(version) = app.selected_version.as_ref().map(|v| v.id.clone()) else {
                    app.status = "请先选择版本".to_string();
                    return Task::none();
                };
                app.status = "正在恢复存档...".to_string();
                Task::perform(
                    async move { launcher::backup::restore(&version, &name).await },
                    Message::BackupRestored,
                )
            } else {
                app.pending_restore = Some(name);
                app.pending_delete = None;
                app.status = "恢复会覆盖当前存档，再点一次确认".to_string();
                Task::none()
            }
        }
        Message::BackupRestored(Ok(())) => {
            app.status = "存档已恢复".to_string();
            match app.selected_version.clone() {
                Some(v) => load_backups(v.id),
                None => Task::none(),
            }
        }
        Message::BackupRestored(Err(e)) => {
            app.status = format!("恢复失败: {}", e);
            Task::none()
        }
        Message::BackupDeletePressed(name) => {
            if app.pending_delete.as_deref() == Some(name.as_str()) {
                app.pending_delete = None;
                let Some(version) = app.selected_version.as_ref().map(|v| v.id.clone()) else {
                    app.status = "请先选择版本".to_string();
                    return Task::none();
                };
                Task::perform(
                    async move { launcher::backup::delete(&version, &name) },
                    Message::BackupDeleted,
                )
            } else {
                app.pending_delete = Some(name);
                app.pending_restore = None;
                Task::none()
            }
        }
        Message::BackupDeleted(Ok(())) => {
            app.status = "备份已删除".to_string();
            match app.selected_version.clone() {
                Some(v) => load_backups(v.id),
                None => Task::none(),
            }
        }
        Message::BackupDeleted(Err(e)) => {
            app.status = format!("删除备份失败: {}", e);
            Task::none()
        }
        Message::CoreEvent(event) => match event {
            CoreEvent::DownloadProgress(p) => {
                if p.percent >= 100 {
                    app.downloading = false;
                }
                app.progress = Some(p);
                Task::none()
            }
            CoreEvent::AuthProgress(p) => {
                app.status = format!("认证: {}", p.message);
                Task::none()
            }
            CoreEvent::GameState(state) => {
                match state {
                    GameState::Launching { .. } => {
                        app.launching = true;
                        app.game_running = true;
                    }
                    GameState::Running { .. } => {
                        app.launching = false;
                        app.game_running = true;
                    }
                    GameState::Stopped { .. } => {
                        app.launching = false;
                        app.game_running = false;
                    }
                }
                Task::none()
            }
            CoreEvent::GameLog { is_stderr, line } => {
                // 转发给独立日志窗口子进程；窗口已关闭则丢弃
                if let Some(tx) = &app.log_writer {
                    let payload = format!("{}\t{}", if is_stderr { "E" } else { "O" }, line);
                    if matches!(
                        tx.try_send(payload),
                        Err(mpsc::error::TrySendError::Closed(_))
                    ) {
                        app.log_writer = None;
                    }
                }
                Task::none()
            }
        },
    }
}

/// 拉起独立日志窗口子进程（deepslate-iced --log-window），返回向其 stdin 写行的 sender
fn spawn_log_window(dark: bool) -> Option<mpsc::Sender<String>> {
    let exe = std::env::current_exe().ok()?;
    let (tx, mut rx) = mpsc::channel::<String>(1024);
    std::thread::Builder::new()
        .name("log-window".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                let mut child = match tokio::process::Command::new(&exe)
                    .arg("--log-window")
                    .arg(if dark { "--dark" } else { "--light" })
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(child) => child,
                    Err(_) => {
                        // 窗口起不来：排空通道，避免主线程 try_send 阻塞语义混淆
                        while rx.recv().await.is_some() {}
                        return;
                    }
                };
                let Some(mut stdin) = child.stdin.take() else {
                    return;
                };
                use tokio::io::AsyncWriteExt;
                while let Some(line) = rx.recv().await {
                    let ok = stdin.write_all(line.as_bytes()).await.is_ok()
                        && stdin.write_all(b"\n").await.is_ok();
                    if !ok {
                        break;
                    }
                }
                drop(stdin);
                let _ = child.wait().await;
            });
        })
        .ok()?;
    Some(tx)
}

/// 拉取指定版本的备份列表
fn load_backups(version: String) -> Task<Message> {
    Task::perform(async move { launcher::backup::list(&version) }, Message::BackupsLoaded)
}

/// 从粘贴的回调 URL 中提取授权码，也接受直接粘贴的裸 code
fn extract_auth_code(input: &str) -> Option<String> {
    if let Some(pos) = input.find("code=") {
        let rest = &input[pos + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        let code = rest[..end].trim();
        if !code.is_empty() {
            return Some(code.to_string());
        }
    }
    let trimmed = input.trim();
    (!trimmed.is_empty() && !trimmed.contains(' ') && !trimmed.contains("://"))
        .then(|| trimmed.to_string())
}

/// 在 blocking 线程池跑 Java 扫描（内部是子进程调用，会阻塞）
async fn scan_java_async() -> Vec<launcher::launch::JavaInfo> {
    match tokio::task::spawn_blocking(launcher::launch::scan_java).await {
        Ok(Ok(javas)) => javas,
        _ => Vec::new(),
    }
}

// ── 订阅 ─────────────────────────────────────────────────

fn subscription(app: &App) -> Subscription<Message> {
    let mut subs = vec![Subscription::run(listen_core_events)];
    // 入场动画进行中：16ms 滴答驱动重绘，动画结束自动停
    if app.page_anim.is_animating(app.now) {
        subs.push(iced::time::every(Duration::from_millis(33)).map(Message::Tick));
    }
    Subscription::batch(subs)
}

fn listen_core_events() -> impl Stream<Item = Message> {
    iced::stream::channel(64, |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
        let mut rx = EVENT_BUS.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if output.send(Message::CoreEvent(event)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

// ── 视图 ─────────────────────────────────────────────────

fn view(app: &App) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);

    let nav = |page: Page| {
        let active = app.page == page;
        button(text(page.label()).size(14))
            .width(Length::Fill)
            .padding(7.0)
            .style(style::nav(active))
            .on_press(Message::Navigate(page))
    };

    let busy = app.game_running || app.launching;
    let status_dot = if busy { s.accent } else { s.text_faint };

    let sidebar = container(
        column![
            row![
                text("◆").size(18).color(s.accent),
                column![
                    text("Deepslate").size(17).color(s.text_primary),
                    text("深板岩启动器").size(9).color(s.text_faint),
                ],
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            Space::new().height(Length::from(20)),
            nav(Page::Home),
            nav(Page::Launch),
            nav(Page::Settings),
            Space::new().height(Length::Fill),
            row![
                text("●").size(10).color(status_dot),
                text(game_state_label(app)).size(11).color(s.text_muted),
            ]
            .spacing(6),
        ]
        .spacing(4)
        .padding(14),
    )
    .width(Length::from(168))
    .height(Length::Fill)
    .style(style::sidebar);

    // 页面自身按窗口尺寸布局（网格/弹性区域），不使用外层滚动
    let content = container(page_view(app))
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill);

    container(row![sidebar, content])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::page)
        .into()
}

/// 带标题的卡片区块
fn section_card<'a>(
    s: &style::Scheme,
    title: &str,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    container(
        column![
            text(title.to_string()).size(15).color(s.text_primary),
            body,
        ]
        .spacing(10),
    )
    .padding(16)
    .style(style::card)
    .into()
}

/// 卡片入场：随页面进度 t 依次上浮淡入（delay 为错峰偏移，动画结束后零开销）
fn enter<'a>(t: f32, delay: f32, element: Element<'a, Message>) -> Element<'a, Message> {
    let ct = ((t - delay) / 0.5).clamp(0.0, 1.0);
    if ct >= 1.0 {
        return element;
    }
    let dy = (1.0 - ct) * 14.0;
    let scale = 1.0 - (1.0 - ct) * 0.03;
    float(element)
        .scale(scale)
        .translate(move |_viewport, _bounds| Vector::new(0.0, dy))
        .into()
}

fn game_state_label(app: &App) -> &'static str {
    if app.launching {
        "游戏启动中..."
    } else if app.game_running {
        "游戏运行中"
    } else {
        "空闲"
    }
}

fn page_view(app: &App) -> Element<'_, Message> {
    let t = app.page_anim.interpolate(0.0f32, 1.0, app.now);
    let page = match app.page {
        Page::Launch => launch_page(app, t),
        Page::Home => home_page(app, t),
        Page::Settings => settings_page(app, t),
    };
    // 整页轻微上浮入场
    if t >= 1.0 {
        page
    } else {
        let dy = (1.0 - t) * 10.0;
        float(page)
            .translate(move |_viewport, _bounds| Vector::new(0.0, dy))
            .into()
    }
}

/// Microsoft 登录：系统浏览器授权 + 粘贴回调 URL（webview 无法自动捕获 code）
fn ms_login_section<'a>(app: &'a App, s: &'static style::Scheme) -> Element<'a, Message> {
    let login_btn = {
        let b = button(text("完成登录").size(13)).padding(6.0);
        if app.ms_logging_in || app.ms_code_input.trim().is_empty() {
            b
        } else {
            b.style(style::primary).on_press(Message::MsLoginPressed)
        }
    };

    column![
        row![
            button(text("打开微软登录页").size(12))
                .padding(6.0)
                .style(style::secondary)
                .on_press(Message::MsAuthPressed),
            text("浏览器完成授权后，把跳转到的 URL 粘贴到下面")
                .size(11)
                .color(s.text_faint),
        ]
        .spacing(8),
        row![
            text_input("粘贴回调 URL（含 code=...）", &app.ms_code_input)
                .on_input(Message::MsCodeChanged)
                .on_submit(Message::MsLoginPressed)
                .style(style::text_field),
            login_btn,
        ]
        .spacing(8),
    ]
    .spacing(8)
    .into()
}

fn launch_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let tab_row = row(
        [VersionTab::Release, VersionTab::Snapshot, VersionTab::Old]
            .iter()
            .map(|tab| {
                let active = app.version_tab == *tab;
                button(text(tab.label()).size(13))
                    .padding(7.0)
                    .style(style::nav(active))
                    .on_press(Message::TabChanged(*tab))
                    .into()
            })
            .collect::<Vec<_>>(),
    )
    .spacing(6);

    let version_picker = pick_list(
        app.filtered.as_slice(),
        app.selected_version.as_ref(),
        Message::VersionSelected,
    )
    .placeholder("选择版本")
    .style(style::pick);

    let profile_picker = pick_list(
        app.profiles.clone(),
        app.selected_profile.as_ref(),
        Message::ProfileSelected,
    )
    .placeholder("选择档案")
    .style(style::pick);

    let new_profile_input = text_input("新建离线档案...", &app.new_profile_name)
        .on_input(Message::NewProfileNameChanged)
        .on_submit(Message::CreateProfile)
        .style(style::text_field);

    let java_picker = pick_list(
        app.java_list.clone(),
        app.selected_java.as_ref(),
        Message::JavaSelected,
    )
    .placeholder("选择 Java")
    .style(style::pick);

    // 下载按钮
    let download_btn = {
        let b = button(text("下载版本")).padding(8.0);
        if app.downloading || app.launching || app.selected_version.is_none() {
            b
        } else {
            b.style(style::primary).on_press(Message::DownloadPressed)
        }
    };

    // 启动/终止按钮
    let launch_btn = if app.game_running || app.launching {
        button(text("终止游戏"))
            .padding(8.0)
            .style(style::danger)
            .on_press(Message::KillPressed)
    } else {
        let ready = app.selected_version.is_some()
            && app.selected_profile.is_some()
            && app.selected_java.is_some();
        let b = button(text("启动游戏")).padding(8.0);
        if ready {
            b.style(style::primary).on_press(Message::LaunchPressed)
        } else {
            b
        }
    };

    let progress_section: Element<Message> = match (&app.progress, app.downloading) {
        (Some(p), _) => column![
            progress_bar(0.0..=100.0, p.percent as f32).style(style::progress),
            text(format!("{}/{}（{}%）— {}", p.current, p.total, p.percent, p.current_file))
                .size(12)
                .color(s.text_muted),
        ]
        .spacing(8)
        .into(),
        (None, true) => text("准备下载...").size(12).color(s.text_muted).into(),
        (None, false) => Space::new().height(Length::from(1)).into(),
    };

    let selected_summary = row![
        text("档案:").size(12).color(s.text_muted),
        text(
            app.selected_profile
                .as_ref()
                .map(|p| p.name.as_str())
                .unwrap_or("未选择")
        )
        .size(12)
        .color(s.text_primary),
        text("版本:").size(12).color(s.text_muted),
        text(
            app.selected_version
                .as_ref()
                .map(|v| v.id.as_str())
                .unwrap_or("未选择")
        )
        .size(12)
        .color(s.text_primary),
    ]
    .spacing(8);

    let version_card = section_card(s, "版本", column![tab_row, version_picker].spacing(10).into());
    let profile_card = section_card(
        s,
        "档案",
        column![
            row![profile_picker, new_profile_input].spacing(8),
            ms_login_section(app, s),
        ]
        .spacing(10)
        .into(),
    );
    let java_card = section_card(
        s,
        "Java 运行时",
        row![
            java_picker,
            button(text("重新扫描").size(13))
                .padding(6.0)
                .style(style::secondary)
                .on_press(Message::RescanJava),
        ]
        .spacing(8)
        .into(),
    );
    let actions_card = section_card(
        s,
        "启动",
        column![
            row![download_btn, launch_btn].spacing(12),
            progress_section,
            selected_summary,
            text(app.status.as_str()).size(12).color(s.text_muted),
        ]
        .spacing(12)
        .into(),
    );

    // 双栏网格：左（版本/档案）右（Java/启动），整页不滚动
    let left = column![enter(t, 0.00, version_card), enter(t, 0.12, profile_card)]
        .spacing(16)
        .width(Length::Fill);
    let right = column![enter(t, 0.24, java_card), enter(t, 0.36, actions_card)]
        .spacing(16)
        .width(Length::Fill);

    column![
        text("启动").size(28).color(s.text_primary),
        text(format!(
            "游戏目录: {}",
            launcher::config::resolve_game_dir(&app.config).display()
        ))
        .size(11)
        .color(s.text_faint),
        rule::horizontal(1.0).style(style::rule_style),
        row![left, right].spacing(16),
    ]
    .spacing(16)
    .height(Length::Fill)
    .into()
}

// ── 首页 ─────────────────────────────────────────────────

fn info_row<'a>(s: &style::Scheme, label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(13).color(s.text_muted),
        text(value).size(13).color(s.text_primary),
    ]
    .spacing(8)
    .into()
}

fn home_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);

    let profile_line = info_row(s, 
        "档案:",
        app.selected_profile
            .as_ref()
            .map(|p| {
                format!(
                    "{}（{}）",
                    p.name,
                    if p.is_online() { "在线" } else { "离线" }
                )
            })
            .unwrap_or_else(|| "未选择".to_string()),
    );
    let version_line = info_row(s, 
        "版本:",
        app.selected_version
            .as_ref()
            .map(|v| v.id.clone())
            .unwrap_or_else(|| "未选择".to_string()),
    );
    let java_line = info_row(s, 
        "Java:",
        app.selected_java
            .as_ref()
            .map(|j| j.version.clone())
            .unwrap_or_else(|| "未找到".to_string()),
    );
    let state_line = info_row(s, "游戏:", game_state_label(app).to_string());
    let root_line = info_row(s, "根目录:", app.config.root_dir.clone());

    let status_card = section_card(
        s,
        "当前状态",
        column![
            profile_line,
            version_line,
            java_line,
            state_line,
            root_line,
        ]
        .spacing(6)
        .into(),
    );

    let launch_btn = if app.game_running || app.launching {
        button(text("终止游戏"))
            .padding(8.0)
            .style(style::danger)
            .on_press(Message::KillPressed)
    } else {
        let b = button(text("启动游戏")).padding(8.0);
        let ready = app.selected_version.is_some() && app.selected_profile.is_some()
            && app.selected_java.is_some();
        if ready {
            b.style(style::primary).on_press(Message::LaunchPressed)
        } else {
            b
        }
    };

    let download_btn = {
        let b = button(text("下载当前版本")).padding(8.0);
        if app.downloading || app.selected_version.is_none() {
            b
        } else {
            b.style(style::primary).on_press(Message::DownloadPressed)
        }
    };

    column![
        text("首页").size(28).color(s.text_primary),
        rule::horizontal(1.0).style(style::rule_style),
        enter(t, 0.0, status_card),
        enter(t, 0.12, row![launch_btn, download_btn,].spacing(12).into()),
        text("提示：版本 / 档案 / Java 在「启动」页选择；目录与隔离选项在「设置」页。")
            .size(12)
            .color(s.text_faint),
        text(app.status.as_str()).size(12).color(s.text_muted),
    ]
    .spacing(16)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── 设置页 ───────────────────────────────────────────────

fn settings_page(app: &App, t: f32) -> Element<'_, Message> {
    let s = style::scheme_of(app.dark_mode);
    let config_path = launcher::config::config_path()
        .map(|p| p.join("config.json").display().to_string())
        .unwrap_or_else(|e| e);
    let game_dir = launcher::config::resolve_game_dir(&app.config).display().to_string();
    let saves_dir = format!("{}/saves", app.config.root_dir);
    let versions_dir = if app.config.version_isolation {
        format!("{}/versions", app.config.root_dir)
    } else {
        format!("{}/game/versions", app.config.root_dir)
    };

    // 存档备份区块（针对当前选中版本）
    let backup_section: Element<Message> = match &app.selected_version {
        Some(version) => {
            let game_busy = app.game_running || app.launching;
            let create_btn = {
                let b = button(text("创建备份")).padding(8.0);
                if game_busy {
                    b
                } else {
                    b.style(style::primary).on_press(Message::BackupCreatePressed)
                }
            };

            let saves_src = launcher::backup::version_saves_dir(&app.config, &version.id);
            let list: Element<Message> = if app.backups.is_empty() {
                text("暂无备份").size(12).color(s.text_muted).into()
            } else {
                let rows = app
                    .backups
                    .iter()
                    .map(|name| {
                        let restoring = app.pending_restore.as_deref() == Some(name.as_str());
                        let deleting = app.pending_delete.as_deref() == Some(name.as_str());
                        let restore_btn = {
                            let b = button(text(if restoring {
                                "确认恢复？"
                            } else {
                                "恢复"
                            })
                            .size(12))
                            .padding(5.0);
                            let b = if restoring {
                                b.style(style::danger)
                            } else {
                                b.style(style::secondary)
                            };
                            if game_busy {
                                b
                            } else {
                                b.on_press(Message::BackupRestorePressed(name.clone()))
                            }
                        };
                        let delete_btn = {
                            let b = button(text(if deleting { "确认删除？" } else { "删除" }).size(12))
                                .padding(5.0);
                            if deleting {
                                b.style(style::danger)
                            } else {
                                b.style(style::secondary)
                            }
                            .on_press(Message::BackupDeletePressed(name.clone()))
                        };
                        row![
                            text(name.clone()).size(12),
                            Space::new().width(Length::Fill),
                            restore_btn,
                            delete_btn,
                        ]
                        .spacing(6)
                        .into()
                    })
                    .collect::<Vec<_>>();
                column(rows).spacing(6).into()
            };

            column![
                row![
                    text(format!("存档备份 — {}", version.id)).size(15).color(s.text_primary),
                    Space::new().width(Length::Fill),
                    create_btn,
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
                text(format!("源目录: {}", saves_src.display())).size(11).color(s.text_muted),
                // 弹性列表区域：只在备份条目过多或窗口较小时出现内部滚动
                scrollable(list).height(Length::Fill).width(Length::Fill),
                if game_busy {
                    text("游戏运行中，恢复/备份已禁用").size(11).color(s.text_muted)
                } else {
                    text("恢复会覆盖当前存档（自动保留最近 5 份备份）").size(11).color(s.text_muted)
                },
            ]
            .spacing(8)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        }
        None => column![
            text("存档备份").size(15).color(s.text_primary),
            text("请先在「启动」页选择一个版本").size(12).color(s.text_muted),
        ]
        .spacing(6)
        .into(),
    };

    const CONCURRENCY: [usize; 6] = [4, 8, 16, 32, 64, 128];
    let concurrency_picker = pick_list(
        CONCURRENCY.to_vec(),
        CONCURRENCY
            .iter()
            .find(|&&c| c == app.config.download_concurrency),
        Message::ConcurrencySelected,
    )
    .placeholder("并发数")
    .style(style::pick);

    let reset_btn = if app.confirm_reset {
        button(text("确认重置？（再点一次执行）"))
            .padding(8.0)
            .style(style::danger)
            .on_press(Message::ResetAllPressed)
    } else {
        button(text("重置所有配置"))
            .padding(8.0)
            .style(style::danger)
            .on_press(Message::ResetAllPressed)
    };

    let appearance_card = section_card(
        s,
        "外观",
        column![
            checkbox(app.dark_mode)
                .label("暗色模式（牛皮纸黄 × 石板灰）")
                .on_toggle(Message::ToggleDarkMode)
                .size(13)
                .style(style::checkbox_style),
            checkbox(app.config.log_window)
                .label("启动游戏时打开独立日志窗口（stdout/stderr，超出 2000 行自动压缩）")
                .on_toggle(Message::ToggleLogWindow)
                .size(13)
                .style(style::checkbox_style),
        ]
        .spacing(10)
        .into(),
    );
    // 常规：根目录 + 下载并发数合并为一张卡
    let general_card = section_card(
        s,
        "常规",
        column![
            text_input("根目录，如 /home/xxx/Games/ObsidianLauncher", &app.config.root_dir)
                .on_input(Message::RootDirChanged)
                .style(style::text_field),
            row![
                text("下载并发数").size(13).color(s.text_muted),
                concurrency_picker,
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(10)
        .into(),
    );
    let isolation_card = section_card(
        s,
        "目录隔离",
        column![
            checkbox(app.config.version_isolation)
                .label("版本隔离（每个版本独立实例目录）")
                .on_toggle(Message::ToggleVersionIsolation)
                .size(13)
                .style(style::checkbox_style),
            checkbox(app.config.saves_isolation)
                .label("存档隔离（每版本独立 saves，符号链接实现）")
                .on_toggle(Message::ToggleSavesIsolation)
                .size(13)
                .style(style::checkbox_style),
        ]
        .spacing(10)
        .into(),
    );
    let dirs_card = section_card(
        s,
        "实际目录",
        column![
            text(format!("配置文件: {}", config_path)).size(12).color(s.text_muted),
            text(format!("游戏目录: {}", game_dir)).size(12).color(s.text_muted),
            text(format!("存档目录: {}", saves_dir)).size(12).color(s.text_muted),
            text(format!("版本目录: {}", versions_dir)).size(12).color(s.text_muted),
        ]
        .spacing(5)
        .into(),
    );
    // 备份卡：弹性填充剩余高度，列表过长时只在卡内滚动
    let backup_card: Element<Message> = container(backup_section)
        .padding(16)
        .style(style::card)
        .height(Length::Fill)
        .width(Length::Fill)
        .into();

    // 2×2 网格 + 底部弹性备份区，整页不滚动
    let grid_left = column![enter(t, 0.00, general_card), enter(t, 0.10, appearance_card)]
        .spacing(16)
        .width(Length::Fill);
    let grid_right = column![enter(t, 0.20, isolation_card), enter(t, 0.30, dirs_card)]
        .spacing(16)
        .width(Length::Fill);

    column![
        text("设置").size(28).color(s.text_primary),
        rule::horizontal(1.0).style(style::rule_style),
        row![grid_left, grid_right].spacing(16),
        enter(t, 0.40, backup_card),
        row![
            button(text("保存设置"))
                .padding(8.0)
                .style(style::primary)
                .on_press(Message::SaveSettings),
            reset_btn,
        ]
        .spacing(12),
        text("保存时会同时记录当前选中的档案与 Java 路径。重置会删除 config.json 与 profiles.json（游戏文件不受影响）。")
            .size(12)
            .color(s.text_faint),
    ]
    .spacing(16)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── 入口 ─────────────────────────────────────────────────

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--log-window") {
        // 独立日志窗口子进程模式：主进程经 stdin 逐行喂日志
        let dark = !args.iter().any(|a| a == "--light");
        return log_window::run(dark);
    }

    iced::application(App::boot, update, view)
        .title(|_app: &App| "深板岩启动器".to_string())
        .theme(|app: &App| if app.dark_mode { style::theme_dark() } else { style::theme_light() })
        .subscription(subscription)
        .window(iced::window::Settings {
            min_size: Some(Size::new(900.0, 560.0)),
            ..Default::default()
        })
        .window_size(Size::new(1040.0, 700.0))
        .run()
}

// ── 性能诊断测试：页面切换卡顿根因分析 ────────────────────
//
// 运行：cargo test -p gui-iced switch_perf -- --nocapture
// 输出各项每帧耗时（µs），定位卡顿出在 update / view 重建 / 数据规模哪一环。

#[cfg(test)]
mod switch_perf {
    use super::*;
    use std::hint::black_box;
    use std::time::Duration;

    fn make_app(n_versions: usize) -> App {
        let versions: Vec<launcher::download::VersionInfo> = (0..n_versions)
            .map(|i| launcher::download::VersionInfo {
                id: format!("1.20.{}", i),
                kind: if i % 5 == 0 { "snapshot" } else { "release" }.to_string(),
                url: format!("https://example.com/v{}.json", i),
                release_time: "2026-01-01T00:00:00Z".to_string(),
            })
            .collect();
        let selected = versions.first().cloned();
        let profiles: Vec<UserProfile> = (0..3)
            .map(|i| UserProfile {
                name: format!("player{}", i),
                uuid: format!("00000000-0000-0000-0000-{:012}", i),
                auth_method: launcher::AuthMethod::Offline,
                access_token: None,
                refresh_token: None,
                expires_at: None,
            })
            .collect();
        let java_list: Vec<launcher::launch::JavaInfo> = (0..3)
            .map(|i| launcher::launch::JavaInfo {
                path: format!("/usr/lib/jvm/jdk-2{}/", i),
                version: format!("21.0.{}", i),
            })
            .collect();

        let mut app = App {
            page: Page::Launch,
            status: String::new(),
            dark_mode: true,
            page_anim: Animation::new(true),
            now: Instant::now(),
            config: AppConfig::default(),
            versions,
            filtered: Vec::new(),
            version_tab: VersionTab::Release,
            selected_version: selected,
            profiles,
            selected_profile: None,
            new_profile_name: String::new(),
            selected_java: java_list.first().cloned(),
            java_list,
            downloading: false,
            progress: None,
            launching: false,
            game_running: false,
            ms_code_input: String::new(),
            ms_logging_in: false,
            backups: (0..6).map(|i| format!("2026-10-2{}_12-00-00", i)).collect(),
            pending_restore: None,
            pending_delete: None,
            confirm_reset: false,
            log_writer: None,
        };
        app.refilter();
        app
    }

    /// 连续 frames 帧：推进 16ms 时钟 + update(Tick) + view() 重建，返回 (平均, 最大)
    fn run_frames(app: &mut App, page: Page, frames: usize, animated: bool) -> (f64, f64) {
        update(app, Message::Navigate(page));
        if !animated {
            // 快进到动画结束后
            app.now += Duration::from_secs(2);
            update(app, Message::Tick(app.now));
        }
        let mut total_us = 0.0f64;
        let mut max_us = 0.0f64;
        for _ in 0..frames {
            let now = app.now + Duration::from_millis(16);
            let t0 = Instant::now();
            update(app, Message::Tick(now));
            black_box(view(app));
            let us = t0.elapsed().as_secs_f64() * 1e6;
            total_us += us;
            max_us = max_us.max(us);
        }
        (total_us / frames as f64, max_us)
    }

    fn avg<F: FnMut()>(mut f: F, iters: usize) -> f64 {
        // 预热
        for _ in 0..3 {
            f();
        }
        let t0 = Instant::now();
        for _ in 0..iters {
            f();
        }
        t0.elapsed().as_secs_f64() * 1e6 / iters as f64
    }

    #[test]
    fn page_switch_profile() {
        let mut big = make_app(800);
        let mut small = make_app(10);

        // 1. Navigate 本身的成本（重置动画 + 分支处理）
        let nav_big = avg(
            || {
                black_box(update(&mut big, Message::Navigate(Page::Home)));
            },
            200,
        );
        let nav_small = avg(
            || {
                black_box(update(&mut small, Message::Navigate(Page::Home)));
            },
            200,
        );

        // 2. 动画进行中（enter/float 包装生效）每帧 update+view 成本
        let (anim_launch_big, anim_launch_big_max) = run_frames(&mut big, Page::Launch, 30, true);
        let (anim_launch_small, _) = run_frames(&mut small, Page::Launch, 30, true);
        let (anim_settings_big, _) = run_frames(&mut big, Page::Settings, 30, true);
        let (anim_home_big, _) = run_frames(&mut big, Page::Home, 30, true);

        // 3. 动画结束后（无 enter 包装）每帧成本 —— 对照组
        let (idle_launch_big, _) = run_frames(&mut big, Page::Launch, 30, false);
        let (idle_settings_big, _) = run_frames(&mut big, Page::Settings, 30, false);

        // 4. 过滤缓存重算成本（现在仅发生在版本加载/tab 切换，而非每帧）
        let fv_big = avg(
            || {
                black_box(big.refilter());
            },
            500,
        );

        // 5. subscription 求值成本（每条消息后都会调用）
        let sub_cost = avg(
            || {
                black_box(subscription(&big));
            },
            2000,
        );

        println!("┌─ 页面切换性能剖析（debug 构建, µs/次）────────────────");
        println!("│ Navigate update (800 版本)          {:>10.1}", nav_big);
        println!("│ Navigate update (10 版本)           {:>10.1}", nav_small);
        println!("│ 动画中每帧 update+view·启动页(800)  {:>10.1}（最差 {:.1}）", anim_launch_big, anim_launch_big_max);
        println!("│ 动画中每帧 update+view·启动页(10)   {:>10.1}", anim_launch_small);
        println!("│ 动画中每帧 update+view·设置页(800)  {:>10.1}", anim_settings_big);
        println!("│ 动画中每帧 update+view·首页(800)    {:>10.1}", anim_home_big);
        println!("│ 空闲每帧 update+view·启动页(800)    {:>10.1}", idle_launch_big);
        println!("│ 空闲每帧 update+view·设置页(800)    {:>10.1}", idle_settings_big);
        println!("│ refilter() 重算（仅 tab 切换时）(800){:>10.1}", fv_big);
        println!("│ subscription() 求值                 {:>10.1}", sub_cost);
        println!("└──────────────────────────────────────────────────────");

        // 60fps 预算为 16667µs/帧；构造阶段超预算即解释卡顿
        assert!(
            anim_launch_big < 16_000.0,
            "动画中启动页每帧构造耗时 {:.0}µs，超出 60fps 预算",
            anim_launch_big
        );
    }

    /// 守护：页面入场动画的 16ms 滴答订阅必须自动终止，
    /// 否则会变成常驻 60fps 重绘（表现为整体持续卡顿）
    #[test]
    fn animation_tick_subscription_terminates() {
        let mut app = make_app(10);
        update(&mut app, Message::Navigate(Page::Home));

        // 动画 450ms；滴答走 1s（62 帧）后订阅里应不再包含 time 滴答
        let mut ticks = 0;
        for _ in 0..62 {
            app.now += Duration::from_millis(16);
            let now = app.now;
            update(&mut app, Message::Tick(now));
            if app.page_anim.is_animating(now) {
                ticks += 1;
            }
        }
        assert!(
            !app.page_anim.is_animating(app.now),
            "动画应在 450ms 后结束，但 1s 后仍在进行"
        );
        assert!(ticks <= 30, "动画滴答帧数异常: {}", ticks);
    }
}
