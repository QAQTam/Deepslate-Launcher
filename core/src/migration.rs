//! Read-only discovery and copy-based migration of Minecraft installations.
use crate::{config::AppConfig, maps, mods, saves};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone)]
pub struct GameVersion {
    pub id: String,
    pub game_name: Option<String>,
    pub data_version: Option<i32>,
    pub protocol: Option<i32>,
    pub loader: Option<String>,
    pub root: PathBuf,
    pub run_dir: PathBuf,
}
#[derive(Debug, Clone)]
pub struct FoundWorld {
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    pub data_version: Option<i32>,
    pub protocol: Option<i32>,
    pub associated_version: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub directories: Vec<PathBuf>,
    pub versions: Vec<GameVersion>,
    pub worlds: Vec<FoundWorld>,
    pub warnings: Vec<String>,
}

/// Scan a user-selected tree, bounded to avoid unbounded traversal. Symlinked directories
/// are not followed; users can select their target explicitly.
pub fn scan(root: &Path) -> Result<ScanResult, String> {
    scan_excluding(root, &[])
}

/// The destination can be excluded to avoid importing the application's own maps again.
pub fn scan_excluding(root: &Path, excluded: &[PathBuf]) -> Result<ScanResult, String> {
    scan_cancellable(root, excluded, None)
}

/// Cancellation is checked between directory reads; completed imports are unaffected.
pub fn scan_cancellable(
    root: &Path,
    excluded: &[PathBuf],
    cancel: Option<&AtomicBool>,
) -> Result<ScanResult, String> {
    if !root.is_dir() {
        return Err(format!("不是可读取的目录: {}", root.display()));
    }
    let excluded: Vec<PathBuf> = excluded
        .iter()
        .filter_map(|p| fs::canonicalize(p).ok())
        .collect();
    let mut result = ScanResult::default();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = HashSet::new();
    let mut count = 0;
    while let Some((dir, depth)) = stack.pop() {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("扫描已取消".into());
        }
        let canonical = match fs::canonicalize(&dir) {
            Ok(x) => x,
            Err(e) => {
                result.warnings.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        if excluded.iter().any(|path| canonical.starts_with(path))
            || !visited.insert(canonical.clone())
        {
            continue;
        }
        count += 1;
        if count > 100_000 {
            result
                .warnings
                .push("达到扫描目录上限，结果可能不完整".into());
            break;
        }
        let mut is_game_directory = false;
        if dir.file_name().is_some_and(|n| n == ".minecraft")
            || dir.join("versions").is_dir()
                && (dir.join("assets").is_dir()
                    || dir.join("libraries").is_dir()
                    || dir.join("saves").is_dir())
        {
            if !result.directories.contains(&canonical) {
                result.directories.push(canonical.clone());
                scan_versions(&canonical, &mut result);
                is_game_directory = true;
            }
        }
        if dir.join("level.dat").is_file() {
            match saves::parse_world(&dir) {
                Ok(w) => result.worlds.push(FoundWorld {
                    path: canonical,
                    name: w.name,
                    version: w.version_name,
                    data_version: w.data_version,
                    protocol: None,
                    associated_version: None,
                }),
                Err(e) => result.warnings.push(format!("{}: {e}", dir.display())),
            };
            continue;
        }
        if depth >= 8 {
            if is_game_directory {
                queue_saves(&canonical, &mut stack);
            }
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(v) => v,
            Err(e) => {
                result.warnings.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        for e in entries {
            let e = match e {
                Ok(e) => e,
                Err(e) => {
                    result.warnings.push(e.to_string());
                    continue;
                }
            };
            let Ok(t) = e.file_type() else { continue };
            if !t.is_dir() || t.is_symlink() {
                continue;
            }
            let name = e.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "assets"
                    | "libraries"
                    | "cache"
                    | "logs"
                    | "natives"
                    | "region"
                    | "node_modules"
                    | "target"
                    | ".git"
                    | "backups"
                    | ".deepslate-trash"
                    | ".staging"
            ) {
                continue;
            }
            stack.push((e.path(), depth + 1));
        }
        // Process known locations first, before a deeper generic traversal can
        // mark a saves directory visited at the outer scan depth limit.
        if is_game_directory {
            queue_saves(&canonical, &mut stack);
        }
    }
    result.directories.sort();
    result.worlds.sort_by(|a, b| a.path.cmp(&b.path));
    result
        .versions
        .sort_by(|a, b| a.root.cmp(&b.root).then(a.id.cmp(&b.id)));
    for world in &mut result.worlds {
        let mut matches = result.versions.iter().filter(|v| {
            v.data_version.is_some()
                && v.data_version == world.data_version
                && v.game_name.as_deref() == Some(world.version.as_str())
        });
        if let Some(v) = matches.next() {
            let protocol = v.protocol;
            world.associated_version = v.game_name.clone();
            if matches.all(|x| x.protocol == protocol) {
                world.protocol = protocol;
            }
        }
    }
    Ok(result)
}
fn queue_saves(root: &Path, stack: &mut Vec<(PathBuf, usize)>) {
    let shared = root.join("saves");
    if shared.is_dir()
        && !fs::symlink_metadata(&shared)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true)
    {
        stack.push((shared, 0));
    }
    if let Ok(entries) = fs::read_dir(root.join("versions")) {
        for entry in entries.flatten() {
            if !entry
                .file_type()
                .is_ok_and(|t| t.is_dir() && !t.is_symlink())
            {
                continue;
            }
            let saves = entry.path().join("saves");
            if saves.is_dir()
                && !fs::symlink_metadata(&saves)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(true)
            {
                stack.push((saves, 0));
            }
        }
    }
}
fn scan_versions(root: &Path, result: &mut ScanResult) {
    let Ok(entries) = fs::read_dir(root.join("versions")) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(s) = fs::read_to_string(dir.join(format!("{id}.json"))) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&s) else {
            continue;
        };
        let loader = json
            .get("libraries")
            .and_then(Value::as_array)
            .and_then(|a| {
                a.iter()
                    .filter_map(|x| {
                        x.get("name")?
                            .as_str()?
                            .strip_prefix("net.fabricmc:fabric-loader:")
                    })
                    .next()
            })
            .map(str::to_owned);
        let version = resolve_jar_version(root, &id, &mut HashSet::new());
        let run_dir = dir.clone();
        result.versions.push(GameVersion {
            id,
            game_name: version
                .as_ref()
                .and_then(|v| v.get("name")?.as_str().map(str::to_owned)),
            data_version: version
                .as_ref()
                .and_then(|v| v.get("world_version")?.as_i64())
                .and_then(|v| i32::try_from(v).ok()),
            protocol: version
                .as_ref()
                .and_then(|v| v.get("protocol_version")?.as_i64())
                .and_then(|v| i32::try_from(v).ok()),
            loader,
            root: root.to_path_buf(),
            run_dir,
        });
    }
}
fn resolve_jar_version(root: &Path, id: &str, seen: &mut HashSet<String>) -> Option<Value> {
    if seen.len() >= 32
        || crate::launch::validate_version_id(id).is_err()
        || !seen.insert(id.to_owned())
    {
        return None;
    }
    let dir = root.join("versions").join(id);
    if let Some(version) = read_jar_version(&dir.join(format!("{id}.jar"))) {
        return Some(version);
    }
    let json: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(format!("{id}.json"))).ok()?).ok()?;
    for key in ["jar", "inheritsFrom"] {
        if let Some(parent) = json.get(key).and_then(Value::as_str) {
            if let Some(version) = resolve_jar_version(root, parent, seen) {
                return Some(version);
            }
        }
    }
    None
}
fn read_jar_version(path: &Path) -> Option<Value> {
    let file = fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let entry = zip.by_name("version.json").ok()?;
    let mut bytes = Vec::new();
    entry.take(1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

// The map copier intentionally skips symlinks. Refuse incomplete imports instead of
// silently omitting world data; the transient Minecraft lock is never copied.
fn check_world_tree(path: &Path) -> Result<(), String> {
    let mut pending = vec![path.to_path_buf()];
    let mut count = 0usize;
    while let Some(dir) = pending.pop() {
        count += 1;
        if count > 100_000 {
            return Err("世界目录数量过多".into());
        }
        for entry in fs::read_dir(&dir).map_err(|e| format!("读取 {} 失败: {e}", dir.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() == "session.lock" {
                continue;
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err(format!(
                    "世界包含符号链接，请先复制为普通文件: {}",
                    entry.path().display()
                ));
            }
            if kind.is_dir() {
                pending.push(entry.path())
            }
        }
    }
    Ok(())
}

pub fn import_world(config: &AppConfig, path: &Path) -> Result<Option<String>, String> {
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if fs::canonicalize(maps::root(config)).is_ok_and(|root| canonical.starts_with(root)) {
        return Err("该世界已经位于目标地图库中".into());
    }
    check_world_tree(&canonical)?;
    let source = canonical.to_string_lossy().into_owned();
    maps::import_dir_once(config, &canonical, source).map(|item| item.map(|m| m.manifest.id))
}
/// Copy selected Fabric jars without overwriting existing files or duplicate mod IDs.
pub fn import_mods(source: &Path, target: &Path) -> Result<(usize, Vec<String>), String> {
    if !source.is_dir() {
        return Err(format!("模组目录不存在: {}", source.display()));
    }
    mods::ensure_dir(target)?;
    let existing = mods::list(target);
    let mut ids: HashSet<String> = existing
        .iter()
        .filter(|m| !m.id.is_empty())
        .map(|m| m.id.clone())
        .collect();
    let mut imported = 0;
    let mut skipped = Vec::new();
    for item in mods::list(source) {
        if fs::symlink_metadata(&item.path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(true)
        {
            skipped.push(format!("{}: 不导入符号链接", item.file_name));
            continue;
        }
        if !item.has_fabric_meta || item.metadata_error.is_some() {
            skipped.push(format!("{}: 无法识别 Fabric 元数据", item.file_name));
            continue;
        }
        if item.id.is_empty() || ids.contains(&item.id) {
            skipped.push(format!("{}: ID 缺失或重复", item.file_name));
            continue;
        }
        let dest = target.join(&item.file_name);
        let mut output = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
        {
            Ok(f) => f,
            Err(e) => {
                skipped.push(format!("{}: {e}", item.file_name));
                continue;
            }
        };
        let copied = (|| {
            let mut src = fs::File::open(&item.path)?;
            std::io::copy(&mut src, &mut output)?;
            output.sync_all()
        })();
        if let Err(e) = copied {
            drop(output);
            let _ = fs::remove_file(&dest);
            skipped.push(format!("{}: {e}", item.file_name));
            continue;
        }
        ids.insert(item.id);
        imported += 1;
    }
    Ok((imported, skipped))
}

/// Merge instance configuration without replacing existing files or following links.
pub fn import_config(source: &Path, target: &Path) -> Result<(usize, Vec<String>), String> {
    if !source.is_dir() {
        return Err(format!("配置目录不存在: {}", source.display()));
    }
    fn copy(
        source: &Path,
        target: &Path,
        count: &mut usize,
        skipped: &mut Vec<String>,
    ) -> Result<(), String> {
        if let Ok(meta) = fs::symlink_metadata(target) {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!("目标不是普通目录: {}", target.display()));
            }
        } else {
            fs::create_dir_all(target).map_err(|e| e.to_string())?;
        }
        for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let dest = target.join(entry.file_name());
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                skipped.push(format!("跳过链接 {}", path.display()));
                continue;
            }
            if kind.is_dir() {
                if let Err(e) = copy(&path, &dest, count, skipped) {
                    skipped.push(e);
                }
            } else if kind.is_file() {
                let mut out = match fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&dest)
                {
                    Ok(v) => v,
                    Err(e) => {
                        skipped.push(format!("{}: {e}", dest.display()));
                        continue;
                    }
                };
                let outcome = (|| {
                    let mut input = fs::File::open(&path)?;
                    std::io::copy(&mut input, &mut out)?;
                    out.sync_all()
                })();
                if let Err(e) = outcome {
                    drop(out);
                    let _ = fs::remove_file(&dest);
                    skipped.push(format!("{}: {e}", dest.display()));
                } else {
                    *count += 1;
                }
            }
        }
        Ok(())
    }
    let mut count = 0;
    let mut skipped = Vec::new();
    copy(source, target, &mut count, &mut skipped)?;
    Ok((count, skipped))
}
