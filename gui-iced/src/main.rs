//! 深板岩启动器（Deepslate Launcher）— iced 前端。
//!
//! 只消费 `deepslate-core`：数据加载用 Task::perform，
//! 长任务（下载/启动）通过 core 的 CoreEvent 事件总线回流（见 `events` 模块）。
//! 页面切换/卡片入场动画基于 iced 内置 Animation + Float 变换。

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use iced::{
    animation::Animation,
    futures::{SinkExt, Stream},
    widget::{
        button, checkbox, container, float, opaque, pick_list, progress_bar, row, rule, scrollable,
        stack, text, text_input, Space,
    },
    Element, Length, Size, Subscription, Task, Vector,
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
    Friends,
    Maps,
    Server,
    Mods,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Page::Home => "首页",
            Page::Launch => "启动",
            Page::Friends => "好友",
            Page::Maps => "世界",
            Page::Server => "服务器",
            Page::Mods => "模组",
            Page::Settings => "设置",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionTab {
    Release,
    Installed,
}

impl VersionTab {
    fn label(self) -> &'static str {
        match self {
            VersionTab::Release => "正式版",
            VersionTab::Installed => "已安装",
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
    friends: launcher::friends::FriendSnapshot,
    friends_loading: bool,
    friend_error: Option<String>,
    new_profile_name: String,
    java_list: Vec<launcher::launch::JavaInfo>,
    selected_java: Option<launcher::launch::JavaInfo>,
    status: String,
    downloading: bool,
    progress: Option<launcher::download::DownloadProgress>,
    launching: bool,
    game_running: bool,
    active_managed: bool,
    enhanced_started_at: Option<Instant>,
    enhanced_startup_duration: Option<Duration>,
    enhanced_phase: Option<(String, Instant)>,
    // 亮/暗主题（持久化在 config.ui_mode: "light"/"dark"）
    dark_mode: bool,
    // 页面入场动画（0 → 1），now 为最近一次时钟，驱动插值
    page_anim: Animation<bool>,
    now: Instant,
    show_launch_account: bool,
    show_launch_maintenance: bool,
    show_launch_advanced: bool,
    show_manage_favorites: bool,
    // Microsoft 登录（浏览器授权 + 粘贴回调 URL）
    ms_code_input: String,
    ms_logging_in: bool,
    ms_auth_session: Option<launcher::auth::AuthSession>,
    // 设置页危险区二次确认
    confirm_reset: bool,
    confirm_empty_trash: bool,
    root_changed: bool,
    // 独立日志窗口子进程的写入端（None = 未启用或窗口已关闭）
    log_writer: Option<mpsc::Sender<String>>,
    // 本地已安装版本（原版 + Fabric 等加载器）
    local_versions: Vec<launcher::versions::LocalVersion>,
    /// 「已安装」tab 的 VersionInfo 投影（只在扫描后重算）
    local_choices: Vec<launcher::download::VersionInfo>,
    /// 全部正式版缓存（普通下载和 Fabric 安装共用）
    releases: Vec<launcher::download::VersionInfo>,
    // Fabric 安装
    fabric_game: Option<launcher::download::VersionInfo>,
    fabric_loaders: Vec<launcher::fabric::LoaderVersion>,
    fabric_loader: Option<launcher::fabric::LoaderVersion>,
    fabric_loading: bool,
    fabric_installing: bool,
    /// 安装完成后待选中的版本 id
    pending_select: Option<String>,
    // 地图库（全局，不按版本拆分）
    saves: Vec<launcher::maps::MapSummary>,
    pending_creations: Vec<launcher::maps::PendingCreation>,
    map_import_path: String,
    pending_map_restore: Option<String>,
    active_map: Option<String>,
    renaming_save: Option<String>,
    save_name_input: String,
    pending_save_delete: Option<String>,
    pending_world_enter: Option<String>,
    map_menu_open: Option<String>,
    // 快速加入服务器
    server_address: String,
    server_profile_name: String,
    server_ping_status: HashMap<String, String>,
    // 本地服务端
    server_running: bool,
    server_ready: bool,
    server_busy: bool,
    server_logs: Vec<String>,
    server_port_input: String,
    pending_server_join: bool,
    // 新建世界
    new_world_name: String,
    new_world_seed: String,
    new_world_flat: bool,
    new_world_cheats: bool,
    new_world_gamemode: String,
    new_world_difficulty: String,
    new_world_keep_inventory: bool,
    new_world_mob_griefing: bool,
    new_world_daylight_cycle: bool,
    new_world_weather_cycle: bool,
    new_world_hostile_mobs: bool,
    new_world_pvp: bool,
    new_world_creating: bool,
    // 高级启动
    window_width_input: String,
    window_height_input: String,
    // EULA：首次启动前的阻塞式授权（主窗口模态，不复用日志窗口）
    eula_accepted: bool,
    eula_sha1: Option<String>,
    eula_checking: bool,
    eula_required: bool,
    eula_text: Option<String>,
    eula_source: Option<String>,
    eula_warning: Option<String>,
    eula_loading: bool,
    eula_error: Option<String>,
    eula_scrolled_to_bottom: bool,
    eula_checked: bool,
    pending_action: Option<PendingAction>,
    // 模组
    mods: Vec<launcher::mods::ModInfo>,
    mods_version: Option<String>,
    pending_mod_delete: Option<String>,
}

#[derive(Debug, Clone)]
enum PendingAction {
    None,
    Launch(Option<launcher::launch::QuickPlayTarget>),
    ServerInstall,
    ServerStart,
    CreateWorld,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let config = launcher::config::load().unwrap_or_default();
        // 兼容旧版本：以前把“同意”只存在 config.json 里；现在以
        // <root>/eula.txt 为唯一凭据，启动时把旧状态迁移成标记文件。
        let mut eula_accepted = launcher::eula::is_accepted(&config);
        if !eula_accepted && config.server_eula_accepted {
            eula_accepted = launcher::eula::accept(&config).is_ok();
        }
        let eula_sha1 = launcher::eula::accepted_sha1(&config);
        let game_dir = launcher::config::resolve_game_dir(&config);
        let server_address = config.recent_servers.first().cloned().unwrap_or_default();
        let server_port_input = config.server_port.to_string();
        let window_width_input = config.window_width.to_string();
        let window_height_input = config.window_height.to_string();
        let now = Instant::now();
        // 启动即播放入场动画
        let mut page_anim = Animation::new(false)
            .duration(Duration::from_millis(400))
            .easing(iced::animation::Easing::EaseOutCubic);
        page_anim.go_mut(true, now);
        let mut app = Self {
            page: Page::Launch,
            status: format!("根目录: {}", config.root_dir),
            dark_mode: config.ui_mode != "light",
            page_anim,
            now,
            show_launch_account: false,
            show_launch_maintenance: false,
            show_launch_advanced: false,
            show_manage_favorites: false,
            config,
            versions: Vec::new(),
            filtered: Vec::new(),
            version_tab: VersionTab::Release,
            selected_version: None,
            profiles: Vec::new(),
            selected_profile: None,
            friends: launcher::friends::FriendSnapshot::default(),
            friends_loading: false,
            friend_error: None,
            new_profile_name: String::new(),
            java_list: Vec::new(),
            selected_java: None,
            downloading: false,
            progress: None,
            launching: false,
            game_running: false,
            active_managed: false,
            enhanced_started_at: None,
            enhanced_startup_duration: None,
            enhanced_phase: None,
            ms_code_input: String::new(),
            ms_logging_in: false,
            ms_auth_session: None,
            confirm_reset: false,
            confirm_empty_trash: false,
            root_changed: false,
            log_writer: None,
            local_versions: Vec::new(),
            local_choices: Vec::new(),
            releases: Vec::new(),
            fabric_game: None,
            fabric_loaders: Vec::new(),
            fabric_loader: None,
            fabric_loading: false,
            fabric_installing: false,
            pending_select: None,
            saves: Vec::new(),
            pending_creations: Vec::new(),
            map_import_path: String::new(),
            pending_map_restore: None,
            active_map: None,
            renaming_save: None,
            save_name_input: String::new(),
            pending_save_delete: None,
            pending_world_enter: None,
            map_menu_open: None,
            server_address,
            server_profile_name: String::new(),
            server_ping_status: HashMap::new(),
            server_running: false,
            server_ready: false,
            server_busy: false,
            server_logs: Vec::new(),
            server_port_input,
            pending_server_join: false,
            new_world_name: "新的世界".to_string(),
            new_world_seed: String::new(),
            new_world_flat: false,
            new_world_cheats: false,
            new_world_gamemode: "survival".to_string(),
            new_world_difficulty: "normal".to_string(),
            new_world_keep_inventory: false,
            new_world_mob_griefing: true,
            new_world_daylight_cycle: true,
            new_world_weather_cycle: true,
            new_world_hostile_mobs: true,
            new_world_pvp: true,
            new_world_creating: false,
            window_width_input,
            window_height_input,
            eula_accepted,
            eula_sha1,
            eula_checking: false,
            eula_required: false,
            eula_text: None,
            eula_source: None,
            eula_warning: None,
            eula_loading: false,
            eula_error: None,
            eula_scrolled_to_bottom: false,
            eula_checked: false,
            pending_action: None,
            mods: Vec::new(),
            mods_version: None,
            pending_mod_delete: None,
        };

        let cleanup_config = app.config.clone();
        let cleanup = Task::perform(
            async move { launcher::worldgen::cleanup_stale(&cleanup_config) },
            Message::StaleCleanupDone,
        );
        let eula_check = if app.eula_accepted {
            app.eula_checking = true;
            let config = app.config.clone();
            Task::perform(
                async move { launcher::eula::check_for_update(&config).await },
                Message::EulaUpdateChecked,
            )
        } else {
            Task::none()
        };
        let load = Task::batch([
            Task::perform(launcher::download::get_versions(), Message::VersionsLoaded),
            Task::perform(
                async { launcher::profiles::load() },
                Message::ProfilesLoaded,
            ),
            Task::perform(async { scan_java_async().await }, Message::JavaLoaded),
            load_local_versions(game_dir),
            cleanup,
            load_saves_task(&mut app),
            eula_check,
        ]);

        (app, load)
    }

    /// 重算当前 tab 的版本缓存：保留仍在列表中的当前选择，否则退回第一个。
    /// 仅在版本列表加载 / tab 切换时调用。
    fn refilter(&mut self) {
        self.filtered = match self.version_tab {
            VersionTab::Installed => self.local_choices.clone(),
            VersionTab::Release => self.releases.clone(),
        };
        // 当前选择仍在列表里就保留，否则退回列表第一个
        let keep = self
            .selected_version
            .as_ref()
            .and_then(|selected| self.filtered.iter().find(|v| v.id == selected.id).cloned());
        self.selected_version = keep.or_else(|| self.filtered.first().cloned());
    }
}

// ── 消息 ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    ToggleLaunchAccount,
    ToggleLaunchMaintenance,
    ToggleLaunchAdvanced,
    ToggleManageFavorites,
    ToggleMapMenu(String),
    /// 动画时钟滴答
    Tick(Instant),
    /// 亮/暗主题切换
    ToggleDarkMode(bool),
    /// 启动游戏时打开独立日志窗口
    ToggleLogWindow(bool),
    ToggleEnhancedExperience(bool),
    VersionsLoaded(Result<Vec<launcher::download::VersionInfo>, String>),
    LocalVersionsLoaded(std::path::PathBuf, Vec<launcher::versions::LocalVersion>),
    ProfilesLoaded(Result<ProfileStore, String>),
    FriendsLoaded(String, Result<launcher::friends::FriendSnapshot, String>),
    FriendSnapshotSaved(String, Result<(), String>),
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
    ConcurrencySelected(usize),
    SaveSettings,
    SettingsSaved(Result<(), String>),
    ResetAllPressed,
    ResetAllDone(Result<(), String>),
    // Fabric 安装
    FabricGameSelected(launcher::download::VersionInfo),
    FabricLoadersLoaded(Result<Vec<launcher::fabric::LoaderVersion>, String>),
    FabricLoaderSelected(launcher::fabric::LoaderVersion),
    FabricInstallPressed,
    FabricInstalled(Result<launcher::fabric::InstallResult, String>),
    RefreshFabricLoaders,
    // 存档管理
    RefreshSaves,
    SavesLoaded(String, Result<Vec<launcher::maps::MapSummary>, String>),
    PendingCreationsLoaded(String, Result<Vec<launcher::maps::PendingCreation>, String>),
    RecoverCreation(String),
    CreationRecovered(Result<String, String>),
    SaveEnter(String),
    SaveRenameStart(String),
    SaveRenameChanged(String),
    SaveRenameCancel,
    SaveRenameConfirm,
    SaveRenameDone(Result<(), String>),
    SaveDuplicate(String),
    SaveDuplicated(Result<String, String>),
    SaveDeletePressed(String),
    SaveDeleted(Result<String, String>),
    MapImportPathChanged(String),
    ImportMap,
    MapImported(Result<String, String>),
    PickMapFile,
    PickMapFolder,
    MapPathPicked(Option<std::path::PathBuf>),
    ExportMap(String),
    ExportMapAs(String),
    MapExported(Result<String, String>),
    MapExportPathDone(Result<Option<String>, String>),
    MapBackup(String),
    MapBackupDone(Result<String, String>),
    MapRestore(String),
    MapRestored(Result<(), String>),
    MapRefreshed(Result<(), String>),
    EmptyTrash,
    TrashEmptied(Result<usize, String>),
    OpenSavesFolder,
    OpenFolderDone(Result<(), String>),
    NewWorldNameChanged(String),
    NewWorldSeedChanged(String),
    NewWorldFlat(bool),
    NewWorldCheats(bool),
    NewWorldGameMode(String),
    NewWorldDifficulty(String),
    NewWorldKeepInventory(bool),
    NewWorldMobGriefing(bool),
    NewWorldDaylightCycle(bool),
    NewWorldWeatherCycle(bool),
    NewWorldHostileMobs(bool),
    NewWorldPvp(bool),
    CreateWorld,
    WorldCreated(Result<String, String>),
    StaleCleanupDone(Result<usize, String>),
    // 快速加入服务器
    ServerAddressChanged(String),
    ServerProfileNameChanged(String),
    SaveServerProfile,
    RemoveServerProfile(String),
    PingServer(String),
    ServerPingFinished(String, Result<launcher::server_ping::ServerPing, String>),
    QuickJoinServer,
    ConfigPersisted(Result<(), String>),
    EnhancedLanguageSelected(String),
    EnhancedGcSelected(String),
    EnhancedMemorySelected(u32),
    EnhancedCompactHeaders(bool),
    // 本地服务端
    ServerPortChanged(String),
    ServerMemorySelected(u32),
    ServerOnlineMode(bool),
    ServerInstall,
    ServerStart,
    ServerStartAndJoin,
    ServerStop,
    ServerJoinNow,
    ServerInstalled(Result<(), String>),
    ServerStarted(Result<(), String>),
    ServerStopped(Result<(), String>),
    OpenEula,
    OpenEulaDone(Result<(), String>),
    EulaLoaded(Result<launcher::eula::EulaDocument, String>),
    EulaUpdateChecked(Result<launcher::eula::EulaUpdate, String>),
    EulaRetry,
    EulaScrolled(bool),
    EulaChecked(bool),
    EulaAccept,
    EulaCancel,
    OpenServerFolder,
    // 品牌设置
    ToggleBrandTitle(bool),
    BrandTextChanged(String),
    ToggleGameDefaults(bool),
    // 高级启动
    ToggleDisableMultiplayer(bool),
    ToggleDisableChat(bool),
    ToggleFullscreen(bool),
    WindowWidthChanged(String),
    WindowHeightChanged(String),
    GraphicsBackendSelected(String),
    // 模组
    RefreshMods,
    ModsLoaded(String, bool, Result<Vec<launcher::mods::ModInfo>, String>),
    ToggleMod(String, bool),
    ModToggled(Result<(), String>),
    ModDeletePressed(String),
    ModDeleted(Result<String, String>),
    EmptyModTrash,
    ModTrashEmptied(Result<usize, String>),
    OpenModsFolder,
    CoreEvent(CoreEvent),
}

// ── 更新逻辑 ─────────────────────────────────────────────

fn update(app: &mut App, message: Message) -> Task<Message> {
    if app.root_changed
        && !matches!(
            &message,
            Message::RootDirChanged(_)
                | Message::SaveSettings
                | Message::SettingsSaved(_)
                | Message::Navigate(_)
                | Message::Tick(_)
                | Message::CoreEvent(_)
        )
    {
        app.status = "根目录已更改，请保存设置并重新启动".to_string();
        return Task::none();
    }
    match message {
        Message::Navigate(page) => {
            app.page = page;
            app.confirm_reset = false;
            app.confirm_empty_trash = false;
            app.renaming_save = None;
            app.pending_save_delete = None;
            app.pending_world_enter = None;
            app.pending_map_restore = None;
            app.map_menu_open = None;
            // 重播页面入场动画
            app.now = Instant::now();
            app.page_anim = Animation::new(false)
                .duration(Duration::from_millis(300))
                .easing(iced::animation::Easing::EaseOutCubic);
            app.page_anim.go_mut(true, app.now);
            if page == Page::Maps {
                load_saves_task(app)
            } else if page == Page::Mods {
                load_mods_task(app)
            } else if page == Page::Friends {
                load_friends_task(app)
            } else {
                Task::none()
            }
        }
        Message::ToggleLaunchAccount => {
            app.show_launch_account = !app.show_launch_account;
            Task::none()
        }
        Message::ToggleLaunchMaintenance => {
            app.show_launch_maintenance = !app.show_launch_maintenance;
            Task::none()
        }
        Message::ToggleLaunchAdvanced => {
            app.show_launch_advanced = !app.show_launch_advanced;
            Task::none()
        }
        Message::ToggleManageFavorites => {
            app.show_manage_favorites = !app.show_manage_favorites;
            Task::none()
        }
        Message::ToggleMapMenu(id) => {
            app.map_menu_open = if app.map_menu_open.as_deref() == Some(id.as_str()) {
                None
            } else {
                Some(id)
            };
            Task::none()
        }
        Message::Tick(t) => {
            app.now = t;
            Task::none()
        }
        Message::ToggleDarkMode(dark) => {
            app.dark_mode = dark;
            app.config.ui_mode = if dark {
                "dark".to_string()
            } else {
                "light".to_string()
            };
            let config = app.config.clone();
            Task::perform(
                async move { launcher::config::save(&config) },
                Message::SettingsSaved,
            )
        }
        Message::ToggleLogWindow(on) => {
            app.config.log_window = on;
            let config = app.config.clone();
            Task::perform(
                async move { launcher::config::save(&config) },
                Message::SettingsSaved,
            )
        }
        Message::ToggleEnhancedExperience(on) => {
            app.config.enhanced_experience = on;
            if !on {
                app.version_tab = VersionTab::Release;
                app.refilter();
            }
            app.mods.clear();
            app.mods_version = None;
            app.pending_mod_delete = None;
            app.status = "增强体验设置将在下次游戏会话生效".into();
            Task::none()
        }
        Message::VersionsLoaded(Ok(versions)) => {
            app.versions = versions;
            // 普通模式允许安装版本清单里的任一正式版；快照仍不加入稳定版列表。
            app.releases = app
                .versions
                .iter()
                .filter(|v| v.kind == "release")
                .cloned()
                .collect();
            app.refilter();
            // Fabric 卡片默认选最新正式版，并拉取该版本可用的加载器
            if app.fabric_game.is_none() {
                app.fabric_game = app.releases.first().cloned();
            }
            match app.fabric_game.clone() {
                Some(game) if app.fabric_loaders.is_empty() => {
                    app.fabric_loading = true;
                    Task::perform(
                        launcher::fabric::list_loaders(game.id),
                        Message::FabricLoadersLoaded,
                    )
                }
                _ => Task::none(),
            }
        }
        Message::VersionsLoaded(Err(e)) => {
            app.status = format!("加载版本列表失败: {}", e);
            Task::none()
        }
        Message::LocalVersionsLoaded(dir, _)
            if dir != launcher::config::resolve_game_dir(&app.config) =>
        {
            Task::none()
        }
        Message::LocalVersionsLoaded(_, local) => {
            app.local_versions = local;
            app.local_choices = local_choices(&app.local_versions);
            app.refilter();
            // 安装完成后自动选中新版本
            if let Some(id) = app.pending_select.take() {
                if let Some(choice) = app.local_choices.iter().find(|v| v.id == id).cloned() {
                    app.selected_version = Some(choice);
                }
            }
            Task::none()
        }
        Message::ProfilesLoaded(Ok(store)) => {
            app.profiles = store.profiles.clone();
            app.selected_profile = app
                .config
                .last_profile
                .as_ref()
                .and_then(|name| app.profiles.iter().find(|p| &p.name == name))
                .or_else(|| {
                    store
                        .last_used
                        .as_ref()
                        .and_then(|name| app.profiles.iter().find(|p| &p.name == name))
                })
                .cloned()
                .or_else(|| app.profiles.first().cloned());
            app.friends = launcher::friends::FriendSnapshot::default();
            load_friends_task(app)
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
            app.pending_world_enter = None;
            app.selected_version = Some(v);
            Task::none()
        }
        Message::ProfileSelected(p) => {
            app.selected_profile = Some(p);
            app.friends = launcher::friends::FriendSnapshot::default();
            load_friends_task(app)
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
            Task::perform(
                async move { launcher::profiles::create(name) },
                Message::ProfileCreated,
            )
        }
        Message::ProfileCreated(Ok(profile)) => {
            app.status = format!("已创建档案: {}", profile.name);
            app.new_profile_name.clear();
            app.profiles.push(profile.clone());
            app.selected_profile = Some(profile);
            app.friends = launcher::friends::FriendSnapshot::default();
            load_friends_task(app)
        }
        Message::ProfileCreated(Err(e)) => {
            app.status = format!("创建档案失败: {}", e);
            Task::none()
        }
        Message::FriendsLoaded(account_id, Ok(snapshot)) => {
            if app
                .selected_profile
                .as_ref()
                .is_some_and(|profile| profile.uuid == account_id)
                && snapshot.updated_at >= app.friends.updated_at
            {
                app.friends = snapshot;
                app.friends_loading = false;
                app.friend_error = None;
            }
            Task::none()
        }
        Message::FriendsLoaded(account_id, Err(error)) => {
            if app
                .selected_profile
                .as_ref()
                .is_some_and(|profile| profile.uuid == account_id)
                && app.friends.updated_at == 0
            {
                app.friends_loading = false;
                app.friends = launcher::friends::FriendSnapshot::default();
                app.friend_error = Some(error.clone());
                app.status = format!("加载好友列表失败: {error}");
            }
            Task::none()
        }
        Message::FriendSnapshotSaved(account_id, Err(error)) => {
            if app
                .selected_profile
                .as_ref()
                .is_some_and(|profile| profile.uuid == account_id)
            {
                app.status = format!("保存好友快照失败: {error}");
            }
            Task::none()
        }
        Message::FriendSnapshotSaved(_, Ok(())) => Task::none(),
        Message::RescanJava => {
            Task::perform(async { scan_java_async().await }, Message::JavaLoaded)
        }
        Message::DownloadPressed => {
            if app.downloading || app.fabric_installing || app.launching || app.game_running {
                app.status = "已有下载、安装或游戏任务正在进行".to_string();
                return Task::none();
            }
            let Some(version) = app.selected_version.clone() else {
                app.status = "请先选择一个版本".to_string();
                return Task::none();
            };
            if version.local {
                app.status = "该版本已安装，可直接启动".to_string();
                return Task::none();
            }
            let game_dir = launcher::config::resolve_game_dir(&app.config);
            let concurrency = app.config.download_concurrency;
            let events = core_event_sender();

            app.downloading = true;
            app.progress = None;
            app.status = format!("正在下载 {} ...", version.id);
            Task::perform(
                async move {
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
            let prefetch = if !app.eula_accepted {
                let config = app.config.clone();
                app.eula_loading = true;
                app.eula_error = None;
                Task::perform(
                    async move { launcher::eula::load_or_fetch(&config).await },
                    Message::EulaLoaded,
                )
            } else {
                Task::none()
            };
            Task::batch([
                load_local_versions(launcher::config::resolve_game_dir(&app.config)),
                prefetch,
            ])
        }
        Message::DownloadFinished(Err(e)) => {
            app.downloading = false;
            app.status = format!("下载失败: {}", e);
            Task::none()
        }
        Message::LaunchPressed => start_launch(app, None),
        Message::LaunchFinished(Ok(())) => {
            app.launching = false;
            app.game_running = false;
            app.active_managed = false;
            app.status = "游戏已退出".to_string();
            if let Some(id) = app.active_map.take() {
                let config = app.config.clone();
                Task::perform(
                    async move { launcher::maps::refresh_after_play(&config, &id).map(|_| ()) },
                    Message::MapRefreshed,
                )
            } else {
                Task::none()
            }
        }
        Message::LaunchFinished(Err(e)) => {
            app.launching = false;
            app.game_running = false;
            app.active_managed = false;
            app.active_map = None;
            app.status = format!("启动失败: {}", e);
            load_saves_task(app)
        }
        Message::KillPressed => {
            if app.active_managed {
                app.status = "正在请求游戏保存并退出...".to_string();
                Task::perform(launcher::launch::quit_managed_game(), Message::KillFinished)
            } else {
                app.status = "正在终止游戏...".to_string();
                Task::perform(launcher::launch::kill_game(), Message::KillFinished)
            }
        }
        Message::KillFinished(Ok(())) => {
            app.status = "已请求终止游戏，等待进程退出...".to_string();
            Task::none()
        }
        Message::KillFinished(Err(e)) => {
            app.status = format!("终止失败: {}", e);
            Task::none()
        }
        Message::MsAuthPressed => {
            let session = match launcher::auth::AuthSession::new() {
                Ok(session) => session,
                Err(error) => {
                    app.status = error;
                    return Task::none();
                }
            };
            app.status = "已打开浏览器，请在浏览器中完成 Microsoft 授权...".to_string();
            let url = session.authorization_url();
            app.ms_auth_session = Some(session);
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
            let Some(session) = app.ms_auth_session.take() else {
                app.status = "请先打开微软登录页".to_string();
                return Task::none();
            };
            if let Err(error) = session.callback_code(&app.ms_code_input) {
                app.ms_auth_session = Some(session);
                app.status = error;
                return Task::none();
            }
            let callback = app.ms_code_input.clone();
            let events = core_event_sender();
            app.ms_logging_in = true;
            app.status = "正在完成 Microsoft 登录...".to_string();
            Task::perform(
                async move {
                    let online = session.login(&callback, &events).await?;
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
            app.friends = launcher::friends::FriendSnapshot::default();
            app.status = format!("Microsoft 登录成功: {}", profile.name);
            load_friends_task(app)
        }
        Message::MsLoginFinished(Err(e)) => {
            app.ms_logging_in = false;
            app.status = format!("登录失败: {}", e);
            Task::none()
        }
        Message::RootDirChanged(dir) => {
            if app.config.root_dir == dir {
                return Task::none();
            }
            if app.game_running
                || app.launching
                || app.downloading
                || app.fabric_installing
                || app.server_running
                || app.server_busy
                || app.new_world_creating
                || app.ms_logging_in
            {
                app.status = "请先停止当前任务，再修改根目录".to_string();
                return Task::none();
            }
            app.config.root_dir = dir;
            app.root_changed = true;
            app.local_versions.clear();
            app.local_choices.clear();
            app.selected_version = None;
            app.saves.clear();
            app.pending_creations.clear();
            app.mods.clear();
            app.mods_version = None;
            app.profiles.clear();
            app.selected_profile = None;
            app.friends = launcher::friends::FriendSnapshot::default();
            app.friends_loading = false;
            app.friend_error = None;
            app.pending_select = None;
            app.refilter();
            app.eula_accepted = launcher::eula::is_accepted(&app.config);
            app.eula_sha1 = launcher::eula::accepted_sha1(&app.config);
            app.eula_checking = false;
            app.eula_text = None;
            app.eula_source = None;
            app.eula_warning = None;
            app.eula_error = None;
            app.status = "根目录已变更；请保存设置并重新启动以重新加载档案".to_string();
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
            app.config = config.clone();
            Task::perform(
                async move { launcher::config::save(&config) },
                Message::SettingsSaved,
            )
        }
        Message::SettingsSaved(Ok(())) => {
            app.status = if app.root_changed {
                "设置已保存，请重新启动"
            } else {
                "设置已保存"
            }
            .to_string();
            Task::none()
        }
        Message::SettingsSaved(Err(e)) => {
            app.status = format!("保存设置失败: {}", e);
            Task::none()
        }
        Message::ResetAllPressed => {
            if app.confirm_reset {
                app.confirm_reset = false;
                Task::perform(
                    async { launcher::config::reset_all() },
                    Message::ResetAllDone,
                )
            } else {
                app.confirm_reset = true;
                app.status = "再次点击红色按钮才会真正重置".to_string();
                Task::none()
            }
        }
        Message::ResetAllDone(Ok(())) => {
            app.config = AppConfig::default();
            app.eula_accepted = launcher::eula::is_accepted(&app.config);
            app.eula_sha1 = launcher::eula::accepted_sha1(&app.config);
            app.eula_checking = false;
            app.eula_text = None;
            app.eula_source = None;
            app.eula_warning = None;
            app.eula_error = None;
            app.profiles.clear();
            app.selected_profile = None;
            app.friends = launcher::friends::FriendSnapshot::default();
            app.friends_loading = false;
            app.friend_error = None;
            app.status = "所有配置已重置".to_string();
            Task::perform(
                async { launcher::profiles::load() },
                Message::ProfilesLoaded,
            )
        }
        Message::ResetAllDone(Err(e)) => {
            app.status = format!("重置失败: {}", e);
            Task::none()
        }
        Message::FabricGameSelected(game) => {
            app.fabric_game = Some(game.clone());
            app.fabric_loaders.clear();
            app.fabric_loader = None;
            app.fabric_loading = true;
            app.status = format!("正在获取 {} 可用的 Fabric 加载器...", game.id);
            Task::perform(
                launcher::fabric::list_loaders(game.id),
                Message::FabricLoadersLoaded,
            )
        }
        Message::RefreshFabricLoaders => match app.fabric_game.clone() {
            Some(game) => {
                app.fabric_loading = true;
                Task::perform(
                    launcher::fabric::list_loaders(game.id),
                    Message::FabricLoadersLoaded,
                )
            }
            None => {
                app.status = "请先选择基准游戏版本".to_string();
                Task::none()
            }
        },
        Message::FabricLoadersLoaded(Ok(loaders)) => {
            app.fabric_loading = false;
            app.fabric_loaders = loaders;
            // 保留原选择（若仍存在），否则选第一个稳定版
            let current = app.fabric_loader.take().map(|l| l.version);
            app.fabric_loader = current
                .and_then(|v| app.fabric_loaders.iter().find(|l| l.version == v).cloned())
                .or_else(|| launcher::fabric::default_loader(&app.fabric_loaders));
            app.status = format!("已获取 {} 个 Fabric 加载器版本", app.fabric_loaders.len());
            Task::none()
        }
        Message::FabricLoadersLoaded(Err(e)) => {
            app.fabric_loading = false;
            app.fabric_loaders.clear();
            app.fabric_loader = None;
            app.status = format!("获取 Fabric 加载器列表失败: {}", e);
            Task::none()
        }
        Message::FabricLoaderSelected(loader) => {
            app.fabric_loader = Some(loader);
            Task::none()
        }
        Message::FabricInstallPressed => {
            if app.fabric_installing || app.downloading || app.game_running || app.launching {
                return Task::none();
            }
            let Some(game) = app.fabric_game.clone() else {
                app.status = "请先选择基准游戏版本".to_string();
                return Task::none();
            };
            let Some(loader) = app.fabric_loader.clone() else {
                app.status = "请先选择 Fabric 加载器版本".to_string();
                return Task::none();
            };

            let version_id = launcher::fabric::version_id(&game.id, &loader.version);
            let opts = launcher::fabric::InstallOptions {
                game_version: game.id.clone(),
                loader_version: loader.version.clone(),
                game_dir: launcher::config::resolve_game_dir(&app.config),
                instance_dir: launcher::config::get_version_root(&app.config, &version_id),
                concurrency: app.config.download_concurrency,
            };
            let events = core_event_sender();

            app.fabric_installing = true;
            app.progress = None;
            app.status = format!(
                "正在安装 Fabric {} / Minecraft {} ...",
                loader.version, game.id
            );
            Task::perform(
                async move { launcher::fabric::install(opts, events).await },
                Message::FabricInstalled,
            )
        }
        Message::FabricInstalled(Ok(result)) => {
            app.fabric_installing = false;
            app.pending_select = Some(result.version_id.clone());
            app.version_tab = VersionTab::Installed;
            app.status = format!(
                "Fabric {} / Minecraft {} 安装完成（{} 个库）",
                result.loader_version, result.game_version, result.libraries_total
            );
            load_local_versions(launcher::config::resolve_game_dir(&app.config))
        }
        Message::FabricInstalled(Err(e)) => {
            app.fabric_installing = false;
            app.status = format!("Fabric 安装失败: {}", e);
            Task::none()
        }
        Message::RefreshSaves => load_saves_task(app),
        Message::SavesLoaded(root, _) if root != app.config.root_dir => Task::none(),
        Message::SavesLoaded(_, Ok(worlds)) => {
            app.saves = worlds;
            Task::none()
        }
        Message::SavesLoaded(_, Err(e)) => {
            app.status = format!("加载存档失败: {}", e);
            Task::none()
        }
        Message::PendingCreationsLoaded(root, _) if root != app.config.root_dir => Task::none(),
        Message::PendingCreationsLoaded(_, Ok(pending)) => {
            app.pending_creations = pending;
            Task::none()
        }
        Message::PendingCreationsLoaded(_, Err(e)) => {
            app.status = format!("读取待恢复世界失败: {e}");
            Task::none()
        }
        Message::RecoverCreation(id) => {
            if app.game_running || app.launching {
                app.status = "请先退出游戏再恢复世界".into();
                return Task::none();
            }
            let config = app.config.clone();
            app.status = "正在验证待恢复世界...".into();
            Task::perform(
                async move {
                    launcher::maps::recover_client_creation(&config, &id)
                        .map(|map| map.name.clone())
                },
                Message::CreationRecovered,
            )
        }
        Message::CreationRecovered(Ok(name)) => {
            app.status = format!("已恢复世界：{name}");
            load_saves_task(app)
        }
        Message::CreationRecovered(Err(e)) => {
            app.status = format!("恢复失败，原有数据已保留: {e}");
            load_saves_task(app)
        }
        Message::SaveEnter(id) => {
            if app.pending_world_enter.as_deref() != Some(id.as_str()) {
                app.pending_world_enter = Some(id);
                app.status =
                    "请确认当前游戏版本与地图兼容，再次点击进入；启动前会自动备份".to_string();
                return Task::none();
            }
            app.pending_world_enter = None;
            app.status = format!("正在进入存档 {id} ...");
            start_launch(
                app,
                Some(launcher::launch::QuickPlayTarget::Singleplayer(id)),
            )
        }
        Message::SaveRenameStart(id) => {
            let old = app
                .saves
                .iter()
                .find(|w| w.id == id)
                .map(|w| w.name.clone())
                .unwrap_or_else(|| id.clone());
            app.renaming_save = Some(id);
            app.save_name_input = old;
            app.pending_save_delete = None;
            Task::none()
        }
        Message::SaveRenameChanged(name) => {
            app.save_name_input = name;
            Task::none()
        }
        Message::SaveRenameCancel => {
            app.renaming_save = None;
            app.save_name_input.clear();
            Task::none()
        }
        Message::SaveRenameConfirm => {
            if app.game_running || app.launching {
                app.status = "请先退出游戏再修改地图".to_string();
                return Task::none();
            }
            let Some(id) = app.renaming_save.clone() else {
                return Task::none();
            };
            let name = app.save_name_input.trim().to_string();
            if name.is_empty() {
                app.status = "世界名称不能为空".to_string();
                return Task::none();
            }
            let config = app.config.clone();
            app.status = "正在重命名存档...".to_string();
            Task::perform(
                async move { launcher::maps::rename(&config, &id, &name) },
                Message::SaveRenameDone,
            )
        }
        Message::SaveRenameDone(Ok(())) => {
            app.renaming_save = None;
            app.save_name_input.clear();
            app.status = "存档已重命名".to_string();
            load_saves_task(app)
        }
        Message::SaveRenameDone(Err(e)) => {
            app.status = format!("重命名失败: {}", e);
            Task::none()
        }
        Message::SaveDuplicate(id) => {
            if app.game_running || app.launching {
                app.status = "请先退出游戏再复制地图".to_string();
                return Task::none();
            }
            let config = app.config.clone();
            app.status = "正在复制存档...".to_string();
            Task::perform(
                async move { launcher::maps::duplicate(&config, &id, None) },
                Message::SaveDuplicated,
            )
        }
        Message::SaveDuplicated(Ok(new_id)) => {
            app.status = format!("已复制存档: {new_id}");
            load_saves_task(app)
        }
        Message::SaveDuplicated(Err(e)) => {
            app.status = format!("复制存档失败: {}", e);
            Task::none()
        }
        Message::SaveDeletePressed(id) => {
            if app.game_running || app.launching {
                app.status = "请先退出游戏再删除地图".to_string();
                return Task::none();
            }
            if app.pending_save_delete.as_deref() == Some(id.as_str()) {
                app.pending_save_delete = None;
                let config = app.config.clone();
                app.status = "正在移入回收站...".to_string();
                Task::perform(
                    async move {
                        launcher::maps::move_to_trash(&config, &id).map(|p| p.display().to_string())
                    },
                    Message::SaveDeleted,
                )
            } else {
                app.pending_save_delete = Some(id);
                app.renaming_save = None;
                app.status = "删除会移动到 maps/.deepslate-trash/，再点一次确认".to_string();
                Task::none()
            }
        }
        Message::SaveDeleted(Ok(path)) => {
            app.status = format!("已移入回收站: {path}");
            load_saves_task(app)
        }
        Message::SaveDeleted(Err(e)) => {
            app.status = format!("删除存档失败: {}", e);
            Task::none()
        }
        Message::MapImportPathChanged(path) => {
            app.map_import_path = path;
            Task::none()
        }
        Message::ImportMap => {
            let path = app.map_import_path.trim().to_string();
            if path.is_empty() {
                app.status = "请输入地图 zip 或世界文件夹路径".to_string();
                return Task::none();
            }
            app.status = "正在导入地图...".to_string();
            import_map_task(app.config.clone(), std::path::PathBuf::from(path))
        }
        Message::MapImported(Ok(name)) => {
            app.map_import_path.clear();
            app.status = format!("地图已导入: {name}");
            load_saves_task(app)
        }
        Message::MapImported(Err(e)) => {
            app.status = format!("导入地图失败: {e}");
            Task::none()
        }
        Message::PickMapFile => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .set_title("导入地图 zip")
                    .add_filter("Minecraft 地图", &["zip"])
                    .pick_file()
                    .await
                    .map(|f| f.path().to_path_buf())
            },
            Message::MapPathPicked,
        ),
        Message::PickMapFolder => Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .set_title("导入世界文件夹")
                    .pick_folder()
                    .await
                    .map(|f| f.path().to_path_buf())
            },
            Message::MapPathPicked,
        ),
        Message::MapPathPicked(Some(path)) => {
            app.map_import_path = path.display().to_string();
            app.status = "正在导入地图...".to_string();
            import_map_task(app.config.clone(), path)
        }
        Message::MapPathPicked(None) => {
            app.status = "已取消选择".to_string();
            Task::none()
        }
        Message::ExportMap(id) => {
            if app.game_running || app.launching {
                app.status = "请先退出游戏再导出地图".to_string();
                return Task::none();
            }
            let config = app.config.clone();
            app.status = "正在导出地图...".to_string();
            Task::perform(
                async move {
                    launcher::maps::export_zip(&config, &id, None).map(|p| p.display().to_string())
                },
                Message::MapExported,
            )
        }
        Message::ExportMapAs(id) if app.game_running || app.launching => {
            let _ = id;
            app.status = "请先退出游戏再导出地图".to_string();
            Task::none()
        }
        Message::ExportMapAs(id) => Task::perform(
            {
                let config = app.config.clone();
                async move {
                    let Some(path) = rfd::AsyncFileDialog::new()
                        .set_title("导出地图")
                        .set_file_name(format!("{id}.zip"))
                        .add_filter("Minecraft 地图", &["zip"])
                        .save_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                    else {
                        return Ok(None);
                    };
                    launcher::maps::export_zip(&config, &id, Some(&path))
                        .map(|p| Some(p.display().to_string()))
                }
            },
            Message::MapExportPathDone,
        ),
        Message::MapExported(Ok(path)) => {
            app.status = format!("地图已导出: {path}");
            Task::none()
        }
        Message::MapExported(Err(e)) => {
            app.status = format!("导出地图失败: {e}");
            Task::none()
        }
        Message::MapExportPathDone(Ok(Some(path))) => {
            app.status = format!("地图已导出: {path}");
            Task::none()
        }
        Message::MapExportPathDone(Ok(None)) => {
            app.status = "已取消导出".to_string();
            Task::none()
        }
        Message::MapExportPathDone(Err(e)) => {
            app.status = format!("导出地图失败: {e}");
            Task::none()
        }
        Message::MapBackup(id) => {
            if app.game_running || app.launching {
                app.status = "请先退出游戏再备份地图".to_string();
                return Task::none();
            }
            let config = app.config.clone();
            app.status = "正在备份地图...".to_string();
            Task::perform(
                async move { launcher::maps::backup(&config, &id) },
                Message::MapBackupDone,
            )
        }
        Message::MapBackupDone(Ok(name)) => {
            app.status = format!("地图备份已创建: {name}");
            Task::none()
        }
        Message::MapBackupDone(Err(e)) => {
            app.status = format!("备份地图失败: {e}");
            Task::none()
        }
        Message::MapRestore(id) => {
            if app.game_running || app.launching {
                app.status = "请先退出游戏再恢复地图".to_string();
                return Task::none();
            }
            if app.pending_map_restore.as_deref() != Some(id.as_str()) {
                app.pending_map_restore = Some(id);
                app.status = "再次点击「确认恢复？」会用最近一份备份覆盖当前地图".to_string();
                return Task::none();
            }
            app.pending_map_restore = None;
            let config = app.config.clone();
            app.status = "正在恢复地图备份...".to_string();
            Task::perform(
                async move {
                    let backups = launcher::maps::list_backups(&config, &id)?;
                    let name = backups
                        .first()
                        .ok_or_else(|| "没有可用的地图备份".to_string())?;
                    launcher::maps::restore_backup(&config, &id, name)
                },
                Message::MapRestored,
            )
        }
        Message::MapRestored(Ok(())) => {
            app.status = "地图已从最近备份恢复".to_string();
            load_saves_task(app)
        }
        Message::MapRestored(Err(e)) => {
            app.status = format!("恢复地图失败: {e}");
            Task::none()
        }
        Message::MapRefreshed(Ok(())) => load_saves_task(app),
        Message::MapRefreshed(Err(e)) => {
            app.status = format!("刷新地图元数据失败: {e}");
            load_saves_task(app)
        }
        Message::EmptyTrash => {
            if !app.confirm_empty_trash {
                app.confirm_empty_trash = true;
                app.status = "再次点击确认永久清空地图回收站，此操作不可撤销".to_string();
                return Task::none();
            }
            app.confirm_empty_trash = false;
            let config = app.config.clone();
            Task::perform(
                async move { launcher::maps::empty_trash(&config) },
                Message::TrashEmptied,
            )
        }
        Message::TrashEmptied(Ok(count)) => {
            app.status = format!("已清空回收站（{count} 项）");
            Task::none()
        }
        Message::TrashEmptied(Err(e)) => {
            app.status = format!("清空回收站失败: {}", e);
            Task::none()
        }
        Message::OpenSavesFolder => {
            let config = app.config.clone();
            Task::perform(
                async move {
                    let dir = launcher::maps::ensure_root(&config)?;
                    open::that(&dir).map_err(|e| e.to_string())
                },
                Message::OpenFolderDone,
            )
        }
        Message::OpenFolderDone(Ok(())) => Task::none(),
        Message::OpenFolderDone(Err(e)) => {
            app.status = format!("打开目录失败: {}", e);
            Task::none()
        }
        Message::NewWorldNameChanged(name) => {
            app.new_world_name = name;
            Task::none()
        }
        Message::NewWorldSeedChanged(seed) => {
            app.new_world_seed = seed;
            Task::none()
        }
        Message::NewWorldFlat(flat) => {
            app.new_world_flat = flat;
            Task::none()
        }
        Message::NewWorldCheats(cheats) => {
            app.new_world_cheats = cheats;
            Task::none()
        }
        Message::NewWorldGameMode(mode) => {
            app.new_world_gamemode = mode;
            Task::none()
        }
        Message::NewWorldDifficulty(difficulty) => {
            app.new_world_difficulty = difficulty;
            Task::none()
        }
        Message::NewWorldKeepInventory(enabled) => {
            app.new_world_keep_inventory = enabled;
            Task::none()
        }
        Message::NewWorldMobGriefing(enabled) => {
            app.new_world_mob_griefing = enabled;
            Task::none()
        }
        Message::NewWorldDaylightCycle(enabled) => {
            app.new_world_daylight_cycle = enabled;
            Task::none()
        }
        Message::NewWorldWeatherCycle(enabled) => {
            app.new_world_weather_cycle = enabled;
            Task::none()
        }
        Message::NewWorldHostileMobs(enabled) => {
            app.new_world_hostile_mobs = enabled;
            Task::none()
        }
        Message::NewWorldPvp(enabled) => {
            app.new_world_pvp = enabled;
            Task::none()
        }
        Message::CreateWorld => create_world_action(app),
        Message::WorldCreated(Ok(id)) => {
            app.new_world_creating = false;
            app.status = format!("已创建世界: {id}");
            load_saves_task(app)
        }
        Message::WorldCreated(Err(e)) => {
            app.new_world_creating = false;
            app.status = format!("创建世界失败: {}", e);
            Task::none()
        }
        Message::StaleCleanupDone(Ok(count)) if count > 0 => {
            app.status = format!("已清理 {count} 个上次未完成的生成任务");
            load_saves_task(app)
        }
        Message::StaleCleanupDone(Ok(_)) => Task::none(),
        Message::StaleCleanupDone(Err(e)) => {
            app.status = format!("清理陈旧生成任务失败: {e}");
            Task::none()
        }
        Message::ServerAddressChanged(address) => {
            app.server_address = address;
            Task::none()
        }
        Message::ServerProfileNameChanged(name) => {
            app.server_profile_name = name;
            Task::none()
        }
        Message::SaveServerProfile => {
            let address = app.server_address.trim().to_string();
            if address.is_empty()
                || address.len() > 255
                || address.chars().any(char::is_control)
                || address.chars().any(char::is_whitespace)
            {
                app.status = "请输入有效服务器地址后再保存".into();
                return Task::none();
            }
            let name = app.server_profile_name.trim();
            if name.chars().count() > 48 || name.chars().any(char::is_control) {
                app.status = "服务器名称最多 48 个字符".into();
                return Task::none();
            }
            let name = if name.is_empty() {
                address.clone()
            } else {
                name.to_string()
            };
            app.config
                .saved_servers
                .retain(|item| item.address != address);
            app.config
                .saved_servers
                .insert(0, launcher::config::SavedServer { name, address });
            app.config.saved_servers.truncate(32);
            app.server_profile_name.clear();
            persist_config_task(app)
        }
        Message::RemoveServerProfile(address) => {
            app.config
                .saved_servers
                .retain(|item| item.address != address);
            app.server_ping_status.remove(&address);
            persist_config_task(app)
        }
        Message::PingServer(address) => {
            app.server_ping_status
                .insert(address.clone(), "正在查询服务器状态…".into());
            let result_address = address.clone();
            Task::perform(
                async move { launcher::server_ping::ping(&address).await },
                move |result| Message::ServerPingFinished(result_address, result),
            )
        }
        Message::ServerPingFinished(address, result) => {
            let status = match result {
                Ok(ping) => {
                    let mut parts = vec![format!("在线 · {} ms", ping.latency_ms)];
                    if let (Some(online), Some(max)) = (ping.online, ping.max) {
                        parts.push(format!("{online}/{max} 人"));
                    }
                    if let Some(version) = ping.version {
                        parts.push(version);
                    }
                    if !ping.motd.is_empty() {
                        parts.push(ping.motd.chars().take(80).collect());
                    }
                    parts.join(" · ")
                }
                Err(error) => format!("暂不可达 · {}", error.chars().take(64).collect::<String>()),
            };
            app.server_ping_status.insert(address, status);
            Task::none()
        }
        Message::QuickJoinServer => {
            let address = app.server_address.trim().to_string();
            if address.is_empty() {
                app.status = "请输入服务器地址（host 或 host:port）".to_string();
                return Task::none();
            }
            app.config.recent_servers.retain(|s| s != &address);
            app.config.recent_servers.insert(0, address.clone());
            app.config.recent_servers.truncate(8);
            let config = app.config.clone();
            let persist = Task::perform(
                async move { launcher::config::save(&config) },
                Message::ConfigPersisted,
            );
            app.status = format!("正在加入服务器 {address} ...");
            Task::batch([
                persist,
                start_launch(
                    app,
                    Some(launcher::launch::QuickPlayTarget::Multiplayer(address)),
                ),
            ])
        }
        Message::ConfigPersisted(Ok(())) => Task::none(),
        Message::ConfigPersisted(Err(e)) => {
            app.status = format!("保存配置失败: {}", e);
            Task::none()
        }
        Message::EnhancedLanguageSelected(language) => {
            app.config.enhanced_language = language;
            Task::none()
        }
        Message::EnhancedGcSelected(gc) => {
            app.config.enhanced_gc = gc;
            Task::none()
        }
        Message::EnhancedMemorySelected(memory) => {
            app.config.enhanced_max_memory_mb = memory;
            Task::none()
        }
        Message::EnhancedCompactHeaders(enabled) => {
            app.config.enhanced_compact_object_headers = enabled;
            Task::none()
        }
        Message::ServerPortChanged(text) => {
            app.server_port_input = text;
            Task::none()
        }
        Message::ServerMemorySelected(mb) => {
            app.config.server_memory_mb = mb;
            persist_config_task(app)
        }
        Message::ServerOnlineMode(on) => {
            app.config.server_online_mode = on;
            persist_config_task(app)
        }
        Message::ServerInstall => server_action(app, true),
        Message::ServerStart => {
            app.pending_server_join = false;
            server_action(app, false)
        }
        Message::ServerStartAndJoin => {
            app.pending_server_join = true;
            server_action(app, false)
        }
        Message::ServerJoinNow => {
            let address = format!("127.0.0.1:{}", app.config.server_port);
            app.status = format!("正在加入服务器 {address} ...");
            start_launch(
                app,
                Some(launcher::launch::QuickPlayTarget::Multiplayer(address)),
            )
        }
        Message::ServerStop => {
            if app.server_busy {
                return Task::none();
            }
            app.server_busy = true;
            app.status = "正在停止服务端...".to_string();
            Task::perform(launcher::server::stop(), Message::ServerStopped)
        }
        Message::ServerInstalled(Ok(())) => {
            app.server_busy = false;
            app.status = "服务端已安装/校验完成".to_string();
            Task::none()
        }
        Message::ServerInstalled(Err(e)) => {
            app.server_busy = false;
            app.status = format!("服务端安装失败: {}", e);
            Task::none()
        }
        Message::ServerStarted(Ok(())) => {
            app.server_busy = false;
            app.server_running = true;
            app.status = "服务端启动中...".to_string();
            Task::none()
        }
        Message::ServerStarted(Err(e)) => {
            app.server_busy = false;
            app.server_running = false;
            app.pending_server_join = false;
            app.status = format!("服务端启动失败: {}", e);
            Task::none()
        }
        Message::ServerStopped(Ok(())) => {
            app.server_busy = false;
            Task::none()
        }
        Message::ServerStopped(Err(e)) => {
            app.server_busy = false;
            app.status = format!("停止服务端失败: {}", e);
            Task::none()
        }
        Message::OpenEula => Task::perform(
            async move { open::that("https://aka.ms/MinecraftEULA").map_err(|e| e.to_string()) },
            Message::OpenEulaDone,
        ),
        Message::OpenEulaDone(Ok(())) => Task::none(),
        Message::OpenEulaDone(Err(e)) => {
            app.status = format!("打开 EULA 失败: {}", e);
            Task::none()
        }
        Message::EulaLoaded(Ok(doc)) => {
            let first_load = app.eula_text.is_none();
            app.eula_loading = false;
            app.eula_error = None;
            app.eula_source = Some(doc.summary());
            app.eula_sha1 = Some(doc.sha1.clone());
            app.eula_warning = doc.warning;
            app.eula_text = Some(doc.text);
            if first_load {
                app.eula_scrolled_to_bottom = false;
                app.eula_checked = false;
            }
            Task::none()
        }
        Message::EulaLoaded(Err(e)) => {
            app.eula_loading = false;
            app.eula_error = Some(e);
            Task::none()
        }
        Message::EulaUpdateChecked(result) => handle_eula_update(app, result),
        Message::EulaRetry => {
            if app.eula_loading {
                return Task::none();
            }
            app.eula_loading = true;
            app.eula_error = None;
            app.eula_scrolled_to_bottom = false;
            app.eula_checked = false;
            let config = app.config.clone();
            Task::perform(
                async move { launcher::eula::load_or_fetch(&config).await },
                Message::EulaLoaded,
            )
        }
        Message::EulaScrolled(at_bottom) => {
            if at_bottom {
                app.eula_scrolled_to_bottom = true;
            }
            Task::none()
        }
        Message::EulaChecked(checked) => {
            if app.eula_scrolled_to_bottom {
                app.eula_checked = checked;
            }
            Task::none()
        }
        Message::EulaAccept => accept_eula(app),
        Message::EulaCancel => {
            app.eula_required = false;
            app.pending_action = None;
            app.eula_checked = false;
            app.status = "已取消当前操作，尚未同意 EULA".to_string();
            Task::none()
        }
        Message::OpenServerFolder => {
            let Some(version) = app.selected_version.as_ref() else {
                app.status = "请先在「启动」页选择版本".to_string();
                return Task::none();
            };
            let dir = launcher::server::server_dir(&app.config, &version.id);
            Task::perform(
                async move {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    open::that(&dir).map_err(|e| e.to_string())
                },
                Message::OpenFolderDone,
            )
        }
        Message::ToggleBrandTitle(on) => {
            app.config.brand_title = on;
            Task::none()
        }
        Message::BrandTextChanged(text) => {
            app.config.brand_text = text;
            Task::none()
        }
        Message::ToggleGameDefaults(on) => {
            app.config.apply_game_defaults = on;
            Task::none()
        }
        Message::ToggleDisableMultiplayer(on) => {
            app.config.disable_multiplayer = on;
            persist_config_task(app)
        }
        Message::ToggleDisableChat(on) => {
            app.config.disable_chat = on;
            persist_config_task(app)
        }
        Message::ToggleFullscreen(on) => {
            app.config.fullscreen = on;
            persist_config_task(app)
        }
        Message::WindowWidthChanged(text) => {
            app.window_width_input = text;
            if let Ok(width) = app.window_width_input.trim().parse::<u32>() {
                if (320..=7680).contains(&width) {
                    app.config.window_width = width;
                    return persist_config_task(app);
                }
            }
            Task::none()
        }
        Message::WindowHeightChanged(text) => {
            app.window_height_input = text;
            if let Ok(height) = app.window_height_input.trim().parse::<u32>() {
                if (240..=4320).contains(&height) {
                    app.config.window_height = height;
                    return persist_config_task(app);
                }
            }
            Task::none()
        }
        Message::GraphicsBackendSelected(backend) => {
            app.config.graphics_backend = backend;
            persist_config_task(app)
        }
        Message::RefreshMods => load_mods_task(app),
        Message::ModsLoaded(root, mode, _)
            if root != app.config.root_dir || mode != app.config.enhanced_experience =>
        {
            Task::none()
        }
        Message::ModsLoaded(_, _, Ok(mods)) => {
            app.mods = mods;
            Task::none()
        }
        Message::ModsLoaded(_, _, Err(e)) => {
            app.status = format!("加载模组失败: {}", e);
            Task::none()
        }
        Message::ToggleMod(file_name, enabled) => {
            let Some(version) = app.mods_version.clone() else {
                return Task::none();
            };
            let config = app.config.clone();
            app.status = if enabled {
                "正在启用模组...".to_string()
            } else {
                "正在禁用模组...".to_string()
            };
            Task::perform(
                async move {
                    let dir = launcher::mods::mods_dir(&config, &version);
                    launcher::mods::set_enabled(&dir, &file_name, enabled)
                },
                Message::ModToggled,
            )
        }
        Message::ModToggled(Ok(())) => {
            app.status = "模组状态已更新（下次启动生效）".to_string();
            load_mods_task(app)
        }
        Message::ModToggled(Err(e)) => {
            app.status = format!("切换模组失败: {}", e);
            Task::none()
        }
        Message::ModDeletePressed(file_name) => {
            if app.pending_mod_delete.as_deref() == Some(file_name.as_str()) {
                app.pending_mod_delete = None;
                let Some(version) = app.mods_version.clone() else {
                    return Task::none();
                };
                let config = app.config.clone();
                app.status = "正在移入模组回收站...".to_string();
                Task::perform(
                    async move {
                        let dir = launcher::mods::mods_dir(&config, &version);
                        launcher::mods::delete_to_trash(&dir, &file_name)
                            .map(|p| p.display().to_string())
                    },
                    Message::ModDeleted,
                )
            } else {
                app.pending_mod_delete = Some(file_name);
                app.status = "再次点击确认删除（移动到 mods/.deepslate-trash/）".to_string();
                Task::none()
            }
        }
        Message::ModDeleted(Ok(path)) => {
            app.status = format!("已移入模组回收站: {path}");
            load_mods_task(app)
        }
        Message::ModDeleted(Err(e)) => {
            app.status = format!("删除模组失败: {}", e);
            Task::none()
        }
        Message::EmptyModTrash => {
            let Some(version) = app.mods_version.clone() else {
                return Task::none();
            };
            let config = app.config.clone();
            Task::perform(
                async move {
                    let dir = launcher::mods::mods_dir(&config, &version);
                    launcher::mods::empty_trash(&dir)
                },
                Message::ModTrashEmptied,
            )
        }
        Message::ModTrashEmptied(Ok(count)) => {
            app.status = format!("已清空模组回收站（{count} 项）");
            Task::none()
        }
        Message::ModTrashEmptied(Err(e)) => {
            app.status = format!("清空模组回收站失败: {}", e);
            Task::none()
        }
        Message::OpenModsFolder => {
            let Some(version) = app.mods_version.as_ref() else {
                return Task::none();
            };
            let dir = launcher::mods::mods_dir(&app.config, version);
            Task::perform(
                async move {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    open::that(&dir).map_err(|e| e.to_string())
                },
                Message::OpenFolderDone,
            )
        }
        Message::CoreEvent(event) => match event {
            CoreEvent::DownloadProgress(p) => {
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
                        app.enhanced_started_at = None;
                        app.enhanced_startup_duration = None;
                        app.enhanced_phase = None;
                    }
                    GameState::Running { .. } => {
                        app.launching = false;
                        app.game_running = true;
                        if app.active_managed {
                            let started = Instant::now();
                            app.enhanced_started_at = Some(started);
                            app.enhanced_startup_duration = None;
                            app.enhanced_phase = Some(("Minecraft 进程启动".into(), started));
                        }
                    }
                    GameState::Stopped { .. } => {
                        // The launch task releases the library lock after emitting this event.
                        // Its completion message is the only safe point to refresh the map.
                    }
                }
                Task::none()
            }
            CoreEvent::FriendsSnapshot {
                profile_id,
                snapshot,
            } => {
                if app
                    .selected_profile
                    .as_ref()
                    .is_some_and(|profile| profile.uuid == profile_id)
                    && snapshot.updated_at >= app.friends.updated_at
                {
                    app.friends = snapshot.clone();
                    app.friends_loading = false;
                    app.friend_error = None;
                }
                let account_id = profile_id.clone();
                Task::perform(
                    async move { launcher::friends::save_snapshot_for(&account_id, &snapshot) },
                    move |result| Message::FriendSnapshotSaved(profile_id, result),
                )
            }
            CoreEvent::BridgeState { state, message } => {
                let phase = state
                    .strip_prefix("phase_")
                    .map(|phase| match phase {
                        "client_starting" => "正在启动 Minecraft",
                        "client_started" => "Minecraft 已启动",
                        "preparing_instance" => "正在准备独立游戏实例",
                        "resolving_components" => "正在检查 Fabric 与 Bridge 组件",
                        "checking_game_files" => "正在检查并安装 Minecraft 文件",
                        "validating_components" => "正在校验游戏与 Bridge 组件",
                        "starting_client" => "正在启动 Minecraft 客户端",
                        "resource_loading" => "正在加载游戏资源",
                        "resource_finalizing" => "正在完成资源初始化",
                        "localized_resources_ready" => "中文与字体已就绪",
                        "connecting" => "正在连接服务器",
                        "login_started" => "正在开始登录",
                        "authenticating" => "正在验证身份",
                        "server_configuring" => "正在应用服务器配置",
                        "server_configured" => "服务器配置已就绪",
                        "server_joined" => "正在进入服务器世界",
                        "opening_world" => "正在打开世界",
                        "creating_world" => "正在创建世界",
                        "rules_applied" => "正在应用游戏规则",
                        "world_loading" => "正在加载世界",
                        "world_start_server" => "正在启动本地服务器",
                        "world_prepare_spawn" => "正在准备出生区域",
                        "world_initial_chunks" => "正在加载初始区块",
                        "world_player_chunks" => "正在加载周边区块",
                        "world_attached" => "正在准备客户端世界",
                        "first_chunk" => "周边地形已开始加载",
                        "ready" => "游戏已就绪",
                        "world_ready" => "世界已就绪",
                        "operation_failed" => "操作未能完成",
                        _ => "",
                    })
                    .filter(|label| !label.is_empty());
                app.status = if let Some(phase) = phase {
                    match message {
                        Some(progress) => format!("{phase}（{progress}）"),
                        None => phase.to_string(),
                    }
                } else {
                    match state.as_str() {
                        "waiting_for_bridge" => "等待游戏桥接组件...".into(),
                        "connected" => "桥接组件已连接，等待游戏加载...".into(),
                        "ready" => "游戏已就绪，正在打开世界...".into(),
                        "loading" => "正在加载世界...".into(),
                        "entered" => "已进入世界".into(),
                        "saving" => "正在保存并退出世界...".into(),
                        "closed" => "世界已关闭，等待游戏退出...".into(),
                        "failed" => format!("增强体验会话失败: {}", message.unwrap_or_default()),
                        "update_failed" => format!(
                            "更新失败，使用已验证的组件: {}",
                            message.unwrap_or_default()
                        ),
                        _ => return Task::none(),
                    }
                };
                let phase_label = phase.or_else(|| match state.as_str() {
                    "waiting_for_bridge" => Some("等待游戏桥接组件"),
                    "connected" => Some("桥接组件已连接"),
                    "ready" => Some("游戏已就绪"),
                    "loading" => Some("正在加载世界"),
                    "entered" => Some("已进入世界"),
                    "saving" => Some("正在保存并退出世界"),
                    "closed" => Some("世界已关闭"),
                    _ => None,
                });
                if app.enhanced_started_at.is_some() {
                    if let Some(label) = phase_label {
                        if app
                            .enhanced_phase
                            .as_ref()
                            .is_none_or(|(current, _)| current != label)
                        {
                            app.enhanced_phase = Some((label.to_string(), Instant::now()));
                        }
                    }
                }
                if phase == Some("world_ready") || state == "entered" {
                    if let Some(started) = app.enhanced_started_at.take() {
                        app.enhanced_startup_duration = Some(started.elapsed());
                    }
                    app.enhanced_phase = None;
                } else if state == "failed" {
                    app.enhanced_started_at = None;
                    app.enhanced_phase = None;
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
            CoreEvent::ServerLog { is_stderr, line } => {
                if app.server_logs.len() >= 2000 {
                    app.server_logs.drain(0..500);
                }
                app.server_logs.push(if is_stderr {
                    format!("[E] {line}")
                } else {
                    line
                });
                Task::none()
            }
            CoreEvent::ServerState(state) => match state {
                launcher::events::ServerState::Starting { version } => {
                    app.server_running = true;
                    app.server_ready = false;
                    app.status = format!("正在启动服务端 {version} ...");
                    Task::none()
                }
                launcher::events::ServerState::Running { pid } => {
                    app.server_running = true;
                    app.server_ready = false;
                    app.status = format!("服务端运行中（PID {pid}），等待就绪...");
                    Task::none()
                }
                launcher::events::ServerState::Ready { port } => {
                    app.server_running = true;
                    app.server_ready = true;
                    app.status = format!("服务端已就绪：加入地址 127.0.0.1:{port}");
                    if app.pending_server_join {
                        app.pending_server_join = false;
                        start_launch(
                            app,
                            Some(launcher::launch::QuickPlayTarget::Multiplayer(format!(
                                "127.0.0.1:{port}"
                            ))),
                        )
                    } else {
                        Task::none()
                    }
                }
                launcher::events::ServerState::Stopped { exit_code } => {
                    app.server_running = false;
                    app.server_ready = false;
                    app.server_busy = false;
                    app.pending_server_join = false;
                    app.status = format!("服务端已停止（退出码 {exit_code}）");
                    Task::none()
                }
            },
        },
    }
}

/// 加载全局地图库。地图库与版本解耦，`instance/saves` 会链接到 `<root>/maps`。
fn load_friends_task(app: &mut App) -> Task<Message> {
    let Some(profile) = app.selected_profile.as_ref() else {
        app.friends = launcher::friends::FriendSnapshot::default();
        app.friends_loading = false;
        app.friend_error = None;
        return Task::none();
    };
    let account_id = profile.uuid.clone();
    app.friends_loading = true;
    app.friend_error = None;
    let task_account_id = account_id.clone();
    Task::perform(
        async move { launcher::friends::load_for(&task_account_id) },
        move |result| Message::FriendsLoaded(account_id, result),
    )
}

fn load_saves_task(app: &mut App) -> Task<Message> {
    let config = app.config.clone();
    let root = config.root_dir.clone();
    app.pending_save_delete = None;
    let pending_config = config.clone();
    let pending_root = root.clone();
    let worlds = Task::perform(
        async move {
            launcher::runtime::ensure(&config)?;
            launcher::maps::list(&config)
        },
        move |result| Message::SavesLoaded(root.clone(), result),
    );
    let pending = Task::perform(
        async move { launcher::maps::list_pending_creations(&pending_config) },
        move |result| Message::PendingCreationsLoaded(pending_root.clone(), result),
    );
    Task::batch([worlds, pending])
}

fn import_map_task(config: AppConfig, path: std::path::PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            let map = launcher::maps::import_path(&config, &path)?;
            Ok::<String, String>(format!("{} ({})", map.name, map.id))
        },
        Message::MapImported,
    )
}

fn load_mods_task(app: &mut App) -> Task<Message> {
    let selected = if app.config.enhanced_experience {
        Some("26.3".to_string())
    } else {
        app.selected_version.as_ref().map(|v| v.id.clone())
    };
    let Some(version) = selected else {
        app.mods.clear();
        app.mods_version = None;
        app.status = "请先在「启动」页选择版本".to_string();
        return Task::none();
    };
    let config = app.config.clone();
    let root = config.root_dir.clone();
    let enhanced = config.enhanced_experience;
    app.mods_version = Some(version.clone());
    app.pending_mod_delete = None;
    Task::perform(
        async move {
            let dir = launcher::mods::mods_dir(&config, &version);
            launcher::mods::ensure_dir(&dir)?;
            Ok(launcher::mods::list(&dir))
        },
        move |result| Message::ModsLoaded(root.clone(), enhanced, result),
    )
}

fn persist_config_task(app: &App) -> Task<Message> {
    let config = app.config.clone();
    Task::perform(
        async move { launcher::config::save(&config) },
        Message::ConfigPersisted,
    )
}

/// 打开主窗口内的 EULA 模态，并记住用户原本要执行的动作。
fn show_eula_gate(app: &mut App, pending: PendingAction) -> Task<Message> {
    if eula_is_accepted(app) {
        return resume_pending_action(app, pending);
    }

    app.eula_required = true;
    app.pending_action = Some(pending);
    app.eula_scrolled_to_bottom = false;
    app.eula_checked = false;
    if app.eula_checking {
        return Task::none();
    }
    if app.eula_text.is_some() {
        return Task::none();
    }
    if app.eula_loading {
        return Task::none();
    }

    app.eula_loading = true;
    app.eula_error = None;
    let config = app.config.clone();
    Task::perform(
        async move { launcher::eula::load_or_fetch(&config).await },
        Message::EulaLoaded,
    )
}

/// 标记文件是唯一事实源；每次真正执行启动/开服/建世界前重新读一次，
/// 防止运行中删除 eula.txt 或切换根目录后继续使用旧的内存状态。
fn eula_is_accepted(app: &mut App) -> bool {
    let marker = launcher::eula::is_accepted(&app.config);
    // 接受标记之外还要核对“接受时的正文哈希”和当前缓存正文。否则协议
    // 更新检查把内存状态置为未接受后，下一次动作又会因为 eula.txt 仍在
    // 而把旧同意重新当成有效。
    let hash_matches = launcher::eula::accepted_sha1(&app.config)
        .zip(launcher::eula::load_cached(&app.config).map(|doc| doc.sha1))
        .map_or(true, |(accepted, current)| accepted == current);
    let accepted = marker && hash_matches;
    app.eula_accepted = accepted;
    accepted
}

/// 启动后的协议更新检查结果。
///
/// - 未变化：继续启动期间被“检查中”挡下的动作；
/// - 已变化：把新正文装进模态并强制重新阅读/勾选；
/// - 检查失败：不把网络错误当作“协议未变”，但也不阻断已接受版本的离线使用，
///   在状态栏留下提示，下次启动/重试时再检查。
fn handle_eula_update(
    app: &mut App,
    result: Result<launcher::eula::EulaUpdate, String>,
) -> Task<Message> {
    app.eula_checking = false;
    match result {
        Ok(launcher::eula::EulaUpdate::NotAccepted) => {
            app.eula_accepted = false;
            app.eula_text = None;
            app.eula_sha1 = None;
            if app.pending_action.is_some() {
                let pending = app.pending_action.take().unwrap_or(PendingAction::None);
                show_eula_gate(app, pending)
            } else {
                Task::none()
            }
        }
        Ok(launcher::eula::EulaUpdate::Unchanged) => {
            app.eula_accepted = true;
            app.eula_required = false;
            let pending = app.pending_action.take().unwrap_or(PendingAction::None);
            resume_pending_action(app, pending)
        }
        Ok(launcher::eula::EulaUpdate::Changed(doc)) => {
            let summary = doc.summary();
            app.eula_accepted = false;
            app.eula_sha1 = Some(doc.sha1.clone());
            app.eula_source = Some(summary);
            app.eula_warning = Some(if let Some(warning) = doc.warning {
                format!("协议已更新，需要重新阅读并同意。{warning}")
            } else {
                "协议已更新，需要重新阅读并同意。".to_string()
            });
            app.eula_text = Some(doc.text);
            app.eula_error = None;
            app.eula_loading = false;
            app.eula_scrolled_to_bottom = false;
            app.eula_checked = false;
            app.eula_required = true;
            app.status = "Minecraft EULA 已更新，请重新阅读并同意".to_string();
            Task::none()
        }
        Err(error) => {
            app.status = format!("无法检查 EULA 更新，将按已接受版本继续: {error}");
            if app.eula_required {
                app.eula_loading = false;
                app.eula_error = Some(error);
                Task::none()
            } else {
                let pending = app.pending_action.take().unwrap_or(PendingAction::None);
                resume_pending_action(app, pending)
            }
        }
    }
}

/// 写 `<root>/eula.txt`，然后继续被拦截的启动/开服/建世界动作。
fn accept_eula(app: &mut App) -> Task<Message> {
    if app.eula_text.is_none()
        || app.eula_error.is_some()
        || !app.eula_scrolled_to_bottom
        || !app.eula_checked
    {
        return Task::none();
    }
    let Some(sha1) = app.eula_sha1.clone() else {
        app.eula_error = Some("缺少 EULA 正文哈希，请重新获取后再同意".to_string());
        return Task::none();
    };
    if let Err(error) = launcher::eula::accept_document(&app.config, &sha1) {
        app.eula_error = Some(error);
        return Task::none();
    }

    app.eula_accepted = true;
    app.config.server_eula_accepted = true;
    app.eula_required = false;
    app.eula_checked = false;
    app.status = "已同意 Minecraft EULA".to_string();
    let pending = app.pending_action.take().unwrap_or(PendingAction::None);
    Task::batch([
        persist_config_task(app),
        resume_pending_action(app, pending),
    ])
}

fn resume_pending_action(app: &mut App, pending: PendingAction) -> Task<Message> {
    match pending {
        PendingAction::None => Task::none(),
        PendingAction::Launch(quick_play) => start_launch(app, quick_play),
        PendingAction::ServerInstall => server_action(app, true),
        PendingAction::ServerStart => server_action(app, false),
        PendingAction::CreateWorld => create_world_action(app),
    }
}

/// 统一的启动入口：普通启动 / 快速进入存档 / 快速加入服务器。
fn start_launch(
    app: &mut App,
    quick_play: Option<launcher::launch::QuickPlayTarget>,
) -> Task<Message> {
    start_launch_with_creation(app, quick_play, None)
}

fn start_launch_with_creation(
    app: &mut App,
    quick_play: Option<launcher::launch::QuickPlayTarget>,
    creation: Option<launcher::bridge::ManagedTarget>,
) -> Task<Message> {
    let enhanced = app.config.enhanced_experience;
    let managed_world = if enhanced {
        if let Some(target) = creation.clone() {
            Some(target)
        } else {
            match &quick_play {
                Some(launcher::launch::QuickPlayTarget::Singleplayer(id)) => {
                    Some(launcher::bridge::ManagedTarget::Open(id.clone()))
                }
                Some(launcher::launch::QuickPlayTarget::Multiplayer(address)) => {
                    if app.config.disable_multiplayer {
                        app.status = "当前已禁用多人游戏".into();
                        return Task::none();
                    }
                    Some(launcher::bridge::ManagedTarget::JoinServer(
                        address.trim().to_string(),
                    ))
                }
                None => {
                    app.page = Page::Maps;
                    app.status = "请选择要进入的世界".into();
                    return load_saves_task(app);
                }
                _ => {
                    app.status = "增强体验暂不支持加入 Realm".into();
                    return Task::none();
                }
            }
        }
    } else {
        None
    };
    let version = match (enhanced, app.selected_version.clone()) {
        (true, _) => None,
        (false, Some(version)) => Some(version),
        (false, None) => {
            app.status = "请先选择一个版本".to_string();
            return Task::none();
        }
    };
    let Some(profile) = app.selected_profile.clone() else {
        app.status = "请先选择或创建档案".to_string();
        return Task::none();
    };
    let Some(java) = app.selected_java.clone() else {
        app.status = "请先选择 Java 运行时".to_string();
        return Task::none();
    };
    if app.fabric_installing || app.downloading {
        app.status = "当前有下载/安装任务，请稍后再启动".to_string();
        return Task::none();
    }
    if app.game_running || app.launching {
        app.status = "游戏已在运行".to_string();
        return Task::none();
    }
    if !eula_is_accepted(app) {
        return show_eula_gate(app, PendingAction::Launch(quick_play));
    }
    if app.eula_checking {
        app.pending_action = Some(PendingAction::Launch(quick_play));
        app.status = "正在检查 Minecraft EULA 是否有更新...".to_string();
        return Task::none();
    }

    let config = app.config.clone();
    let mode = if enhanced {
        launcher::runtime::RuntimeMode::Enhanced
    } else {
        launcher::runtime::RuntimeMode::Normal
    };
    let runtime = launcher::runtime::paths_for_mode(&app.config, mode);
    let events = core_event_sender();

    if app.config.log_window && app.log_writer.is_none() {
        app.log_writer = spawn_log_window(app.dark_mode);
    }

    let branding = app
        .config
        .brand_title
        .then(|| app.config.brand_text.clone())
        .filter(|text| !text.trim().is_empty());
    let selected_profile = profile.clone();
    let active_id = managed_world
        .as_ref()
        .and_then(|target| target.id())
        .map(str::to_string);
    let mut opts = launcher::launch::LaunchOptions {
        version: version.as_ref().map(|v| v.id.clone()).unwrap_or_default(),
        game_dir: runtime.game_dir,
        instance_dir: runtime.instance_dir,
        map_config: Some(config.clone()),
        username: profile.name,
        uuid: profile.uuid,
        runtime_dir: std::path::PathBuf::from(java.path),
        auth_method: profile.auth_method,
        access_token: profile.access_token,
        download_concurrency: app.config.download_concurrency,
        quick_play: if enhanced { None } else { quick_play.clone() },
        managed_world,
        branding,
        apply_game_defaults: app.config.apply_game_defaults,
        disable_multiplayer: app.config.disable_multiplayer,
        disable_chat: app.config.disable_chat,
        fullscreen: app.config.fullscreen,
        window_width: app.config.window_width,
        window_height: app.config.window_height,
        graphics_backend: app.config.graphics_backend.clone(),
    };

    app.status = match &quick_play {
        Some(launcher::launch::QuickPlayTarget::Singleplayer(id)) => {
            format!("正在进入存档 {id} ...")
        }
        Some(launcher::launch::QuickPlayTarget::Multiplayer(address)) => {
            format!("正在加入服务器 {address} ...")
        }
        Some(launcher::launch::QuickPlayTarget::Realms(id)) => {
            format!("正在进入 Realm {id} ...")
        }
        None => format!(
            "正在启动 {} ...",
            version.as_ref().map(|v| v.id.as_str()).unwrap_or("26.3")
        ),
    };
    app.launching = true;
    app.game_running = true;
    app.active_managed = enhanced;
    app.progress = None;
    app.active_map = match &quick_play {
        Some(launcher::launch::QuickPlayTarget::Singleplayer(id)) => Some(id.clone()),
        _ => active_id,
    };
    Task::perform(
        async move {
            let profile = launcher::profiles::load()?.current_profile(&selected_profile)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let profile_needs_refresh = profile.needs_refresh(now);
            if profile_needs_refresh {
                let refresh = profile
                    .refresh_token
                    .as_deref()
                    .filter(|token| !token.is_empty())
                    .ok_or("在线凭据已过期，请重新登录")?;
                let online = launcher::auth::refresh_login(refresh, &events).await?;
                if online.uuid != profile.uuid {
                    return Err("刷新后的账号与当前档案不一致，请重新登录".to_string());
                }
                let updated = launcher::profiles::upsert_online(online)?;
                opts.access_token = updated.access_token;
                opts.username = updated.name;
            } else {
                opts.access_token = profile.access_token;
                opts.username = profile.name;
            }
            let _ = events
                .send(CoreEvent::BridgeState {
                    state: "phase_preparing_instance".into(),
                    message: None,
                })
                .await;
            launcher::runtime::ensure_for_mode(&config, mode)?;
            if enhanced {
                let installed = match launcher::enhanced::install(&config, events.clone()).await {
                    Ok(installed) => installed,
                    Err(update_error) => {
                        let previous =
                            launcher::enhanced::load_installed(&config).map_err(|_| {
                                format!("安装增强体验失败，且没有可用的已安装组合: {update_error}")
                            })?;
                        let _ = events
                            .send(CoreEvent::BridgeState {
                                state: "update_failed".into(),
                                message: Some(update_error),
                            })
                            .await;
                        previous
                    }
                };
                opts.version = installed.lock.version_id;
            }
            let _ = events
                .send(CoreEvent::BridgeState {
                    state: "phase_starting_client".into(),
                    message: None,
                })
                .await;
            launcher::launch::launch(opts, events)
                .await
                .map_err(|e| e.to_string())
        },
        Message::LaunchFinished,
    )
}

/// 安装或启动本地服务端（install_only=true 时只下载/校验）。
fn server_action(app: &mut App, install_only: bool) -> Task<Message> {
    if app.server_busy {
        return Task::none();
    }
    if !install_only && app.server_running {
        app.status = "服务端已在运行".to_string();
        return Task::none();
    }
    let version = if app.config.enhanced_experience {
        launcher::enhanced::MINECRAFT.to_string()
    } else if let Some(version) = &app.selected_version {
        version.id.clone()
    } else {
        app.status = "请先在「启动」页选择版本".to_string();
        return Task::none();
    };
    let Some(java) = app.selected_java.clone() else {
        app.status = "请先在「启动」页选择 Java 运行时".to_string();
        return Task::none();
    };
    let Ok(port) = app.server_port_input.trim().parse::<u16>() else {
        app.status = "端口必须是 1-65535 的数字".to_string();
        return Task::none();
    };
    if port == 0 {
        app.status = "端口不能为 0".to_string();
        return Task::none();
    }
    if !eula_is_accepted(app) {
        return show_eula_gate(
            app,
            if install_only {
                PendingAction::ServerInstall
            } else {
                PendingAction::ServerStart
            },
        );
    }
    if app.eula_checking {
        app.pending_action = Some(if install_only {
            PendingAction::ServerInstall
        } else {
            PendingAction::ServerStart
        });
        app.status = "正在检查 Minecraft EULA 是否有更新...".to_string();
        return Task::none();
    }

    app.config.server_port = port;
    let opts = launcher::server::ServerOptions::new(
        &app.config,
        &version,
        std::path::PathBuf::from(java.path),
        app.config.server_memory_mb,
        port,
        app.config.server_online_mode,
        true,
    );
    let events = core_event_sender();
    app.server_busy = true;
    if install_only {
        app.status = "正在安装/校验服务端...".to_string();
        let action = Task::perform(
            async move { launcher::server::install(&opts, &events).await.map(|_| ()) },
            Message::ServerInstalled,
        );
        Task::batch([persist_config_task(app), action])
    } else {
        app.server_logs.clear();
        app.status = format!("正在启动服务端（端口 {port}）...");
        let action = Task::perform(
            async move { launcher::server::start(opts, events).await },
            Message::ServerStarted,
        );
        Task::batch([persist_config_task(app), action])
    }
}

fn create_world_action(app: &mut App) -> Task<Message> {
    if app.config.enhanced_experience {
        if app.new_world_name.trim().is_empty() || app.new_world_name.trim().chars().count() > 64 {
            app.status = "世界名称须为 1–64 个字符".into();
            return Task::none();
        }
        if !eula_is_accepted(app) {
            return show_eula_gate(app, PendingAction::CreateWorld);
        }
        if app.eula_checking {
            app.pending_action = Some(PendingAction::CreateWorld);
            app.status = "正在检查 Minecraft EULA 是否有更新...".into();
            return Task::none();
        }
        let target = launcher::bridge::ManagedTarget::Create {
            id: launcher::maps::new_id(),
            name: app.new_world_name.trim().to_string(),
            seed: app.new_world_seed.trim().to_string(),
            mode: app.new_world_gamemode.clone(),
            difficulty: app.new_world_difficulty.clone(),
            preset: if app.new_world_flat { "flat" } else { "normal" }.into(),
            cheats: app.new_world_cheats,
            rules: launcher::bridge::WorldGameRules {
                keep_inventory: app.new_world_keep_inventory,
                mob_griefing: app.new_world_mob_griefing,
                daylight_cycle: app.new_world_daylight_cycle,
                weather_cycle: app.new_world_weather_cycle,
                hostile_mobs: app.new_world_hostile_mobs,
                pvp: app.new_world_pvp,
            },
        };
        return start_launch_with_creation(app, None, Some(target));
    }
    if app.new_world_creating {
        return Task::none();
    }
    if app.game_running {
        app.status = "请先退出游戏再生成世界".to_string();
        return Task::none();
    }
    let Some(version) = app.selected_version.clone() else {
        app.status = "请先在「启动」页选择版本".to_string();
        return Task::none();
    };
    let Some(java) = app.selected_java.clone() else {
        app.status = "请先在「启动」页选择 Java 运行时".to_string();
        return Task::none();
    };
    if !eula_is_accepted(app) {
        return show_eula_gate(app, PendingAction::CreateWorld);
    }
    if app.eula_checking {
        app.pending_action = Some(PendingAction::CreateWorld);
        app.status = "正在检查 Minecraft EULA 是否有更新...".to_string();
        return Task::none();
    }
    let name = app.new_world_name.trim().to_string();
    if name.is_empty() {
        app.status = "世界名称不能为空".to_string();
        return Task::none();
    }
    let config = app.config.clone();
    let spec = launcher::worldgen::WorldSpec {
        name,
        seed: app.new_world_seed.trim().to_string(),
        world_type: if app.new_world_flat {
            launcher::server::WorldType::Flat
        } else {
            launcher::server::WorldType::Normal
        },
        game_mode: app.new_world_gamemode.clone(),
        cheats: app.new_world_cheats,
    };
    let events = core_event_sender();
    app.new_world_creating = true;
    app.status = "正在生成世界（首次会下载 server.jar，可能需要几分钟）...".to_string();
    Task::perform(
        async move {
            launcher::worldgen::generate(
                &config,
                &version.id,
                std::path::PathBuf::from(java.path),
                config.server_memory_mb.clamp(1024, 4096),
                &spec,
                &events,
            )
            .await
            .map(|map| map.id.clone())
        },
        Message::WorldCreated,
    )
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

/// 扫描本地 versions/ 目录（磁盘 IO 很轻，放默认线程池即可）
fn load_local_versions(game_dir: std::path::PathBuf) -> Task<Message> {
    let source = game_dir.clone();
    Task::perform(
        async move { launcher::versions::scan(&game_dir) },
        move |result| Message::LocalVersionsLoaded(source.clone(), result),
    )
}

/// 把本地版本投影成 pick_list 可用的 VersionInfo（「已安装」tab 专用）
fn local_choices(
    local: &[launcher::versions::LocalVersion],
) -> Vec<launcher::download::VersionInfo> {
    local
        .iter()
        .map(|v| {
            let loader = match v.kind.as_str() {
                "fabric" => Some(format!(
                    "Fabric {}",
                    v.loader_version.clone().unwrap_or_default()
                )),
                "quilt" => Some(format!(
                    "Quilt {}",
                    v.loader_version.clone().unwrap_or_default()
                )),
                "forge" => Some("Forge".to_string()),
                "neoforge" => Some("NeoForge".to_string()),
                _ => None,
            };
            launcher::download::VersionInfo {
                id: v.id.clone(),
                kind: "installed".to_string(),
                url: String::new(),
                release_time: String::new(),
                local: true,
                loader,
                game_version: v.has_loader().then(|| v.game_version.clone()),
            }
        })
        .collect()
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
    // 入场动画进行中：8ms 滴答（约 125fps 上限），显示刷新率/vsync 仍会
    // 决定实际帧率；动画只有 300ms，结束自动停，空闲不重绘。
    if app.page_anim.is_animating(app.now) {
        subs.push(iced::time::every(Duration::from_millis(8)).map(Message::Tick));
    } else if app.enhanced_started_at.is_some() {
        subs.push(iced::time::every(Duration::from_millis(250)).map(Message::Tick));
    }
    Subscription::batch(subs)
}

fn listen_core_events() -> impl Stream<Item = Message> {
    iced::stream::channel(
        64,
        |mut output: iced::futures::channel::mpsc::Sender<Message>| async move {
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
        },
    )
}

// ── 视图 ─────────────────────────────────────────────────
mod views;
use views::view;
mod cli;
use cli::*;

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--log-window") {
        // 独立日志窗口子进程模式：主进程经 stdin 逐行喂日志
        let dark = !args.iter().any(|a| a == "--light");
        return log_window::run(dark);
    }
    if args.iter().any(|a| a == "--install-fabric") {
        std::process::exit(run_cli_install_fabric(&args));
    }
    if args.iter().any(|a| a == "--print-command") {
        std::process::exit(run_cli_print_command(&args));
    }
    if args.iter().any(|a| a == "--launch") {
        std::process::exit(run_cli_launch(&args));
    }
    if args.iter().any(|a| a == "--list-saves") {
        std::process::exit(run_cli_list_saves(&args));
    }
    if args.iter().any(|a| a == "--list-maps") {
        std::process::exit(run_cli_list_maps(&args));
    }
    if args.iter().any(|a| {
        a == "--import-map"
            || a == "--export-map"
            || a == "--trash-map"
            || a == "--backup-map"
            || a == "--list-map-backups"
            || a == "--restore-map"
    }) {
        std::process::exit(run_cli_map_op(&args));
    }
    if args.iter().any(|a| a == "--generate-map") {
        std::process::exit(run_cli_generate_map(&args));
    }
    if args.iter().any(|a| a == "--list-mods") {
        std::process::exit(run_cli_list_mods(&args));
    }
    if args.iter().any(|a| a == "--host-server") {
        std::process::exit(run_cli_host_server(&args));
    }
    if args.iter().any(|a| a == "--create-world") {
        std::process::exit(run_cli_create_world(&args));
    }
    if args
        .iter()
        .any(|a| a == "--rename-save" || a == "--copy-save" || a == "--trash-save")
    {
        std::process::exit(run_cli_save_op(&args));
    }

    iced::application(App::boot, update, view)
        .title(|_app: &App| "深板岩启动器".to_string())
        .theme(|app: &App| {
            if app.dark_mode {
                style::theme_dark()
            } else {
                style::theme_light()
            }
        })
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

    #[cfg(feature = "screenshot")]
    fn render_screenshot_image(app: &App, output: &std::path::Path, width: u32, height: u32) {
        use iced::advanced::{graphics::Viewport, mouse, renderer};
        use iced_runtime::user_interface::{Cache, UserInterface};

        let bounds = Size::new(width as f32, height as f32);
        let mut renderer = iced::Renderer::new(iced::Font::default(), iced::Pixels(16.0));
        let mut interface = UserInterface::build(view(app), bounds, Cache::new(), &mut renderer);
        let theme = if app.dark_mode {
            style::theme_dark()
        } else {
            style::theme_light()
        };
        interface.draw(
            &mut renderer,
            &theme,
            &renderer::Style::default(),
            mouse::Cursor::Unavailable,
        );

        let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("create screenshot pixmap");
        let mut mask = tiny_skia::Mask::new(width, height).expect("create screenshot clip mask");
        let viewport = Viewport::with_physical_size(Size::new(width, height), 1.0);
        let damage = [iced::Rectangle::with_size(bounds)];
        renderer.draw(
            &mut pixmap.as_mut(),
            &mut mask,
            &viewport,
            &damage,
            style::scheme_of(app.dark_mode).bg_deep,
        );
        pixmap.save_png(output).expect("save screenshot PNG");
    }

    #[cfg(feature = "screenshot")]
    fn screenshot_fixture() -> App {
        let mut app = make_app(8);
        app.releases = app.versions.clone();
        app.refilter();
        app.selected_profile = app.profiles.first().cloned();
        app.status = "基线截图状态".into();
        app.saves = vec![launcher::maps::MapSummary {
            manifest: launcher::maps::MapManifest {
                schema: 1,
                id: "sample-world".into(),
                name: "示例世界".into(),
                source: launcher::maps::MapSource::Generated,
                created_at: 0,
                updated_at: 0,
                last_played: 0,
                generator: None,
                game: None,
                migrated_from: None,
            },
            world: launcher::saves::WorldSummary {
                id: "sample-world".into(),
                name: "示例世界".into(),
                game_mode: "survival".into(),
                difficulty: "normal".into(),
                hardcore: false,
                cheats: false,
                last_played: 0,
                version_name: "1.21.4".into(),
                data_version: Some(4189),
                size_bytes: 1_500_000,
                icon: None,
                dir: std::path::PathBuf::from("/tmp/sample-world"),
            },
        }];
        app.config.saved_servers = vec![launcher::config::SavedServer {
            name: "本地测试服".into(),
            address: "127.0.0.1:25565".into(),
        }];
        app.server_ping_status
            .insert("127.0.0.1:25565".into(), "在线 · 1/20".into());
        app
    }

    #[cfg(feature = "screenshot")]
    #[test]
    #[ignore = "手动生成 GUI 离屏验收 PNG"]
    fn render_screenshot_matrix() {
        let output_dir = std::path::PathBuf::from("target/ui-screenshots");
        std::fs::create_dir_all(&output_dir).expect("create screenshot output directory");
        let sizes = [(900, 560), (1040, 700)];
        let pages = [
            (Page::Launch, "launch"),
            (Page::Friends, "friends"),
            (Page::Maps, "maps"),
            (Page::Server, "server"),
            (Page::Mods, "mods"),
            (Page::Home, "home"),
            (Page::Settings, "settings"),
        ];

        for (page, name) in pages {
            for (width, height) in sizes {
                let mut app = screenshot_fixture();
                app.page = page;
                app.now += Duration::from_secs(1);
                let now = app.now;
                let _ = update(&mut app, Message::Tick(now));
                let output = output_dir.join(format!("{name}-{width}x{height}.png"));
                render_screenshot_image(&app, &output, width, height);
            }
        }

        for (width, height) in sizes {
            let mut app = screenshot_fixture();
            app.page = Page::Launch;
            app.config.enhanced_experience = true;
            app.selected_version = None;
            app.show_launch_account = true;
            app.show_launch_maintenance = true;
            app.show_launch_advanced = true;
            app.show_manage_favorites = true;
            let output = output_dir.join(format!("launch-expanded-{width}x{height}.png"));
            render_screenshot_image(&app, &output, width, height);

            let mut app = screenshot_fixture();
            app.page = Page::Maps;
            app.map_menu_open = Some("sample-world".into());
            let output = output_dir.join(format!("maps-more-{width}x{height}.png"));
            render_screenshot_image(&app, &output, width, height);
        }
    }

    #[test]
    fn enhanced_launch_without_target_opens_world_library() {
        let mut app = make_app(1);
        app.config.enhanced_experience = true;
        app.selected_version = None;
        let _ = update(&mut app, Message::LaunchPressed);
        assert_eq!(app.page, Page::Maps);
        assert!(!app.game_running);
    }

    #[test]
    fn disabling_enhanced_mode_keeps_other_version_download_available() {
        let mut app = make_app(4);
        app.config.enhanced_experience = true;
        let manifest = app.versions.clone();
        let _ = update(&mut app, Message::VersionsLoaded(Ok(manifest)));
        assert_eq!(app.releases.len(), 3);
        assert_eq!(app.filtered.len(), 3);

        let other_release = app.releases[2].clone();
        let _ = update(&mut app, Message::VersionSelected(other_release.clone()));
        let selected_id = app.selected_version.as_ref().unwrap().id.clone();
        app.version_tab = VersionTab::Installed;

        let _ = update(&mut app, Message::ToggleEnhancedExperience(false));
        assert!(!app.config.enhanced_experience);
        assert_eq!(app.version_tab, VersionTab::Release);
        assert_eq!(
            app.selected_version.as_ref().map(|v| v.id.as_str()),
            Some(selected_id.as_str())
        );

        let _download_task = update(&mut app, Message::DownloadPressed);
        assert!(app.downloading);
        assert!(app.status.contains(&selected_id));
    }

    #[test]
    fn switching_modes_discards_stale_mod_results() {
        let mut app = make_app(1);
        let root = app.config.root_dir.clone();
        app.mods_version = Some("normal".into());
        let _ = update(&mut app, Message::ToggleEnhancedExperience(true));
        assert!(app.mods_version.is_none());
        let _ = update(
            &mut app,
            Message::ModsLoaded(root, false, Err("stale".into())),
        );
        assert!(!app.status.contains("stale"));
    }

    #[test]
    fn running_game_blocks_pending_world_recovery() {
        let mut app = make_app(1);
        app.game_running = true;
        let _ = update(&mut app, Message::RecoverCreation("pending".into()));
        assert!(app.status.contains("退出游戏"));
    }

    #[test]
    fn unrelated_complete_progress_does_not_unlock_client_download() {
        let mut app = make_app(1);
        app.downloading = true;
        let event = launcher::download::DownloadProgress {
            current: 1,
            total: 1,
            percent: 100,
            current_file: "server".into(),
        };
        let _ = update(
            &mut app,
            Message::CoreEvent(CoreEvent::DownloadProgress(event)),
        );
        assert!(app.downloading);
    }

    #[test]
    fn download_press_does_not_start_second_download() {
        let mut app = make_app(1);
        app.downloading = true;
        app.status = "original".into();
        let _ = update(&mut app, Message::DownloadPressed);
        assert!(app.downloading);
        assert!(app.status.contains("下载"));
        assert!(!app.status.contains("正在下载 1.20."));
    }

    #[test]
    fn running_game_prevents_map_delete_confirmation() {
        let mut app = make_app(1);
        app.game_running = true;
        app.pending_save_delete = Some("world".into());
        let _ = update(&mut app, Message::SaveDeletePressed("world".into()));
        assert!(!app.status.contains("正在移入回收站"));
    }

    #[test]
    fn kill_request_does_not_claim_game_has_exited() {
        let mut app = make_app(1);
        app.game_running = true;
        let _ = update(&mut app, Message::KillFinished(Ok(())));
        assert!(app.game_running);
    }

    #[test]
    fn stopped_event_keeps_active_map_until_launch_task_finishes() {
        let mut app = make_app(1);
        app.game_running = true;
        app.active_map = Some("world".into());
        let _ = update(
            &mut app,
            Message::CoreEvent(CoreEvent::GameState(GameState::Stopped { exit_code: 0 })),
        );
        assert_eq!(app.active_map.as_deref(), Some("world"));
    }

    #[test]
    fn saved_default_profile_is_restored() {
        let mut app = make_app(2);
        let profiles = app.profiles.clone();
        app.config.last_profile = Some(profiles[1].name.clone());
        let store = launcher::ProfileStore {
            profiles: profiles.clone(),
            last_used: Some(profiles[0].name.clone()),
        };
        let _ = update(&mut app, Message::ProfilesLoaded(Ok(store)));
        assert_eq!(
            app.selected_profile.as_ref().unwrap().name,
            profiles[1].name
        );
    }

    #[test]
    fn cancelling_eula_clears_pending_action() {
        let mut app = make_app(1);
        app.eula_required = true;
        app.pending_action = Some(PendingAction::ServerStart);
        let _ = update(&mut app, Message::EulaCancel);
        assert!(!app.eula_required);
        assert!(app.pending_action.is_none());
    }

    #[test]
    fn root_change_clears_old_root_state_and_ignores_late_saves() {
        let mut app = make_app(1);
        let old_root = app.config.root_dir.clone();
        app.local_choices = app.filtered.clone();
        app.selected_version = app.local_choices.first().cloned();
        let _ = update(&mut app, Message::RootDirChanged("new-root".into()));
        assert!(app.local_choices.is_empty());
        assert!(app.selected_version.is_none());
        let _ = update(
            &mut app,
            Message::SavesLoaded(old_root, Err("old response".into())),
        );
        assert!(!app.status.contains("old response"));
    }

    #[test]
    fn root_change_blocks_actions_until_restart() {
        let mut app = make_app(1);
        let _ = update(&mut app, Message::RootDirChanged("new-root".into()));
        let _ = update(&mut app, Message::SaveDeletePressed("world".into()));
        assert!(app.pending_save_delete.is_none());
        assert!(app.status.contains("重新启动"));
    }

    #[test]
    fn profile_creation_requires_saved_root_configuration() {
        let mut app = make_app(1);
        app.new_profile_name = "Steve".into();
        app.root_changed = true;
        let _ = update(&mut app, Message::CreateProfile);
        assert!(app.status.contains("重新启动"));
    }

    #[test]
    fn map_trash_requires_second_confirmation() {
        let mut app = make_app(1);
        let _ = update(&mut app, Message::EmptyTrash);
        assert!(app.status.contains("再次"));
    }

    #[test]
    fn opening_world_requires_explicit_backup_and_version_confirmation() {
        let mut app = make_app(1);
        let _ = update(&mut app, Message::SaveEnter("world".into()));
        assert_eq!(app.pending_world_enter.as_deref(), Some("world"));
        assert!(app.status.contains("再次"));
    }

    #[test]
    fn launch_and_settings_views_expose_scrollable_content() {
        assert!(views::page_requires_outer_scroll(Page::Launch));
        assert!(views::page_requires_outer_scroll(Page::Settings));
        assert!(!views::page_requires_outer_scroll(Page::Maps));
    }

    #[test]
    fn all_seven_navigation_pages_build() {
        let pages = [
            Page::Home,
            Page::Launch,
            Page::Friends,
            Page::Maps,
            Page::Server,
            Page::Mods,
            Page::Settings,
        ];
        assert_eq!(
            pages.map(Page::label),
            ["首页", "启动", "好友", "世界", "服务器", "模组", "设置"]
        );
        let mut app = make_app(1);
        for page in pages {
            app.page = page;
            let _ = view(&app);
        }
    }

    #[test]
    fn launch_sections_and_world_menu_toggle_independently() {
        let mut app = make_app(1);
        assert!(!app.show_launch_account);
        assert!(!app.show_launch_maintenance);
        assert!(!app.show_launch_advanced);
        assert!(!app.show_manage_favorites);

        let _ = update(&mut app, Message::ToggleLaunchAccount);
        let _ = update(&mut app, Message::ToggleLaunchMaintenance);
        let _ = update(&mut app, Message::ToggleLaunchAdvanced);
        let _ = update(&mut app, Message::ToggleManageFavorites);
        assert!(app.show_launch_account);
        assert!(app.show_launch_maintenance);
        assert!(app.show_launch_advanced);
        assert!(app.show_manage_favorites);
        let _ = view(&app);

        let _ = update(&mut app, Message::ToggleMapMenu("first".into()));
        assert_eq!(app.map_menu_open.as_deref(), Some("first"));
        let _ = update(&mut app, Message::ToggleMapMenu("second".into()));
        assert_eq!(app.map_menu_open.as_deref(), Some("second"));
        let _ = update(&mut app, Message::ToggleMapMenu("second".into()));
        assert_eq!(app.map_menu_open, None);
    }

    fn make_app(n_versions: usize) -> App {
        let versions: Vec<launcher::download::VersionInfo> = (0..n_versions)
            .map(|i| launcher::download::VersionInfo {
                id: format!("1.20.{}", i),
                kind: if i % 5 == 0 { "snapshot" } else { "release" }.to_string(),
                url: format!("https://example.com/v{}.json", i),
                release_time: "2026-01-01T00:00:00Z".to_string(),
                local: false,
                loader: None,
                game_version: None,
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
            show_launch_account: false,
            show_launch_maintenance: false,
            show_launch_advanced: false,
            show_manage_favorites: false,
            config: AppConfig::default(),
            versions,
            filtered: Vec::new(),
            version_tab: VersionTab::Release,
            selected_version: selected,
            profiles,
            selected_profile: None,
            friends: launcher::friends::FriendSnapshot::default(),
            friends_loading: false,
            friend_error: None,
            new_profile_name: String::new(),
            selected_java: java_list.first().cloned(),
            java_list,
            downloading: false,
            progress: None,
            launching: false,
            game_running: false,
            active_managed: false,
            enhanced_started_at: None,
            enhanced_startup_duration: None,
            enhanced_phase: None,
            ms_code_input: String::new(),
            ms_logging_in: false,
            ms_auth_session: None,
            confirm_reset: false,
            confirm_empty_trash: false,
            root_changed: false,
            log_writer: None,
            local_versions: Vec::new(),
            local_choices: Vec::new(),
            releases: Vec::new(),
            fabric_game: None,
            fabric_loaders: Vec::new(),
            fabric_loader: None,
            fabric_loading: false,
            fabric_installing: false,
            pending_select: None,
            saves: Vec::new(),
            pending_creations: Vec::new(),
            map_import_path: String::new(),
            pending_map_restore: None,
            active_map: None,
            renaming_save: None,
            save_name_input: String::new(),
            pending_save_delete: None,
            pending_world_enter: None,
            map_menu_open: None,
            server_address: String::new(),
            server_profile_name: String::new(),
            server_ping_status: HashMap::new(),
            server_running: false,
            server_ready: false,
            server_busy: false,
            server_logs: Vec::new(),
            server_port_input: "25565".to_string(),
            pending_server_join: false,
            new_world_name: "新的世界".to_string(),
            new_world_seed: String::new(),
            new_world_flat: false,
            new_world_cheats: false,
            new_world_gamemode: "survival".to_string(),
            new_world_difficulty: "normal".to_string(),
            new_world_keep_inventory: false,
            new_world_mob_griefing: true,
            new_world_daylight_cycle: true,
            new_world_weather_cycle: true,
            new_world_hostile_mobs: true,
            new_world_pvp: true,
            new_world_creating: false,
            window_width_input: "1280".to_string(),
            window_height_input: "720".to_string(),
            eula_accepted: true,
            eula_sha1: None,
            eula_checking: false,
            eula_required: false,
            eula_text: None,
            eula_source: None,
            eula_warning: None,
            eula_loading: false,
            eula_error: None,
            eula_scrolled_to_bottom: false,
            eula_checked: false,
            pending_action: None,
            mods: Vec::new(),
            mods_version: None,
            pending_mod_delete: None,
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
        println!(
            "│ 动画中每帧 update+view·启动页(800)  {:>10.1}（最差 {:.1}）",
            anim_launch_big, anim_launch_big_max
        );
        println!(
            "│ 动画中每帧 update+view·启动页(10)   {:>10.1}",
            anim_launch_small
        );
        println!(
            "│ 动画中每帧 update+view·设置页(800)  {:>10.1}",
            anim_settings_big
        );
        println!(
            "│ 动画中每帧 update+view·首页(800)    {:>10.1}",
            anim_home_big
        );
        println!(
            "│ 空闲每帧 update+view·启动页(800)    {:>10.1}",
            idle_launch_big
        );
        println!(
            "│ 空闲每帧 update+view·设置页(800)    {:>10.1}",
            idle_settings_big
        );
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

    #[test]
    fn eula_modal_builds_with_long_text() {
        let mut app = make_app(2);
        app.eula_required = true;
        app.eula_text = Some("EULA line 1\nEULA line 2\n".repeat(400));
        app.eula_source = Some("https://www.minecraft.net/en-us/eula".to_string());
        app.eula_scrolled_to_bottom = true;
        app.eula_checked = true;
        let _ = view(&app);
    }
}
