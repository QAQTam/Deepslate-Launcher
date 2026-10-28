//! Read-only diagnostic for the same discovery used by the GUI.
use std::path::Path;
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--copy-instance") {
        let source = Path::new(args.get(2).ok_or("missing source instance")?);
        let target = Path::new(args.get(3).ok_or("missing target game directory")?);
        let (count, skipped) =
            deepslate_core::migration::import_mods(&source.join("mods"), &target.join("mods"))?;
        println!("mods copied={count}, skipped={skipped:?}");
        let (count, skipped) = deepslate_core::migration::import_config(
            &source.join("config"),
            &target.join("config"),
        )?;
        println!("config copied={count}, skipped={skipped:?}");
        return Ok(());
    }
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: migration_scan <search-root>")?;
    let scan = deepslate_core::migration::scan(Path::new(&root))?;
    for dir in scan.directories {
        println!("directory: {}", dir.display());
    }
    for version in scan.versions {
        println!(
            "version: {} game={:?} data={:?} protocol={:?} fabric={:?}",
            version.id, version.game_name, version.data_version, version.protocol, version.loader
        );
    }
    for world in scan.worlds {
        println!(
            "world: {} name={} version={} data={:?} protocol={:?}",
            world.path.display(),
            world.name,
            world.version,
            world.data_version,
            world.protocol
        );
    }
    for warning in scan.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
