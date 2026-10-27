use super::*;

/// 读取 `--key value` 形式的命令行参数。
fn cli_arg(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// CLI 模式共享配置：支持 `--root` / `--java` 覆盖，便于无 GUI 测试。
fn cli_config(args: &[String]) -> AppConfig {
    let mut config = launcher::config::load().unwrap_or_default();
    if let Some(root) = cli_arg(args, "--root") {
        config.root_dir = root;
    }
    if let Some(java) = cli_arg(args, "--java") {
        config.java_path = Some(java);
    }
    config.disable_multiplayer = args.iter().any(|a| a == "--disable-multiplayer");
    config.disable_chat = args.iter().any(|a| a == "--disable-chat");
    config.fullscreen = args.iter().any(|a| a == "--fullscreen");
    if let Some(width) = cli_arg(args, "--width").and_then(|v| v.parse::<u32>().ok()) {
        config.window_width = width;
    }
    if let Some(height) = cli_arg(args, "--height").and_then(|v| v.parse::<u32>().ok()) {
        config.window_height = height;
    }
    if let Some(backend) = cli_arg(args, "--graphics-backend") {
        config.graphics_backend = backend;
    }
    config
}

/// CLI 是调试/自动化路径，必须每次显式 `--accept-eula`：它不会像 GUI 那样
/// 在启动时检查协议哈希，因此不能拿旧 `eula.txt` 静默放行。
/// 该参数仍会写入与 GUI 相同的 `<root>/eula.txt`，而不是只对本次进程生效。
fn cli_require_eula(args: &[String], config: &AppConfig) -> bool {
    if !args.iter().any(|a| a == "--accept-eula") {
        eprintln!(
            "需要 --accept-eula 表示同意 Minecraft EULA（会写入 {}）",
            launcher::eula::marker_path(config).display()
        );
        return false;
    }
    if let Err(error) = launcher::eula::accept(config) {
        eprintln!("{error}");
        return false;
    }
    true
}

fn cli_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build CLI runtime")
}

fn cli_quick_play(args: &[String]) -> Option<launcher::launch::QuickPlayTarget> {
    if let Some(id) = cli_arg(args, "--quick-save").or_else(|| cli_arg(args, "--quick-map")) {
        Some(launcher::launch::QuickPlayTarget::Singleplayer(id))
    } else {
        cli_arg(args, "--quick-server").map(launcher::launch::QuickPlayTarget::Multiplayer)
    }
}

async fn cli_progress_printer(mut rx: mpsc::Receiver<CoreEvent>) {
    let mut last = u32::MAX;
    while let Some(event) = rx.recv().await {
        if let CoreEvent::DownloadProgress(p) = event {
            if p.percent != last {
                eprintln!(
                    "[{:3}%] {}/{} {}",
                    p.percent, p.current, p.total, p.current_file
                );
                last = p.percent;
            }
        }
    }
}

/// `--install-fabric <mc-version> [--loader <fabric-version>]`：
/// 无 GUI 安装 Fabric，用于验证与排障；省略 loader 时取该版本第一个稳定版。
pub(super) fn run_cli_install_fabric(args: &[String]) -> i32 {
    let Some(game) = cli_arg(args, "--install-fabric") else {
        eprintln!("用法: deepslate-iced --install-fabric <mc-version> [--loader <fabric-version>] [--root <dir>]");
        return 2;
    };
    let config = cli_config(args);
    let requested_loader = cli_arg(args, "--loader");
    let game_dir = launcher::config::resolve_game_dir(&config);
    let concurrency = config.download_concurrency;

    let rt = cli_runtime();
    rt.block_on(async move {
        let loader = match requested_loader {
            Some(loader) => loader,
            None => match launcher::fabric::list_loaders(game.clone()).await {
                Ok(loaders) => match launcher::fabric::default_loader(&loaders) {
                    Some(loader) => loader.version,
                    None => {
                        eprintln!("Fabric 没有为 Minecraft {game} 提供加载器");
                        return 1;
                    }
                },
                Err(e) => {
                    eprintln!("获取 Fabric 加载器列表失败: {e}");
                    return 1;
                }
            },
        };

        let version_id = launcher::fabric::version_id(&game, &loader);
        let opts = launcher::fabric::InstallOptions {
            game_version: game.clone(),
            loader_version: loader.clone(),
            game_dir: game_dir.clone(),
            instance_dir: launcher::config::get_version_root(&config, &version_id),
            concurrency,
        };

        let (tx, rx) = mpsc::channel::<CoreEvent>(128);
        let printer = tokio::spawn(cli_progress_printer(rx));
        let result = launcher::fabric::install(opts, tx).await;
        let _ = printer.await;
        match result {
            Ok(result) => {
                println!("version_id={}", result.version_id);
                println!(
                    "game={} loader={} libraries={} downloaded={}",
                    result.game_version,
                    result.loader_version,
                    result.libraries_total,
                    result.libraries_downloaded
                );
                0
            }
            Err(e) => {
                eprintln!("Fabric 安装失败: {e}");
                1
            }
        }
    })
}

/// `--print-command <version-id>`：构造并打印启动计划（会补齐缺失的库），不启动进程。
pub(super) fn run_cli_print_command(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--print-command") else {
        eprintln!(
            "用法: deepslate-iced --print-command <version-id> [--root <dir>] [--java <java-home>]"
        );
        return 2;
    };
    let config = cli_config(args);
    let java_home = config.java_path.clone().or_else(|| {
        launcher::launch::scan_java()
            .ok()
            .and_then(|list| list.into_iter().next())
            .map(|j| j.path)
    });
    let Some(java_home) = java_home else {
        eprintln!("未找到 Java 运行时，请用 --java <java-home> 指定");
        return 1;
    };

    let rt = cli_runtime();
    rt.block_on(async move {
        let opts = launcher::launch::LaunchOptions {
            version: version.clone(),
            game_dir: launcher::runtime::paths(&config).game_dir,
            instance_dir: launcher::runtime::paths(&config).instance_dir,
            map_config: Some(config.clone()),
            username: "DeepslateTest".to_string(),
            uuid: launcher::profiles::uuid_from_name("DeepslateTest"),
            runtime_dir: std::path::PathBuf::from(&java_home),
            auth_method: launcher::auth::AuthMethod::Offline,
            access_token: None,
            download_concurrency: config.download_concurrency,
            quick_play: cli_quick_play(args),
            managed_world: None,
            branding: config.brand_title.then(|| config.brand_text.clone()),
            apply_game_defaults: config.apply_game_defaults,
            disable_multiplayer: config.disable_multiplayer,
            disable_chat: config.disable_chat,
            fullscreen: config.fullscreen,
            window_width: config.window_width,
            window_height: config.window_height,
            graphics_backend: config.graphics_backend.clone(),
        };
        let (tx, rx) = mpsc::channel::<CoreEvent>(128);
        let printer = tokio::spawn(cli_progress_printer(rx));
        if let Err(e) = launcher::runtime::ensure(&config) {
            eprintln!("准备运行时目录失败: {e}");
            drop(tx);
            let _ = printer.await;
            return 1;
        }
        let result = launcher::launch::prepare(&opts, &tx).await;
        drop(tx);
        let _ = printer.await;
        match result {
            Ok(plan) => {
                println!("java={}", plan.java.display());
                println!("main_class={}", plan.main_class);
                println!("version_id={}", plan.version_id);
                println!("version_type={}", plan.version_type);
                println!("natives_dir={}", plan.natives_dir.display());
                println!("classpath:");
                for p in &plan.classpath {
                    println!("  {}", p.display());
                }
                println!("jvm_args:");
                for a in &plan.jvm_args {
                    println!("  {a}");
                }
                println!("game_args:");
                for a in &plan.game_args {
                    println!("  {a}");
                }
                println!("command={}", plan.command_line());
                0
            }
            Err(e) => {
                eprintln!("准备启动失败: {e}");
                1
            }
        }
    })
}

/// `--launch <version-id>`：无 GUI 启动游戏，用于排障（日志直接写 stderr）。
pub(super) fn run_cli_launch(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--launch") else {
        eprintln!(
            "用法: deepslate-iced --launch <version-id> --accept-eula [--root <dir>] [--java <java-home>]"
        );
        return 2;
    };
    let config = cli_config(args);
    if !cli_require_eula(args, &config) {
        return 2;
    }
    let java_home = config.java_path.clone().or_else(|| {
        launcher::launch::scan_java()
            .ok()
            .and_then(|list| list.into_iter().next())
            .map(|j| j.path)
    });
    let Some(java_home) = java_home else {
        eprintln!("未找到 Java 运行时，请用 --java <java-home> 指定");
        return 1;
    };

    let opts = launcher::launch::LaunchOptions {
        version: version.clone(),
        game_dir: launcher::runtime::paths(&config).game_dir,
        instance_dir: launcher::runtime::paths(&config).instance_dir,
        map_config: Some(config.clone()),
        username: "DeepslateTest".to_string(),
        uuid: launcher::profiles::uuid_from_name("DeepslateTest"),
        runtime_dir: std::path::PathBuf::from(&java_home),
        auth_method: launcher::auth::AuthMethod::Offline,
        access_token: None,
        download_concurrency: config.download_concurrency,
        quick_play: cli_quick_play(args),
        managed_world: None,
        branding: config.brand_title.then(|| config.brand_text.clone()),
        apply_game_defaults: config.apply_game_defaults,
        disable_multiplayer: config.disable_multiplayer,
        disable_chat: config.disable_chat,
        fullscreen: config.fullscreen,
        window_width: config.window_width,
        window_height: config.window_height,
        graphics_backend: config.graphics_backend.clone(),
    };

    let rt = cli_runtime();
    rt.block_on(async move {
        let (tx, mut rx) = mpsc::channel::<CoreEvent>(256);
        let printer = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CoreEvent::BridgeState { state, message } => {
                        eprintln!("[bridge] {state}: {}", message.unwrap_or_default())
                    }
                    CoreEvent::GameLog { is_stderr, line } => {
                        eprintln!("{} {line}", if is_stderr { "E" } else { "O" });
                    }
                    CoreEvent::GameState(state) => eprintln!("[state] {state:?}"),
                    CoreEvent::ServerLog { is_stderr, line } => {
                        eprintln!("{} {line}", if is_stderr { "SE" } else { "SO" });
                    }
                    CoreEvent::ServerState(state) => eprintln!("[server] {state:?}"),
                    CoreEvent::DownloadProgress(_) | CoreEvent::AuthProgress(_) => {}
                }
            }
        });
        if let Err(e) = launcher::runtime::ensure(&config) {
            eprintln!("准备运行时目录失败: {e}");
            drop(tx);
            let _ = printer.await;
            return 1;
        }
        let result = launcher::launch::launch(opts, tx).await;
        let _ = printer.await;
        match result {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("启动失败: {e}");
                1
            }
        }
    })
}

/// `--list-saves <version-id>`：打印解析后的存档列表（含真实 26.3 level.dat）。
pub(super) fn run_cli_list_saves(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--list-saves") else {
        eprintln!("用法: deepslate-iced --list-saves <version-id> [--root <dir>]");
        return 2;
    };
    let config = cli_config(args);
    let dir = launcher::backup::version_saves_dir(&config, &version);
    println!("saves_dir={}", dir.display());
    let worlds = launcher::saves::list(&dir);
    println!("count={}", worlds.len());
    for world in worlds {
        println!(
            "id={}\tname={}\tmode={}\tversion={}\tlast={}\tsize={}\thardcore={}\tcheats={}",
            world.id,
            world.name,
            world.game_mode,
            world.version_name,
            world.last_played_text(),
            world.size_text(),
            world.hardcore,
            world.cheats
        );
    }
    0
}

/// `--list-maps`：列出启动器地图库（会确保 instance/saves -> maps）。
pub(super) fn run_cli_list_maps(args: &[String]) -> i32 {
    let config = cli_config(args);
    // 无 GUI 时也执行一次崩溃恢复，便于排障和验证。
    let _ = launcher::worldgen::cleanup_stale(&config);
    match launcher::runtime::ensure(&config).and_then(|_| launcher::maps::list(&config)) {
        Ok(maps) => {
            println!("maps_dir={}", launcher::maps::root(&config).display());
            println!("count={}", maps.len());
            for map in maps {
                println!(
                    "id={}\tname={}\tmode={}\tdifficulty={}\tversion={}\tlast={}\tsize={}\tsource={}\thardcore={}\tcheats={}",
                    map.id,
                    map.name,
                    map.game_mode,
                    map.difficulty,
                    map.version_name,
                    map.last_played_text(),
                    map.size_text(),
                    map.source_label(),
                    map.hardcore,
                    map.cheats
                );
            }
            0
        }
        Err(e) => {
            eprintln!("列出地图库失败: {e}");
            1
        }
    }
}

/// `--import-map <zip|dir>` / `--export-map <id> [--out <zip>]` / `--trash-map <id>`：
/// 无 GUI 的地图库操作。
pub(super) fn run_cli_map_op(args: &[String]) -> i32 {
    let config = cli_config(args);
    let result = if let Some(path) = cli_arg(args, "--import-map") {
        launcher::maps::import_path(&config, std::path::Path::new(&path))
            .map(|map| format!("imported {} ({})", map.name, map.id))
    } else if let Some(id) = cli_arg(args, "--export-map") {
        let out = cli_arg(args, "--out");
        launcher::maps::export_zip(&config, &id, out.as_ref().map(std::path::Path::new))
            .map(|path| format!("exported {id} -> {}", path.display()))
    } else if let Some(id) = cli_arg(args, "--trash-map") {
        launcher::maps::move_to_trash(&config, &id)
            .map(|path| format!("trashed {id} -> {}", path.display()))
    } else if let Some(id) = cli_arg(args, "--backup-map") {
        launcher::maps::backup(&config, &id).map(|name| format!("backed up {id} -> {name}"))
    } else if let Some(id) = cli_arg(args, "--list-map-backups") {
        launcher::maps::list_backups(&config, &id).map(|names| {
            if names.is_empty() {
                format!("no backups for {id}")
            } else {
                format!("backups for {id}: {}", names.join(", "))
            }
        })
    } else if let Some(id) = cli_arg(args, "--restore-map") {
        let name = cli_arg(args, "--backup").or_else(|| {
            launcher::maps::list_backups(&config, &id)
                .ok()
                .and_then(|names| names.into_iter().next())
        });
        match name {
            Some(name) => launcher::maps::restore_backup(&config, &id, &name)
                .map(|()| format!("restored {id} from {name}")),
            None => Err("没有可用的地图备份".to_string()),
        }
    } else {
        eprintln!("缺少 --import-map / --export-map / --trash-map / --backup-map / --list-map-backups / --restore-map");
        return 2;
    };
    match result {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("地图库操作失败: {e}");
            1
        }
    }
}

/// `--generate-map <version-id> --accept-eula ...`：后台服务端生成世界并放入地图库。
pub(super) fn run_cli_generate_map(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--generate-map") else {
        eprintln!("用法: deepslate-iced --generate-map <version-id> --accept-eula [--name <name>] [--seed <seed>] [--flat] [--cheats] [--gamemode survival|creative] [--root <dir>]");
        return 2;
    };
    let config = cli_config(args);
    let java_home = config.java_path.clone().or_else(|| {
        launcher::launch::scan_java()
            .ok()
            .and_then(|list| list.into_iter().next())
            .map(|j| j.path)
    });
    let Some(java_home) = java_home else {
        eprintln!("未找到 Java 运行时，请用 --java <java-home> 指定");
        return 1;
    };
    if !cli_require_eula(args, &config) {
        return 2;
    }
    let spec = launcher::worldgen::WorldSpec {
        name: cli_arg(args, "--name").unwrap_or_else(|| "新的世界".to_string()),
        seed: cli_arg(args, "--seed").unwrap_or_default(),
        world_type: if args.iter().any(|a| a == "--flat") {
            launcher::server::WorldType::Flat
        } else if args.iter().any(|a| a == "--large-biomes") {
            launcher::server::WorldType::LargeBiomes
        } else if args.iter().any(|a| a == "--amplified") {
            launcher::server::WorldType::Amplified
        } else {
            launcher::server::WorldType::Normal
        },
        game_mode: cli_arg(args, "--gamemode").unwrap_or_else(|| "survival".to_string()),
        cheats: args.iter().any(|a| a == "--cheats"),
    };

    let rt = cli_runtime();
    rt.block_on(async move {
        let (tx, mut rx) = mpsc::channel::<CoreEvent>(256);
        let printer = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CoreEvent::ServerLog { is_stderr, line } => {
                        eprintln!("{} {line}", if is_stderr { "SE" } else { "SO" });
                    }
                    CoreEvent::DownloadProgress(p) => eprintln!("[download] {}%", p.percent),
                    _ => {}
                }
            }
        });
        let result = launcher::worldgen::generate(
            &config,
            &version,
            java_home.into(),
            config.server_memory_mb.clamp(1024, 4096),
            &spec,
            &tx,
        )
        .await;
        drop(tx);
        let _ = printer.await;
        match result {
            Ok(map) => {
                println!("map_id={}", map.id);
                println!("name={}", map.name);
                0
            }
            Err(e) => {
                eprintln!("生成地图失败: {e}");
                1
            }
        }
    })
}

/// `--list-mods <version-id>`：打印 mods/ 里的 Fabric 元数据与启用状态。
pub(super) fn run_cli_list_mods(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--list-mods") else {
        eprintln!("用法: deepslate-iced --list-mods <version-id> [--root <dir>]");
        return 2;
    };
    let config = cli_config(args);
    let dir = launcher::mods::mods_dir(&config, &version);
    if let Err(e) = launcher::mods::ensure_dir(&dir) {
        eprintln!("{e}");
        return 1;
    }
    let mods = launcher::mods::list(&dir);
    println!("mods_dir={}", dir.display());
    println!("count={}", mods.len());
    for m in mods {
        println!(
            "enabled={}\tname={}\tversion={}\tid={}\tfile={}\tsize={}",
            m.enabled,
            m.name,
            m.version,
            m.id,
            m.file_name,
            m.size_text()
        );
    }
    0
}

/// `--host-server <version-id> --accept-eula [--port N] [--memory MB]`：
/// 无 GUI 启动本地服务端，就绪后自动 stop，便于验证与排障。
pub(super) fn run_cli_host_server(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--host-server") else {
        eprintln!("用法: deepslate-iced --host-server <version-id> --accept-eula [--port N] [--memory MB] [--root <dir>]");
        return 2;
    };
    let config = cli_config(args);
    let java_home = config.java_path.clone().or_else(|| {
        launcher::launch::scan_java()
            .ok()
            .and_then(|list| list.into_iter().next())
            .map(|j| j.path)
    });
    let Some(java_home) = java_home else {
        eprintln!("未找到 Java 运行时，请用 --java <java-home> 指定");
        return 1;
    };
    if !cli_require_eula(args, &config) {
        return 2;
    }
    let port = cli_arg(args, "--port")
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(config.server_port);
    let memory = cli_arg(args, "--memory")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(config.server_memory_mb);
    let opts = launcher::server::ServerOptions::new(
        &config,
        &version,
        std::path::PathBuf::from(&java_home),
        memory,
        port,
        config.server_online_mode,
        true,
    );

    let rt = cli_runtime();
    rt.block_on(async move {
        let (tx, mut rx) = mpsc::channel::<CoreEvent>(256);
        if let Err(e) = launcher::server::start(opts, tx).await {
            eprintln!("启动服务端失败: {e}");
            return 1;
        }
        let mut ready = false;
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(180), rx.recv()).await;
            match event {
                Ok(Some(CoreEvent::ServerLog { is_stderr, line })) => {
                    eprintln!("{} {line}", if is_stderr { "SE" } else { "SO" });
                }
                Ok(Some(CoreEvent::ServerState(state))) => {
                    eprintln!("[server] {state:?}");
                    match state {
                        launcher::events::ServerState::Ready { port } => {
                            if !ready {
                                ready = true;
                                println!("READY 127.0.0.1:{port}");
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                if let Err(e) = launcher::server::stop().await {
                                    eprintln!("停止服务端失败: {e}");
                                    return 1;
                                }
                            }
                        }
                        launcher::events::ServerState::Stopped { exit_code } => {
                            println!("STOPPED exit={exit_code}");
                            return 0;
                        }
                        _ => {}
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    eprintln!("事件通道关闭");
                    return 1;
                }
                Err(_) => {
                    eprintln!("等待服务端就绪超时");
                    let _ = launcher::server::stop().await;
                    return 1;
                }
            }
        }
    })
}

/// `--create-world <version-id> --accept-eula [--name ...] [--seed ...] [--flat] [--cheats]`：
/// 兼容旧参数：后台服务端生成世界并纳入地图库。
pub(super) fn run_cli_create_world(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--create-world") else {
        eprintln!("用法: deepslate-iced --create-world <version-id> --accept-eula [--name <name>] [--seed <seed>] [--flat] [--cheats] [--gamemode survival|creative] [--root <dir>]");
        return 2;
    };
    let config = cli_config(args);
    let java_home = config.java_path.clone().or_else(|| {
        launcher::launch::scan_java()
            .ok()
            .and_then(|list| list.into_iter().next())
            .map(|j| j.path)
    });
    let Some(java_home) = java_home else {
        eprintln!("未找到 Java 运行时，请用 --java <java-home> 指定");
        return 1;
    };
    if !cli_require_eula(args, &config) {
        return 2;
    }
    let spec = launcher::worldgen::WorldSpec {
        name: cli_arg(args, "--name").unwrap_or_else(|| "新的世界".to_string()),
        seed: cli_arg(args, "--seed").unwrap_or_default(),
        world_type: if args.iter().any(|a| a == "--flat") {
            launcher::server::WorldType::Flat
        } else {
            launcher::server::WorldType::Normal
        },
        game_mode: cli_arg(args, "--gamemode").unwrap_or_else(|| "survival".to_string()),
        cheats: args.iter().any(|a| a == "--cheats"),
    };

    let rt = cli_runtime();
    rt.block_on(async move {
        let (tx, mut rx) = mpsc::channel::<CoreEvent>(256);
        let printer = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CoreEvent::ServerLog { is_stderr, line } => {
                        eprintln!("{} {line}", if is_stderr { "SE" } else { "SO" });
                    }
                    CoreEvent::DownloadProgress(p) => {
                        eprintln!("[download] {}%", p.percent);
                    }
                    _ => {}
                }
            }
        });
        let result = launcher::worldgen::generate(
            &config,
            &version,
            java_home.into(),
            config.server_memory_mb.clamp(1024, 4096),
            &spec,
            &tx,
        )
        .await;
        drop(tx);
        let _ = printer.await;
        match result {
            Ok(map) => {
                println!("map_id={}", map.id);
                println!("name={}", map.name);
                0
            }
            Err(e) => {
                eprintln!("生成世界失败: {e}");
                1
            }
        }
    })
}

/// `--rename-save/--copy-save/--trash-save <id> [--new-name <name>] --saves-version <version>`：
/// 无 GUI 的存档操作，也用于验证 NBT 写回是否被游戏接受。
pub(super) fn run_cli_save_op(args: &[String]) -> i32 {
    let Some(version) = cli_arg(args, "--saves-version") else {
        eprintln!("用法: deepslate-iced --saves-version <version-id> (--rename-save <id> --new-name <name> | --copy-save <id> | --trash-save <id>) [--root <dir>]");
        return 2;
    };
    let config = cli_config(args);
    let dir = launcher::backup::version_saves_dir(&config, &version);

    let _guard = match launcher::maps::lock_library(&config) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("无法操作存档: {error}");
            return 1;
        }
    };

    let result = if let Some(id) = cli_arg(args, "--rename-save") {
        let name = cli_arg(args, "--new-name").unwrap_or_default();
        launcher::saves::rename(&dir, &id, &name).map(|()| format!("renamed {id} -> {name}"))
    } else if let Some(id) = cli_arg(args, "--copy-save") {
        let name = cli_arg(args, "--new-name");
        launcher::saves::duplicate(&dir, &id, name.as_deref())
            .map(|new_id| format!("copied {id} -> {new_id}"))
    } else if let Some(id) = cli_arg(args, "--trash-save") {
        launcher::saves::move_to_trash(&dir, &id)
            .map(|path| format!("trashed {id} -> {}", path.display()))
    } else {
        eprintln!("缺少 --rename-save / --copy-save / --trash-save");
        return 2;
    };

    match result {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(e) => {
            eprintln!("存档操作失败: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_cli_save_operation_respects_library_lock() {
        let root = std::env::temp_dir().join(format!("deepslate-cli-lock-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().into();
        let paths = launcher::runtime::ensure(&config).unwrap();
        let world = paths.maps_dir.join("world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core/tests/fixtures/level_26_3.dat"),
            world.join("level.dat"),
        )
        .unwrap();
        let guard = launcher::maps::lock_library(&config).unwrap();
        let args = vec![
            "binary".into(),
            "--saves-version".into(),
            "x".into(),
            "--root".into(),
            config.root_dir.clone(),
            "--rename-save".into(),
            "world".into(),
            "--new-name".into(),
            "x".into(),
        ];
        assert_eq!(run_cli_save_op(&args), 1);
        drop(guard);
        std::fs::remove_dir_all(root).unwrap();
    }
}
