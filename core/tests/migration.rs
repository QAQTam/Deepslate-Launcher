use deepslate_core::{config::AppConfig, maps, migration};
use std::{fs, io::Write, path::Path};
use zip::{write::SimpleFileOptions, ZipWriter};

fn jar(path: &Path, name: &str, data: &str) {
    let file = fs::File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    zip.start_file(name, SimpleFileOptions::default()).unwrap();
    zip.write_all(data.as_bytes()).unwrap();
    zip.finish().unwrap();
}
#[test]
fn discovers_relocated_worlds_and_merged_fabric_and_imports_idempotently() {
    let base = std::env::temp_dir().join(format!("deepslate-migration-{}", uuid::Uuid::new_v4()));
    let mc = base.join("random/location/.minecraft");
    let instance = mc.join("versions/custom-fabric");
    fs::create_dir_all(instance.join("saves/世界")).unwrap();
    fs::create_dir_all(mc.join("saves/shared")).unwrap();
    let fixture = include_bytes!("fixtures/level_26_3.dat");
    fs::write(instance.join("saves/世界/level.dat"), fixture).unwrap();
    fs::write(mc.join("saves/shared/level.dat"), fixture).unwrap();
    fs::write(instance.join("custom-fabric.json"),r#"{"id":"custom-fabric","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient","libraries":[{"name":"net.fabricmc:fabric-loader:0.20.1"}]}"#).unwrap();
    jar(
        &instance.join("custom-fabric.jar"),
        "version.json",
        r#"{"name":"26.3","world_version":5023,"protocol_version":777}"#,
    );
    let scan = migration::scan(&base).unwrap();
    assert_eq!(scan.directories, vec![fs::canonicalize(&mc).unwrap()]);
    assert_eq!(scan.worlds.len(), 2);
    assert!(scan
        .worlds
        .iter()
        .all(|w| w.protocol == Some(777) && w.data_version == Some(5023)));
    assert_eq!(scan.versions[0].loader.as_deref(), Some("0.20.1"));
    let cfg = AppConfig {
        root_dir: base.join("target").to_string_lossy().into_owned(),
        ..Default::default()
    };
    let id = migration::import_world(&cfg, &scan.worlds[0].path).unwrap();
    assert!(id.is_some());
    assert!(migration::import_world(&cfg, &scan.worlds[0].path)
        .unwrap()
        .is_none());
    assert_eq!(maps::list(&cfg).unwrap().len(), 1);
    fs::remove_dir_all(base).unwrap();
}
#[test]
fn imports_fabric_mods_without_overwriting_or_replacing_ids() {
    let base =
        std::env::temp_dir().join(format!("deepslate-mod-migration-{}", uuid::Uuid::new_v4()));
    let src = base.join("src");
    let dst = base.join("dst");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&dst).unwrap();
    jar(
        &src.join("one.jar"),
        "fabric.mod.json",
        r#"{"id":"example","version":"1"}"#,
    );
    jar(
        &dst.join("old.jar"),
        "fabric.mod.json",
        r#"{"id":"example","version":"0"}"#,
    );
    jar(
        &src.join("two.jar"),
        "fabric.mod.json",
        r#"{"id":"other","version":"1"}"#,
    );
    let (n, skipped) = migration::import_mods(&src, &dst).unwrap();
    assert_eq!(n, 1);
    assert_eq!(skipped.len(), 1);
    assert!(dst.join("old.jar").exists());
    assert!(dst.join("two.jar").exists());
    assert_eq!(migration::import_mods(&src, &dst).unwrap().0, 0);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn config_merge_preserves_existing_files() {
    let base = std::env::temp_dir().join(format!(
        "deepslate-config-migration-{}",
        uuid::Uuid::new_v4()
    ));
    let src = base.join("source");
    let dst = base.join("target");
    fs::create_dir_all(src.join("nested")).unwrap();
    fs::create_dir_all(dst.join("nested")).unwrap();
    fs::write(src.join("nested/existing"), b"source").unwrap();
    fs::write(src.join("nested/new"), b"new").unwrap();
    fs::write(dst.join("nested/existing"), b"original").unwrap();
    let (n, skipped) = migration::import_config(&src, &dst).unwrap();
    assert_eq!(n, 1);
    assert_eq!(skipped.len(), 1);
    assert_eq!(fs::read(dst.join("nested/existing")).unwrap(), b"original");
    assert_eq!(fs::read(dst.join("nested/new")).unwrap(), b"new");
    fs::remove_dir_all(base).unwrap();
}

#[cfg(unix)]
#[test]
fn world_with_symlink_is_not_silently_truncated() {
    use std::os::unix::fs::symlink;
    let base = std::env::temp_dir().join(format!("deepslate-link-world-{}", uuid::Uuid::new_v4()));
    let world = base.join("world");
    fs::create_dir_all(&world).unwrap();
    fs::write(
        world.join("level.dat"),
        include_bytes!("fixtures/level_26_3.dat"),
    )
    .unwrap();
    fs::write(base.join("outside"), b"data").unwrap();
    symlink(base.join("outside"), world.join("extra")).unwrap();
    let cfg = AppConfig {
        root_dir: base.join("target").to_string_lossy().into_owned(),
        ..Default::default()
    };
    assert!(migration::import_world(&cfg, &world)
        .unwrap_err()
        .contains("符号链接"));
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn finds_world_inside_game_at_outer_depth_limit_and_excludes_destination() {
    let base = std::env::temp_dir().join(format!("deepslate-deep-scan-{}", uuid::Uuid::new_v4()));
    let mut parent = base.clone();
    for i in 0..7 {
        parent.push(format!("d{i}"));
    }
    let mc = parent.join(".minecraft");
    let saves = mc.join("versions/custom/saves/deep-world");
    fs::create_dir_all(&saves).unwrap();
    let fixture = include_bytes!("fixtures/level_26_3.dat");
    fs::write(saves.join("level.dat"), fixture).unwrap();
    let shared = mc.join("saves/shared");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("level.dat"), fixture).unwrap();
    let destination = base.join("maps");
    fs::create_dir_all(destination.join("existing")).unwrap();
    fs::write(destination.join("existing/level.dat"), fixture).unwrap();
    let result = migration::scan_excluding(&base, &[destination]).unwrap();
    assert_eq!(result.worlds.len(), 2);
    assert!(result
        .worlds
        .iter()
        .any(|w| w.path == fs::canonicalize(&saves).unwrap()));
    assert!(result
        .worlds
        .iter()
        .any(|w| w.path == fs::canonicalize(&shared).unwrap()));
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn scan_honors_cancellation() {
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    let root = std::env::temp_dir();
    assert!(migration::scan_cancellable(&root, &[], Some(&cancelled))
        .unwrap_err()
        .contains("取消"));
}

#[test]
fn resolves_multilevel_fabric_client_inheritance() {
    let base =
        std::env::temp_dir().join(format!("deepslate-inherited-scan-{}", uuid::Uuid::new_v4()));
    let mc = base.join(".minecraft");
    for id in ["custom", "intermediate", "26.3"] {
        fs::create_dir_all(mc.join("versions").join(id)).unwrap();
    }
    fs::write(mc.join("versions/custom/custom.json"),r#"{"id":"custom","jar":"custom","inheritsFrom":"intermediate","libraries":[{"name":"net.fabricmc:fabric-loader:0.20.1"}]}"#).unwrap();
    fs::write(
        mc.join("versions/intermediate/intermediate.json"),
        r#"{"id":"intermediate","inheritsFrom":"26.3"}"#,
    )
    .unwrap();
    fs::write(mc.join("versions/26.3/26.3.json"), r#"{"id":"26.3"}"#).unwrap();
    jar(
        &mc.join("versions/26.3/26.3.jar"),
        "version.json",
        r#"{"name":"26.3","world_version":5023,"protocol_version":777}"#,
    );
    let result = migration::scan(&base).unwrap();
    let version = result.versions.iter().find(|v| v.id == "custom").unwrap();
    assert_eq!(version.game_name.as_deref(), Some("26.3"));
    assert_eq!(version.protocol, Some(777));
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn mod_directory_matches_actual_normal_runtime_for_legacy_configs() {
    let mut config = AppConfig::default();
    config.root_dir = "/tmp/deepslate-example".into();
    for isolated in [false, true] {
        config.version_isolation = isolated;
        config.enhanced_experience = false;
        assert_eq!(
            deepslate_core::mods::mods_dir(&config, "26.3"),
            deepslate_core::runtime::paths(&config)
                .instance_dir
                .join("mods")
        );
    }
}
