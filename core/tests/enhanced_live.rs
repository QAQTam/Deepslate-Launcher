//! Manual desktop integration test. Uses a disposable root supplied by the caller.
use core::events::CoreEvent;
use deepslate_core as core;
use std::time::Duration;

fn verify_created(config: &core::AppConfig, id: &str) {
    let world = core::maps::refresh_after_play(config, id).unwrap();
    assert_eq!(world.name, "Bridge Test Create");
    assert_eq!(world.game_mode, "creative");
    assert!(world.cheats);
    let path = core::maps::root(config)
        .join(id)
        .join("data/minecraft/world_gen_settings.dat");
    let level = core::nbt::NbtFile::read_gzip(&path).unwrap();
    assert_eq!(
        level.root.get_path(&["data", "seed"]),
        Some(&core::nbt::Tag::Long(42))
    );
    assert_eq!(
        level.root.get_path(&[
            "data",
            "dimensions",
            "minecraft:overworld",
            "generator",
            "type"
        ]),
        Some(&core::nbt::Tag::String("minecraft:flat".into()))
    );
    assert!(core::maps::list(config)
        .unwrap()
        .iter()
        .any(|map| map.id == id));
    assert_eq!(world.manifest.generator.unwrap().kind, "flat");
}

#[test]
#[ignore = "需要已生成的临时测试世界"]
fn verify_saved_creation() {
    let mut config = core::AppConfig::default();
    config.root_dir = std::env::var("DEEPSLATE_TEST_ROOT").unwrap();
    verify_created(&config, &std::env::var("DEEPSLATE_TEST_WORLD").unwrap());
}

#[tokio::test]
#[ignore = "需要显示环境、联网和一次性的 26.3 测试世界"]
async fn existing_world_saves_and_client_exits() {
    let root = std::env::var("DEEPSLATE_TEST_ROOT").expect("DEEPSLATE_TEST_ROOT");
    let mut config = core::AppConfig::default();
    config.root_dir = root;
    config.enhanced_experience = true;
    let paths =
        core::runtime::ensure_for_mode(&config, core::runtime::RuntimeMode::Enhanced).unwrap();
    let server = std::env::var("DEEPSLATE_TEST_SERVER").ok();
    let create = server.is_none() && std::env::var_os("DEEPSLATE_TEST_CREATE").is_some();
    let id = if create {
        core::maps::new_id()
    } else {
        "test-world".into()
    };
    let old_level = std::fs::read(paths.maps_dir.join(&id).join("level.dat")).ok();
    if !create && server.is_none() {
        assert!(old_level.is_some());
    }
    let target = if let Some(address) = &server {
        core::bridge::ManagedTarget::JoinServer(address.clone())
    } else if create {
        core::bridge::ManagedTarget::Create {
            id: id.clone(),
            name: "Bridge Test Create".into(),
            seed: "42".into(),
            mode: "creative".into(),
            preset: "flat".into(),
            cheats: true,
        }
    } else {
        core::bridge::ManagedTarget::Open(id.clone())
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let installed = core::enhanced::install(&config, tx.clone()).await.unwrap();
    let opts = core::launch::LaunchOptions {
        version: installed.lock.version_id,
        game_dir: paths.game_dir,
        instance_dir: paths.instance_dir,
        map_config: Some(config.clone()),
        username: "BridgeTest".into(),
        uuid: core::profiles::uuid_from_name("BridgeTest"),
        runtime_dir: "/usr/lib/jvm/java-25-openjdk-amd64".into(),
        auth_method: core::auth::AuthMethod::Offline,
        access_token: None,
        download_concurrency: 32,
        quick_play: None,
        managed_world: Some(target),
        branding: None,
        apply_game_defaults: true,
        disable_multiplayer: false,
        disable_chat: false,
        fullscreen: false,
        window_width: 854,
        window_height: 480,
        graphics_backend: "default".into(),
    };
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = core::launch::launch(opts, tx)
            .await
            .map_err(|e| e.to_string());
        let _ = done_tx.send(result);
    });
    let result = tokio::time::timeout(Duration::from_secs(240), async {
        let mut entered = false;
        let mut closed = false;
        loop {
            tokio::select! {
                done = &mut done_rx => {
                    done.map_err(|e| e.to_string())??;
                    if !entered || !closed { return Err(format!("incomplete lifecycle: entered={entered}, closed={closed}")); }
                    return Ok::<(), String>(());
                }
                event = rx.recv() => match event {
                    Some(CoreEvent::BridgeState { state, message }) => {
                        eprintln!("bridge: {state}: {message:?}");
                        if state == "failed" { return Err(message.unwrap_or_default()); }
                        if state == "entered" && !entered {
                            entered = true;
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            core::launch::quit_managed_game().await?;
                        }
                        if state == "closed" { closed = true; }
                    }
                    Some(CoreEvent::GameLog { line, .. }) => eprintln!("game: {line}"),
                    Some(_) => {},
                    None => {
                        (&mut done_rx).await.map_err(|e| e.to_string())??;
                        if !entered || !closed { return Err(format!("incomplete lifecycle: entered={entered}, closed={closed}")); }
                        return Ok(());
                    },
                }
            }
        }
    }).await;
    if !matches!(result, Ok(Ok(()))) {
        let _ = core::launch::kill_game().await;
    }
    result
        .expect("desktop lifecycle timed out")
        .expect("desktop lifecycle failed");
    if server.is_none() {
        core::maps::refresh_after_play(&config, &id).unwrap();
        let new_level = std::fs::read(paths.maps_dir.join(&id).join("level.dat")).unwrap();
        assert_ne!(Some(new_level), old_level);
    }
    if create {
        verify_created(&config, &id);
    }
}
