//! 存档（世界）解析与管理。
//!
//! 只依赖 `level.dat`（gzip NBT）与目录结构，不改动区域文件：
//! - 列表：名称 / 游戏模式 / 难度 / 作弊 / 版本 / 最后游玩 / 大小 / 图标
//! - 重命名：只改 `Data.LevelName`（与原版 UI 行为一致，文件夹 id 不变）
//! - 复制：整目录复制并生成新的 LevelName / LastPlayed
//! - 删除：移动到 `saves/.deepslate-trash/`，可恢复，避免误删
//!
//! `--quickPlaySingleplayer <id>` 的 id 就是存档文件夹名，因此复制/删除后
//! 直接返回新的 id 即可用于快速进入。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::nbt::{NbtFile, Tag};

pub const TRASH_DIR: &str = ".deepslate-trash";

#[derive(Debug, Clone, PartialEq)]
pub struct WorldSummary {
    /// 存档文件夹名，也是 quickPlaySingleplayer 的参数
    pub id: String,
    pub name: String,
    /// survival / creative / adventure / spectator / unknown
    pub game_mode: String,
    pub difficulty: String,
    pub hardcore: bool,
    pub cheats: bool,
    /// 毫秒时间戳（0 表示未知）
    pub last_played: i64,
    pub version_name: String,
    pub data_version: Option<i32>,
    pub size_bytes: u64,
    pub icon: Option<PathBuf>,
    pub dir: PathBuf,
}

impl WorldSummary {
    pub fn last_played_text(&self) -> String {
        format_timestamp(self.last_played)
    }

    pub fn size_text(&self) -> String {
        human_size(self.size_bytes)
    }
}

fn format_timestamp(millis: i64) -> String {
    if millis <= 0 {
        return "未知".to_string();
    }
    // 不引入 chrono：用 UTC 秒数做一个稳定的「YYYY-MM-DD HH:MM」。
    let secs = millis / 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Howard Hinnant 的 civil_from_days（UTC）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn game_mode_name(value: i32) -> &'static str {
    match value {
        0 => "survival",
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "unknown",
    }
}

pub fn game_mode_label(mode: &str) -> &'static str {
    match mode {
        "survival" => "生存",
        "creative" => "创造",
        "adventure" => "冒险",
        "spectator" => "旁观",
        _ => "未知",
    }
}

/// 列出所有有效存档（含 level.dat 的目录），按最后游玩时间倒序。
pub fn list(saves_dir: &Path) -> Vec<WorldSummary> {
    let Ok(entries) = std::fs::read_dir(saves_dir) else {
        return Vec::new();
    };
    let mut worlds = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if id.starts_with('.') || !dir.join("level.dat").is_file() {
            continue;
        }
        match parse_world(&dir) {
            Ok(world) => worlds.push(world),
            Err(e) => eprintln!("跳过无法解析的存档 {id}: {e}"),
        }
    }
    worlds.sort_by(|a, b| {
        b.last_played
            .cmp(&a.last_played)
            .then_with(|| a.id.cmp(&b.id))
    });
    worlds
}

/// 解析单个存档目录。
pub fn parse_world(dir: &Path) -> Result<WorldSummary, String> {
    let id = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("非法存档目录: {}", dir.display()))?
        .to_string();
    let level_dat = dir.join("level.dat");
    let nbt = NbtFile::read_gzip(&level_dat)?;
    let data = nbt.root.get("Data").unwrap_or(&nbt.root);

    let name = data
        .get_string("LevelName")
        .unwrap_or_else(|| id.clone())
        .trim()
        .to_string();
    let name = if name.is_empty() { id.clone() } else { name };

    let game_type = data
        .get_i32("GameType")
        .or_else(|| data.get_path(&["Player", "GameType"]).and_then(tag_i32))
        .unwrap_or(0);
    let difficulty = data
        .get_path(&["difficulty_settings", "difficulty"])
        .and_then(|t| match t {
            Tag::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| data.get_i32("Difficulty").map(difficulty_name))
        .unwrap_or_else(|| "normal".to_string());
    let hardcore = data
        .get_path(&["difficulty_settings", "hardcore"])
        .and_then(tag_bool)
        .or_else(|| data.get_bool("hardcore"))
        .unwrap_or(false);
    let cheats = data.get_bool("allowCommands").unwrap_or(false);
    let last_played = data
        .get_i64("LastPlayed")
        .filter(|v| *v > 0)
        .or_else(|| {
            std::fs::metadata(&level_dat)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
        })
        .unwrap_or(0);
    let version_name = data
        .get_path(&["Version", "Name"])
        .and_then(|t| match t {
            Tag::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| data.get_string("Version"))
        .or_else(|| data.get_string("version_name"))
        .unwrap_or_default();
    let data_version = data.get_i32("DataVersion");

    let icon = dir.join("icon.png").is_file().then(|| dir.join("icon.png"));
    let size_bytes = dir_size(dir);

    Ok(WorldSummary {
        id,
        name,
        game_mode: game_mode_name(game_type).to_string(),
        difficulty,
        hardcore,
        cheats,
        last_played,
        version_name,
        data_version,
        size_bytes,
        icon,
        dir: dir.to_path_buf(),
    })
}

/// 生成/导入/恢复后的世界完整性门槛。
///
/// 至少要求 `level.dat` 存在、非空且能被 NBT 解析。更深的区域文件校验
/// 依赖具体版本布局（26.3 已改到 `dimensions/`），因此这里不硬编码目录名。
pub fn validate_world_dir(dir: &Path) -> Result<WorldSummary, String> {
    if !dir.is_dir() {
        return Err(format!("世界目录不存在: {}", dir.display()));
    }
    let level_dat = dir.join("level.dat");
    let meta = std::fs::metadata(&level_dat).map_err(|e| format!("缺少可读的 level.dat: {}", e))?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(format!("level.dat 为空或不是文件: {}", level_dat.display()));
    }
    parse_world(dir).map_err(|e| format!("level.dat 校验失败: {e}"))
}

fn tag_i32(tag: &Tag) -> Option<i32> {
    match tag {
        Tag::Byte(v) => Some(*v as i32),
        Tag::Short(v) => Some(*v as i32),
        Tag::Int(v) => Some(*v),
        Tag::Long(v) => Some(*v as i32),
        _ => None,
    }
}

fn tag_bool(tag: &Tag) -> Option<bool> {
    tag_i32(tag).map(|v| v != 0)
}

fn difficulty_name(value: i32) -> String {
    match value {
        0 => "peaceful",
        1 => "easy",
        2 => "normal",
        3 => "hard",
        _ => "normal",
    }
    .to_string()
}

pub fn difficulty_label(difficulty: &str) -> &'static str {
    match difficulty {
        "peaceful" => "和平",
        "easy" => "简单",
        "normal" => "普通",
        "hard" => "困难",
        _ => "普通",
    }
}

fn dir_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 重命名：只改 `Data.LevelName`，不移动文件夹（与原版一致）。
pub fn rename(saves_dir: &Path, id: &str, new_name: &str) -> Result<(), String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("世界名称不能为空".to_string());
    }
    if new_name.chars().count() > 64 {
        return Err("世界名称过长（最多 64 字符）".to_string());
    }
    let dir = world_dir(saves_dir, id)?;
    let level_dat = dir.join("level.dat");
    let mut nbt = NbtFile::read_gzip(&level_dat)?;
    let data = nbt
        .root
        .as_compound_mut()
        .and_then(|root| root.get_mut("Data"))
        .ok_or_else(|| "level.dat 缺少 Data 标签".to_string())?;
    let data = data
        .as_compound_mut()
        .ok_or_else(|| "level.dat 的 Data 不是 Compound".to_string())?;
    data.insert("LevelName".to_string(), Tag::String(new_name.to_string()));
    nbt.write_gzip(&level_dat)
}

/// 复制存档，返回新存档的文件夹 id。
pub fn duplicate(saves_dir: &Path, id: &str, new_name: Option<&str>) -> Result<String, String> {
    let source = world_dir(saves_dir, id)?;
    let source_name = parse_world(&source)
        .map(|w| w.name)
        .unwrap_or_else(|_| id.to_string());
    let base_name = new_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{source_name} (副本)"));

    let mut new_id = format!("{id}-copy");
    let mut counter = 2;
    while saves_dir.join(&new_id).exists() {
        new_id = format!("{id}-copy-{counter}");
        counter += 1;
        if counter > 9999 {
            return Err("无法生成新的存档 id".to_string());
        }
    }
    let target = saves_dir.join(&new_id);
    copy_dir(&source, &target)?;

    let level_dat = target.join("level.dat");
    let mut nbt = NbtFile::read_gzip(&level_dat)?;
    {
        let data = nbt
            .root
            .as_compound_mut()
            .and_then(|root| root.get_mut("Data"))
            .ok_or_else(|| "level.dat 缺少 Data 标签".to_string())?;
        let data = data
            .as_compound_mut()
            .ok_or_else(|| "level.dat 的 Data 不是 Compound".to_string())?;
        data.insert("LevelName".to_string(), Tag::String(base_name));
        data.insert("LastPlayed".to_string(), Tag::Long(now_ms()));
        // 复制出的存档不应继承“已锁定”
        if let Some(Tag::Compound(settings)) = data.get_mut("difficulty_settings") {
            settings.insert("locked".to_string(), Tag::Byte(0));
        }
    }
    nbt.write_gzip(&level_dat)?;

    // session.lock 属于源存档，复制后必须移除，否则客户端可能认为世界被占用
    let _ = std::fs::remove_file(target.join("session.lock"));
    Ok(new_id)
}

/// 删除（移动）到 `saves/.deepslate-trash/`，返回移动后的路径。
pub fn move_to_trash(saves_dir: &Path, id: &str) -> Result<PathBuf, String> {
    let source = world_dir(saves_dir, id)?;
    let trash = saves_dir.join(TRASH_DIR);
    std::fs::create_dir_all(&trash).map_err(|e| format!("创建回收站失败: {e}"))?;
    let target = trash.join(format!("{}-{id}", now_ms()));
    match std::fs::rename(&source, &target) {
        Ok(()) => Ok(target),
        Err(_) => {
            // 跨文件系统时 rename 会失败：退回复制 + 删除
            copy_dir(&source, &target)?;
            std::fs::remove_dir_all(&source).map_err(|e| format!("删除原存档失败: {e}"))?;
            Ok(target)
        }
    }
}

/// 清空回收站，返回删除的条目数。
pub fn empty_trash(saves_dir: &Path) -> Result<usize, String> {
    let trash = saves_dir.join(TRASH_DIR);
    if !trash.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in std::fs::read_dir(&trash)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| format!("删除 {} 失败: {e}", path.display()))?;
            count += 1;
        } else if path.is_file() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("删除 {} 失败: {e}", path.display()))?;
            count += 1;
        }
    }
    Ok(count)
}

fn world_dir(saves_dir: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == ".." {
        return Err(format!("非法存档 id: {id}"));
    }
    let dir = saves_dir.join(id);
    if !dir.is_dir() {
        return Err(format!("存档不存在: {id}"));
    }
    Ok(dir)
}

pub(crate) fn copy_dir(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Err(format!("目标已存在: {}", target.display()));
    }
    std::fs::create_dir_all(target).map_err(|e| format!("创建目录失败: {e}"))?;
    let entries = std::fs::read_dir(source).map_err(|e| format!("读取目录失败: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "session.lock" {
            continue;
        }
        let dest = target.join(&name);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("读取元数据失败 {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_dir(&path, &dest)?;
        } else if meta.is_file() {
            // 用缓冲区复制，避免把大 region 文件一次性读进内存
            let mut input = std::fs::File::open(&path)
                .map_err(|e| format!("打开 {} 失败: {e}", path.display()))?;
            let mut output = std::fs::File::create(&dest)
                .map_err(|e| format!("创建 {} 失败: {e}", dest.display()))?;
            std::io::copy(&mut input, &mut output)
                .map_err(|e| format!("复制 {} 失败: {e}", path.display()))?;
            output.flush().ok();
        }
        // 符号链接跳过：存档目录里几乎不会出现，避免越界复制
    }
    Ok(())
}

/// 给 GUI 用的一行摘要（不含大小）。
pub fn summary_line(world: &WorldSummary) -> String {
    let mut parts = vec![
        game_mode_label(&world.game_mode).to_string(),
        format!("难度 {}", difficulty_label(&world.difficulty)),
        format!(
            "版本 {}",
            if world.version_name.is_empty() {
                "未知"
            } else {
                &world.version_name
            }
        ),
        format!("最后游玩 {}", world.last_played_text()),
    ];
    if world.hardcore {
        parts.push("极限".to_string());
    }
    if world.cheats {
        parts.push("作弊".to_string());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::NbtFile;
    use std::collections::BTreeMap;

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("deepslate-saves-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("world")).unwrap();
        dir
    }

    fn write_world(root: &Path, id: &str, name: &str, game_type: i32, last: i64) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut data = BTreeMap::new();
        data.insert("LevelName".to_string(), Tag::String(name.to_string()));
        data.insert("GameType".to_string(), Tag::Int(game_type));
        data.insert("LastPlayed".to_string(), Tag::Long(last));
        data.insert("allowCommands".to_string(), Tag::Byte(1));
        data.insert("DataVersion".to_string(), Tag::Int(5023));
        data.insert(
            "difficulty_settings".to_string(),
            Tag::Compound(BTreeMap::from([
                ("difficulty".to_string(), Tag::String("easy".to_string())),
                ("hardcore".to_string(), Tag::Byte(0)),
                ("locked".to_string(), Tag::Byte(1)),
            ])),
        );
        data.insert(
            "Version".to_string(),
            Tag::Compound(BTreeMap::from([
                ("Name".to_string(), Tag::String("26.3".to_string())),
                ("Id".to_string(), Tag::Int(5023)),
            ])),
        );
        let nbt = NbtFile {
            root_name: String::new(),
            root: Tag::Compound(BTreeMap::from([("Data".to_string(), Tag::Compound(data))])),
        };
        nbt.write_gzip(&dir.join("level.dat")).unwrap();
        std::fs::write(dir.join("icon.png"), b"png").unwrap();
        std::fs::create_dir_all(dir.join("region")).unwrap();
        std::fs::write(dir.join("region/r.0.0.mca"), vec![0u8; 100]).unwrap();
    }

    #[test]
    fn lists_and_parses_worlds() {
        let root = temp_root("list");
        write_world(&root, "alpha", "Alpha World", 0, 1_700_000_000_000);
        write_world(&root, "beta", "Beta World", 1, 1_800_000_000_000);
        std::fs::create_dir_all(root.join("broken")).unwrap();
        std::fs::write(root.join("broken/level.dat"), b"not nbt").unwrap();

        let worlds = list(&root);
        assert_eq!(worlds.len(), 2);
        assert_eq!(worlds[0].id, "beta");
        assert_eq!(worlds[0].game_mode, "creative");
        assert_eq!(worlds[0].version_name, "26.3");
        assert!(worlds[0].cheats);
        assert!(worlds[0].icon.is_some());
        assert!(worlds[0].size_bytes >= 100);
        assert!(worlds[0].last_played > worlds[1].last_played);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rename_only_changes_level_name() {
        let root = temp_root("rename");
        write_world(&root, "world", "Old Name", 0, 1);
        rename(&root, "world", "新名字").unwrap();
        let world = parse_world(&root.join("world")).unwrap();
        assert_eq!(world.name, "新名字");
        assert_eq!(world.id, "world");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn duplicate_creates_unique_world_and_resets_lock() {
        let root = temp_root("dup");
        write_world(&root, "world", "Source", 0, 1);
        std::fs::write(root.join("world/session.lock"), b"x").unwrap();

        let new_id = duplicate(&root, "world", Some("Copy")).unwrap();
        assert_eq!(new_id, "world-copy");
        let copy = parse_world(&root.join(&new_id)).unwrap();
        assert_eq!(copy.name, "Copy");
        assert!(!root.join(&new_id).join("session.lock").exists());
        assert!(root.join(&new_id).join("region/r.0.0.mca").exists());

        let second = duplicate(&root, "world", None).unwrap();
        assert_eq!(second, "world-copy-2");
        assert!(parse_world(&root.join(second))
            .unwrap()
            .name
            .starts_with("Source"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn moving_to_trash_hides_from_list_and_can_be_emptied() {
        let root = temp_root("trash");
        write_world(&root, "world", "World", 0, 1);
        let moved = move_to_trash(&root, "world").unwrap();
        assert!(moved.exists());
        assert!(list(&root).is_empty());
        assert_eq!(empty_trash(&root).unwrap(), 1);
        assert!(!moved.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parses_real_26_3_level_dat() {
        let root = temp_root("real263");
        let dir = root.join("world");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat"),
            dir.join("level.dat"),
        )
        .unwrap();
        let world = parse_world(&dir).unwrap();
        assert_eq!(world.name, "world");
        assert_eq!(world.version_name, "26.3");
        assert_eq!(world.data_version, Some(5023));
        assert_eq!(world.game_mode, "survival");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rename_and_duplicate_real_26_3_world() {
        let root = temp_root("real263ops");
        let dir = root.join("world");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat"),
            dir.join("level.dat"),
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("dimensions/minecraft/overworld/region")).unwrap();
        std::fs::write(
            dir.join("dimensions/minecraft/overworld/region/r.0.0.mca"),
            b"region",
        )
        .unwrap();

        rename(&root, "world", "深板岩测试世界").unwrap();
        assert_eq!(
            parse_world(&root.join("world")).unwrap().name,
            "深板岩测试世界"
        );

        let copy_id = duplicate(&root, "world", Some("副本世界")).unwrap();
        let copy = parse_world(&root.join(&copy_id)).unwrap();
        assert_eq!(copy.name, "副本世界");
        assert_eq!(copy.version_name, "26.3");
        assert!(root
            .join(&copy_id)
            .join("dimensions/minecraft/overworld/region/r.0.0.mca")
            .is_file());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn validate_world_requires_parseable_level_dat() {
        let root = temp_root("validate");
        let dir = root.join("partial");
        std::fs::create_dir_all(&dir).unwrap();
        let err = validate_world_dir(&dir).unwrap_err();
        assert!(err.contains("level.dat"));

        std::fs::write(dir.join("level.dat"), b"not nbt").unwrap();
        let err = validate_world_dir(&dir).unwrap_err();
        assert!(err.contains("校验失败"));

        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat");
        std::fs::copy(fixture, dir.join("level.dat")).unwrap();
        let world = validate_world_dir(&dir).unwrap();
        assert_eq!(world.version_name, "26.3");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
