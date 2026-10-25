//! Obsidian Launcher — iced 前端。
//!
//! 只消费 `obsidian-launcher-core`：数据加载用 Task::perform，
//! 长任务（下载/启动）通过 core 的 CoreEvent 事件总线回流（见 `events` 模块）。
//! 当前实现：侧边栏 + 启动页（版本/档案/Java/下载/启动/终止）。

use std::sync::LazyLock;

use iced::{
    Color, Element, Length, Size, Subscription, Task, Theme,
    futures::{SinkExt, Stream},
    widget::{Space, button, checkbox, column, container, pick_list, progress_bar, row, rule, scrollable, text, text_input},
};
use tokio::sync::{broadcast, mpsc};

use launcher::events::{CoreEvent, GameState};
use launcher::{AppConfig, ProfileStore, UserProfile};

use obsidian_launcher_core as launcher;

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
    News,
    Settings,
    Mods,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Home => "首页",
            Page::Launch => "启动",
            Page::News => "新闻",
            Page::Settings => "设置",
            Page::Mods => "Mods",
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
    // Microsoft 登录（浏览器授权 + 粘贴回调 URL）
    ms_code_input: String,
    ms_logging_in: bool,
    // Mods 页
    mods: Vec<launcher::modmanager::ModInfo>,
    // 设置页危险区二次确认
    confirm_reset: bool,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let config = launcher::config::load().unwrap_or_default();
        let app = Self {
            page: Page::Launch,
            status: format!("根目录: {}", config.root_dir),
            config,
            versions: Vec::new(),
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
            mods: Vec::new(),
            confirm_reset: false,
        };

        let load = Task::batch([
            Task::perform(launcher::download::get_versions(), Message::VersionsLoaded),
            Task::perform(async { launcher::profiles::load() }, Message::ProfilesLoaded),
            Task::perform(async { scan_java_async().await }, Message::JavaLoaded),
        ]);

        (app, load)
    }

    fn filtered_versions(&self) -> Vec<launcher::download::VersionInfo> {
        self.versions
            .iter()
            .filter(|v| match self.version_tab {
                VersionTab::Release => v.kind == "release",
                VersionTab::Snapshot => v.kind == "snapshot",
                VersionTab::Old => v.kind == "old_alpha" || v.kind == "old_beta",
            })
            .cloned()
            .collect()
    }

    /// 版本 tab 切换后，默认选该分类的最新一个
    fn refresh_default_version(&mut self) {
        self.selected_version = self
            .filtered_versions()
            .into_iter()
            .next()
            .or(self.selected_version.take());
    }
}

// ── 消息 ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
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
    // Mods（GhastLoader）
    ModsLoaded(Result<Vec<launcher::modmanager::ModInfo>, String>),
    ToggleMod(String),
    ModToggled(Result<(), String>),
    InstallGhastloader,
    GhastloaderInstalled(Result<String, String>),
    CoreEvent(CoreEvent),
}

// ── 更新逻辑 ─────────────────────────────────────────────

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => {
            app.page = page;
            if page == Page::Mods {
                return load_mods(&app.config);
            }
            Task::none()
        }
        Message::VersionsLoaded(Ok(versions)) => {
            app.versions = versions;
            app.refresh_default_version();
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
            app.refresh_default_version();
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
            let opts = launcher::launch::LaunchOptions {
                version: version.id.clone(),
                game_dir: launcher::config::resolve_game_dir(&app.config),
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
        Message::ModsLoaded(Ok(mods)) => {
            app.mods = mods;
            Task::none()
        }
        Message::ModsLoaded(Err(e)) => {
            app.mods.clear();
            app.status = format!("加载 mod 列表失败: {}", e);
            Task::none()
        }
        Message::ToggleMod(id) => {
            let game_dir = launcher::config::resolve_game_dir(&app.config).to_string_lossy().to_string();
            let enabled = app
                .mods
                .iter()
                .find(|m| m.id == id)
                .map(|m| !m.enabled)
                .unwrap_or(true);
            Task::perform(
                async move { launcher::modmanager::toggle_mod(&game_dir, &id, enabled) },
                Message::ModToggled,
            )
        }
        Message::ModToggled(Ok(())) => load_mods(&app.config),
        Message::ModToggled(Err(e)) => {
            app.status = format!("切换 mod 状态失败: {}", e);
            Task::none()
        }
        Message::InstallGhastloader => {
            let game_dir = launcher::config::resolve_game_dir(&app.config).to_string_lossy().to_string();
            Task::perform(
                async move { launcher::modmanager::install_ghastloader(&game_dir) },
                Message::GhastloaderInstalled,
            )
        }
        Message::GhastloaderInstalled(Ok(result)) => {
            app.status = format!("GhastLoader: {}", result);
            load_mods(&app.config)
        }
        Message::GhastloaderInstalled(Err(e)) => {
            app.status = format!("GhastLoader 安装失败: {}", e);
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
        },
    }
}

/// 拉取 mod 列表（切换到 Mods 页 / 操作完成后刷新）
fn load_mods(config: &AppConfig) -> Task<Message> {
    let game_dir = launcher::config::resolve_game_dir(config).to_string_lossy().to_string();
    Task::perform(async move { launcher::modmanager::list_mods(&game_dir) }, Message::ModsLoaded)
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

fn subscription(_app: &App) -> Subscription<Message> {
    Subscription::run(listen_core_events)
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

const BG_DEEP: Color = Color::from_rgb8(0x0f, 0x0d, 0x1a);
const BG_SIDEBAR: Color = Color::from_rgb8(0x17, 0x14, 0x28);
const BG_CARD: Color = Color::from_rgb8(0x1c, 0x18, 0x34);
const ACCENT: Color = Color::from_rgb8(0x8b, 0x5c, 0xf6);
const TEXT_PRIMARY: Color = Color::from_rgb8(0xf5, 0xf3, 0xff);
const TEXT_MUTED: Color = Color::from_rgb8(0x9d, 0x93, 0xc8);

fn app_theme() -> Theme {
    Theme::custom(
        "Void Crystal",
        iced::theme::Palette {
            background: BG_DEEP,
            text: TEXT_PRIMARY,
            primary: ACCENT,
            success: Color::from_rgb8(0x34, 0xd3, 0x99),
            warning: Color::from_rgb8(0xfb, 0xbf, 0x24),
            danger: Color::from_rgb8(0xef, 0x44, 0x44),
        },
    )
}

fn card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(BG_CARD.into()),
        border: iced::Border {
            color: Color::from_rgba8(0x8b, 0x5c, 0xf6, 0.25),
            width: 1.0,
            radius: 10.0.into(),
        },
        ..Default::default()
    }
}

fn sidebar_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(BG_SIDEBAR.into()),
        ..Default::default()
    }
}

fn view(app: &App) -> Element<'_, Message> {
    let nav = |page: Page| {
        let active = app.page == page;
        let label = if active {
            format!("● {}", page.label())
        } else {
            format!("  {}", page.label())
        };
        let t = text(label).size(15).color(if active {
            TEXT_PRIMARY
        } else {
            TEXT_MUTED
        });
        button(t).on_press(Message::Navigate(page)).padding(6.0)
    };

    let sidebar = container(
        column![
            text("Obsidian").size(20).color(ACCENT),
            text("Launcher").size(12).color(TEXT_MUTED),
            Space::new().height(Length::from(16)),
            nav(Page::Home),
            nav(Page::Launch),
            nav(Page::News),
            nav(Page::Settings),
            nav(Page::Mods),
            Space::new().height(Length::Fill),
            text(game_state_label(app)).size(12).color(TEXT_MUTED),
        ]
        .spacing(4)
        .padding(12),
    )
    .width(Length::from(150))
    .height(Length::Fill)
    .style(sidebar_style);

    let content = scrollable(container(page_view(app)).padding(20));

    container(row![sidebar, content])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t| container::Style {
            background: Some(BG_DEEP.into()),
            ..Default::default()
        })
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
    match app.page {
        Page::Launch => launch_page(app),
        Page::Home => home_page(app),
        Page::Settings => settings_page(app),
        Page::Mods => mods_page(app),
        Page::News => column![
            text("新闻").size(24).color(TEXT_PRIMARY),
            text("该页面尚未迁移到 iced 版，敬请期待。")
                .size(14)
                .color(TEXT_MUTED),
        ]
        .spacing(12)
        .into(),
    }
}

/// Microsoft 登录：系统浏览器授权 + 粘贴回调 URL（webview 无法自动捕获 code）
fn ms_login_section(app: &App) -> Element<'_, Message> {
    let login_btn = {
        let b = button(text("完成登录").color(Color::WHITE)).padding(6.0);
        if app.ms_logging_in || app.ms_code_input.trim().is_empty() {
            b
        } else {
            b.style(button::primary).on_press(Message::MsLoginPressed)
        }
    };

    column![
        row![
            button(text("打开微软登录页").size(13))
                .padding(6.0)
                .on_press(Message::MsAuthPressed),
            text("浏览器完成授权后，把跳转到的 URL 粘贴到下面")
                .size(11)
                .color(TEXT_MUTED),
        ]
        .spacing(8),
        row![
            text_input("粘贴回调 URL（含 code=...）", &app.ms_code_input)
                .on_input(Message::MsCodeChanged)
                .on_submit(Message::MsLoginPressed),
            login_btn,
        ]
        .spacing(8),
    ]
    .spacing(6)
    .into()
}

fn launch_page(app: &App) -> Element<'_, Message> {
    let tab_row = row(
        [VersionTab::Release, VersionTab::Snapshot, VersionTab::Old]
            .iter()
            .map(|tab| {
                let active = app.version_tab == *tab;
                let t = text(tab.label()).size(13).color(if active {
                    TEXT_PRIMARY
                } else {
                    TEXT_MUTED
                });
                let b = button(t).on_press(Message::TabChanged(*tab));
                if active {
                    b.style(button::primary)
                } else {
                    b
                }
                .into()
            })
            .collect::<Vec<_>>(),
    )
    .spacing(8);

    let version_picker = pick_list(
        app.filtered_versions(),
        app.selected_version.as_ref(),
        Message::VersionSelected,
    )
    .placeholder("选择版本");

    let profile_picker = pick_list(
        app.profiles.clone(),
        app.selected_profile.as_ref(),
        Message::ProfileSelected,
    )
    .placeholder("选择档案");

    let new_profile_input = text_input("新建离线档案...", &app.new_profile_name)
        .on_input(Message::NewProfileNameChanged)
        .on_submit(Message::CreateProfile);

    let java_picker = pick_list(
        app.java_list.clone(),
        app.selected_java.as_ref(),
        Message::JavaSelected,
    )
    .placeholder("选择 Java");

    // 下载按钮
    let download_btn = {
        let b = button(text("下载版本").color(Color::WHITE)).padding(8.0);
        if app.downloading || app.launching || app.selected_version.is_none() {
            b
        } else {
            b.style(button::primary).on_press(Message::DownloadPressed)
        }
    };

    // 启动/终止按钮
    let launch_btn = if app.game_running || app.launching {
        button(text("终止游戏").color(Color::WHITE))
            .padding(8.0)
            .style(button::danger)
            .on_press(Message::KillPressed)
    } else {
        let ready = app.selected_version.is_some()
            && app.selected_profile.is_some()
            && app.selected_java.is_some();
        let b = button(text("启动游戏").color(Color::WHITE)).padding(8.0);
        if ready {
            b.style(button::primary).on_press(Message::LaunchPressed)
        } else {
            b
        }
    };

    let progress_section: Element<Message> = match (&app.progress, app.downloading) {
        (Some(p), _) => column![
            progress_bar(0.0..=100.0, p.percent as f32),
            text(format!("{}/{}（{}%）— {}", p.current, p.total, p.percent, p.current_file))
                .size(12)
                .color(TEXT_MUTED),
        ]
        .spacing(6)
        .into(),
        (None, true) => text("准备下载...").size(12).color(TEXT_MUTED).into(),
        (None, false) => Space::new().height(Length::from(1)).into(),
    };

    let selected_summary = row![
        text("档案:").size(13).color(TEXT_MUTED),
        text(
            app.selected_profile
                .as_ref()
                .map(|p| p.name.as_str())
                .unwrap_or("未选择")
        )
        .size(13),
        text("版本:").size(13).color(TEXT_MUTED),
        text(
            app.selected_version
                .as_ref()
                .map(|v| v.id.as_str())
                .unwrap_or("未选择")
        )
        .size(13),
    ]
    .spacing(8);

    column![
        text("启动").size(26).color(TEXT_PRIMARY),
        text(format!("游戏目录: {}", launcher::config::resolve_game_dir(&app.config).display()))
            .size(12)
            .color(TEXT_MUTED),
        column![
            text("版本").size(15).color(TEXT_PRIMARY),
            tab_row,
            version_picker,
        ]
        .spacing(8),
        rule::horizontal(2.0),
        column![
            text("档案").size(15).color(TEXT_PRIMARY),
            row![profile_picker, new_profile_input].spacing(8),
            ms_login_section(app),
        ]
        .spacing(8),
        rule::horizontal(2.0),
        column![
            text("Java 运行时").size(15).color(TEXT_PRIMARY),
            row![java_picker, button(text("重新扫描").size(13)).on_press(Message::RescanJava)]
                .spacing(8),
        ]
        .spacing(8),
        rule::horizontal(2.0),
        row![download_btn, launch_btn].spacing(12),
        progress_section,
        selected_summary,
        text(app.status.as_str()).size(12).color(TEXT_MUTED),
    ]
    .spacing(14)
    .max_width(640)
    .into()
}

// ── 首页 ─────────────────────────────────────────────────

fn info_row<'a>(label: &'a str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(13).color(TEXT_MUTED),
        text(value).size(13),
    ]
    .spacing(8)
    .into()
}

fn home_page(app: &App) -> Element<'_, Message> {    let info = info_row;

    let profile_line = info(
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
    let version_line = info(
        "版本:",
        app.selected_version
            .as_ref()
            .map(|v| v.id.clone())
            .unwrap_or_else(|| "未选择".to_string()),
    );
    let java_line = info(
        "Java:",
        app.selected_java
            .as_ref()
            .map(|j| j.version.clone())
            .unwrap_or_else(|| "未找到".to_string()),
    );
    let state_line = info("游戏:", game_state_label(app).to_string());
    let root_line = info("根目录:", app.config.root_dir.clone());

    let status_card = container(
        column![
            text("当前状态").size(16).color(TEXT_PRIMARY),
            profile_line,
            version_line,
            java_line,
            state_line,
            root_line,
        ]
        .spacing(6),
    )
    .padding(16)
    .style(card_style);

    let launch_btn = if app.game_running || app.launching {
        button(text("终止游戏").color(Color::WHITE))
            .padding(8.0)
            .style(button::danger)
            .on_press(Message::KillPressed)
    } else {
        let b = button(text("启动游戏").color(Color::WHITE)).padding(8.0);
        let ready = app.selected_version.is_some() && app.selected_profile.is_some()
            && app.selected_java.is_some();
        if ready {
            b.style(button::primary).on_press(Message::LaunchPressed)
        } else {
            b
        }
    };

    let download_btn = {
        let b = button(text("下载当前版本").color(Color::WHITE)).padding(8.0);
        if app.downloading || app.selected_version.is_none() {
            b
        } else {
            b.style(button::primary).on_press(Message::DownloadPressed)
        }
    };

    column![
        text("首页").size(26).color(TEXT_PRIMARY),
        status_card,
        row![launch_btn, download_btn,].spacing(12),
        text("提示：版本 / 档案 / Java 在「启动」页选择；目录与隔离选项在「设置」页。")
            .size(12)
            .color(TEXT_MUTED),
        text(app.status.as_str()).size(12).color(TEXT_MUTED),
    ]
    .spacing(14)
    .max_width(640)
    .into()
}

// ── 设置页 ───────────────────────────────────────────────

fn settings_page(app: &App) -> Element<'_, Message> {
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

    const CONCURRENCY: [usize; 6] = [4, 8, 16, 32, 64, 128];
    let concurrency_picker = pick_list(
        CONCURRENCY.to_vec(),
        CONCURRENCY
            .iter()
            .find(|&&c| c == app.config.download_concurrency),
        Message::ConcurrencySelected,
    )
    .placeholder("并发数");

    let reset_btn = if app.confirm_reset {
        button(text("确认重置？（再点一次执行）").color(Color::WHITE))
            .padding(8.0)
            .style(button::danger)
            .on_press(Message::ResetAllPressed)
    } else {
        button(text("重置所有配置").color(Color::WHITE))
            .padding(8.0)
            .style(button::danger)
            .on_press(Message::ResetAllPressed)
    };

    column![
        text("设置").size(26).color(TEXT_PRIMARY),
        column![
            text("根目录").size(15).color(TEXT_PRIMARY),
            text_input("如 /home/xxx/Games/ObsidianLauncher", &app.config.root_dir)
                .on_input(Message::RootDirChanged),
        ]
        .spacing(8),
        column![
            checkbox(app.config.version_isolation)
                .label("版本隔离（每个版本独立目录）")
                .on_toggle(Message::ToggleVersionIsolation),
            checkbox(app.config.saves_isolation)
                .label("存档隔离（实验级，每版本独立 saves 符号链接）")
                .on_toggle(Message::ToggleSavesIsolation),
        ]
        .spacing(8),
        column![
            text("下载并发数").size(15).color(TEXT_PRIMARY),
            concurrency_picker,
        ]
        .spacing(8),
        rule::horizontal(2.0),
        column![
            text("实际目录").size(15).color(TEXT_PRIMARY),
            text(format!("配置文件: {}", config_path)).size(12).color(TEXT_MUTED),
            text(format!("游戏目录: {}", game_dir)).size(12).color(TEXT_MUTED),
            text(format!("存档目录: {}", saves_dir)).size(12).color(TEXT_MUTED),
            text(format!("版本目录: {}", versions_dir)).size(12).color(TEXT_MUTED),
        ]
        .spacing(6),
        rule::horizontal(2.0),
        row![
            button(text("保存设置").color(Color::WHITE))
                .padding(8.0)
                .style(button::primary)
                .on_press(Message::SaveSettings),
            reset_btn,
        ]
        .spacing(12),
        text("保存时会同时记录当前选中的档案与 Java 路径。重置会删除 config.json 与 profiles.json（游戏文件不受影响）。")
            .size(12)
            .color(TEXT_MUTED),
    ]
    .spacing(14)
    .max_width(640)
    .into()
}

// ── Mods 页（GhastLoader） ───────────────────────────────

fn mods_page(app: &App) -> Element<'_, Message> {
    let install_btn = button(text("安装 / 更新 GhastLoader").color(Color::WHITE))
        .padding(8.0)
        .style(button::primary)
        .on_press(Message::InstallGhastloader);

    let list: Element<Message> = if app.mods.is_empty() {
        text("mods/ 目录暂无 mod。将含 mod.json 的 .jar 放入游戏目录的 mods/ 后切回本页即可。")
            .size(13)
            .color(TEXT_MUTED)
            .into()
    } else {
        let rows = app
            .mods
            .iter()
            .map(|m| {
                let toggle_label = if m.enabled { "已启用" } else { "已禁用" };
                container(row![
                    column![
                        text(m.id.clone()).size(14).color(if m.enabled {
                            TEXT_PRIMARY
                        } else {
                            TEXT_MUTED
                        }),
                        text(format!("v{} — {}", m.version, m.entry)).size(11).color(TEXT_MUTED),
                    ]
                    .spacing(2),
                    Space::new().width(Length::Fill),
                    button(text(toggle_label).size(12))
                        .padding(6.0)
                        .on_press(Message::ToggleMod(m.id.clone())),
                ]
                .spacing(8))
                .padding(12)
                .style(card_style)
                .into()
            })
            .collect::<Vec<_>>();

        column(rows).spacing(10).into()
    };

    column![
        text("Mods — GhastLoader").size(26).color(TEXT_PRIMARY),
        row![
            install_btn,
            text("Java Agent 事件总线式 mod 加载器，启动游戏时自动注入。")
                .size(12)
                .color(TEXT_MUTED),
        ]
        .spacing(12),
        list,
    ]
    .spacing(14)
    .max_width(680)
    .into()
}

// ── 入口 ─────────────────────────────────────────────────

fn main() -> iced::Result {
    iced::application(App::boot, update, view)
        .title(|_app: &App| "Obsidian Launcher".to_string())
        .theme(|_app: &App| app_theme())
        .subscription(subscription)
        .window_size(Size::new(1000.0, 680.0))
        .run()
}
