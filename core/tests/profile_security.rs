use std::{fs, process::Command};

#[test]
fn isolated_profile_child() {
    if std::env::var_os("DEEPSLATE_TEST_CHILD").is_none() {
        return;
    }
    let store = deepslate_core::ProfileStore {
        profiles: vec![],
        last_used: None,
    };
    deepslate_core::profiles::save(&store).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = deepslate_core::profiles::path().unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    deepslate_core::config::reset_all().unwrap();
}

#[test]
fn reset_removes_current_profile_without_deleting_legacy() {
    let root = std::env::temp_dir().join(format!("profile-security-{}", uuid::Uuid::new_v4()));
    let exe_dir = root.join("bin");
    let game_root = root.join("game-root");
    let xdg = root.join("xdg");
    let legacy = xdg.join("obsidian-launcher/profiles.json");
    fs::create_dir_all(&exe_dir).unwrap();
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "legacy").unwrap();
    let exe = exe_dir.join("profile-test");
    fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
    let mut config = deepslate_core::AppConfig::default();
    config.root_dir = game_root.to_string_lossy().into();
    fs::write(
        exe_dir.join("config.json"),
        serde_json::to_string(&config).unwrap(),
    )
    .unwrap();
    fs::create_dir_all(game_root.join("config")).unwrap();
    fs::write(game_root.join("config/profiles.json"), "old secret").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            game_root.join("config/profiles.json"),
            fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        fs::set_permissions(game_root.join("config"), fs::Permissions::from_mode(0o777)).unwrap();
    }
    let status = Command::new(exe)
        .args(["--exact", "isolated_profile_child", "--nocapture"])
        .env("DEEPSLATE_TEST_CHILD", "1")
        .env("HOME", &root)
        .env("XDG_CONFIG_HOME", &xdg)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!game_root.join("config/profiles.json").exists());
    assert_eq!(fs::read_to_string(legacy).unwrap(), "legacy");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_profile_path_is_not_redirected_by_existing_root_profile() {
    let root = std::env::temp_dir().join(format!("profile-legacy-{}", uuid::Uuid::new_v4()));
    let exe_dir = root.join("bin");
    let game_root = root.join("game-root");
    let xdg = root.join("xdg");
    let legacy = xdg.join("obsidian-launcher/profiles.json");
    fs::create_dir_all(&exe_dir).unwrap();
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::create_dir_all(game_root.join("config")).unwrap();
    fs::write(game_root.join("config/profiles.json"), "unrelated").unwrap();
    fs::write(&legacy, "{}").unwrap();
    let exe = exe_dir.join("profile-test");
    fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
    let mut config = deepslate_core::AppConfig::default();
    config.version = 1;
    config.root_dir = game_root.to_string_lossy().into();
    fs::write(
        exe_dir.join("config.json"),
        serde_json::to_string(&config).unwrap(),
    )
    .unwrap();
    let status = Command::new(exe)
        .args(["--exact", "isolated_profile_child"])
        .env("DEEPSLATE_TEST_CHILD", "1")
        .env("HOME", &root)
        .env("XDG_CONFIG_HOME", &xdg)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!legacy.exists());
    assert_eq!(
        fs::read_to_string(game_root.join("config/profiles.json")).unwrap(),
        "unrelated"
    );
    fs::remove_dir_all(root).unwrap();
}
