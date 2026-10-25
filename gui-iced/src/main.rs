//! Obsidian Launcher — iced 前端。
//!
//! 只消费 `obsidian-launcher-core`：数据加载用 Task::perform，
//! 长任务（下载/启动）通过 core 的 CoreEvent 事件总线回流（见 `events` 模块）。
//! 当前实现：侧边栏 + 启动页（版本/档案/Java/下载/启动/终止）。

use std::sync::LazyLock;

use iced::{
    Color, Element, Length, Size, Subscription, Task, Theme,
    futures::{SinkExt, Stream},
    widget::{Space, button, column, container, pick_list, progress_bar, row, rule, scrollable, text, text_input},
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
    CoreEvent(CoreEvent),
}

// ── 更新逻辑 ─────────────────────────────────────────────

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => {
            app.page = page;
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
        other => column![
            text(other.label()).size(24).color(TEXT_PRIMARY),
            text("该页面尚未迁移到 iced 版，敬请期待。")
                .size(14)
                .color(TEXT_MUTED),
        ]
        .spacing(12)
        .into(),
    }
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

// ── 入口 ─────────────────────────────────────────────────

fn main() -> iced::Result {
    iced::application(App::boot, update, view)
        .title(|_app: &App| "Obsidian Launcher".to_string())
        .theme(|_app: &App| app_theme())
        .subscription(subscription)
        .window_size(Size::new(1000.0, 680.0))
        .run()
}
