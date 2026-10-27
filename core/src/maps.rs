//! 地图库（世界库）。
//!
//! 设计目标：地图是启动器管理的长期资产，而不是某个版本的 `saves/` 附属物。
//! 每个地图是 `maps/<map-id>/`，内部就是标准 Minecraft 世界目录，额外带一个
//! 游戏会忽略的 `map.json`：
//!
//! ```text
//! maps/
//!   <map-id>/
//!     level.dat
//!     region/
//!     dimensions/
//!     icon.png
//!     map.json
//!   .staging/
//!   .deepslate-trash/
//!   exports/
//! ```
//!
//! 标准 Minecraft 客户端只认 `level.dat`，所以地图库可以直接作为实例的
//! `saves/` 目录（通过符号链接/junction 接入），`--quickPlaySingleplayer <id>`
//! 中的 id 就是这里的文件夹名。
//!
//! 第一版刻意保持“地图库 = live save”：
//! - 不复制大世界到工作副本；
//! - 游戏退出后只刷新元数据；
//! - 需要回滚时使用备份/导出，而不是工作副本提交。
//! 这样可以直接复用 `saves.rs` 和原版 quickPlay。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::config::AppConfig;
use crate::nbt::{NbtFile, Tag};
use crate::saves::{self, WorldSummary};

pub const MANIFEST_NAME: &str = "map.json";
pub const TRASH_DIR: &str = ".deepslate-trash";
pub const STAGING_DIR: &str = ".staging";
pub const EXPORT_DIR: &str = "exports";

/// 跨进程保护共享地图库；游戏运行与地图读写操作均需持有此锁。
pub fn lock_library(config: &AppConfig) -> Result<File, String> {
    let maps_root = root(config);
    fs::create_dir_all(&maps_root).map_err(|e| format!("创建地图库失败: {e}"))?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(maps_root.join(".deepslate-library.lock"))
        .map_err(|e| format!("打开地图库锁失败: {e}"))?;
    file.try_lock()
        .map_err(|e| format!("地图库正在被游戏或其他任务使用: {e}"))?;
    Ok(file)
}

#[cfg(unix)]
pub(crate) fn duplicate_inheritable_lock(lock: &File) -> std::io::Result<File> {
    lock.try_clone()
}

/// 单个地图包解压上限，防止 zip bomb。世界可以很大，但不能无限。
const MAX_IMPORT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_IMPORT_ENTRIES: usize = 200_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MapSource {
    Generated,
    Imported,
    Downloaded,
    Migrated,
}

impl Default for MapSource {
    fn default() -> Self {
        Self::Imported
    }
}

impl MapSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Generated => "启动器生成",
            Self::Imported => "本地导入",
            Self::Downloaded => "下载",
            Self::Migrated => "旧存档迁移",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeneratorInfo {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub seed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameInfo {
    #[serde(default)]
    pub data_version: Option<i32>,
    #[serde(default)]
    pub version_name: String,
}

/// `map.json` 的内容。游戏不会读取它；启动器用它保存来源、生成参数等
/// 无法从 `level.dat` 稳定推断的信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapManifest {
    #[serde(default = "default_schema")]
    pub schema: u32,
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub source: MapSource,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub last_played: i64,
    #[serde(default)]
    pub generator: Option<GeneratorInfo>,
    #[serde(default)]
    pub game: Option<GameInfo>,
    #[serde(default)]
    pub migrated_from: Option<String>,
}

fn default_schema() -> u32 {
    1
}

impl MapManifest {
    fn new(id: &str, name: &str, source: MapSource) -> Self {
        let now = now_ms();
        Self {
            schema: 1,
            id: id.to_string(),
            name: name.to_string(),
            source,
            created_at: now,
            updated_at: now,
            last_played: 0,
            generator: None,
            game: None,
            migrated_from: None,
        }
    }

    fn from_world(world: &WorldSummary, source: MapSource) -> Self {
        let mut manifest = Self::new(&world.id, &world.name, source);
        manifest.last_played = world.last_played;
        manifest.game = Some(GameInfo {
            data_version: world.data_version,
            version_name: world.version_name.clone(),
        });
        manifest
    }

    fn sync_world(&mut self, world: &WorldSummary) {
        if !world.name.trim().is_empty() {
            self.name = world.name.clone();
        }
        self.last_played = world.last_played.max(self.last_played);
        self.game = Some(GameInfo {
            data_version: world.data_version,
            version_name: world.version_name.clone(),
        });
        self.updated_at = now_ms();
    }
}

/// 地图列表项：`Deref` 到 `WorldSummary`，GUI/CLI 可以直接读 `id/name/...`。
#[derive(Debug, Clone)]
pub struct MapSummary {
    pub manifest: MapManifest,
    pub world: WorldSummary,
}

impl Deref for MapSummary {
    type Target = WorldSummary;

    fn deref(&self) -> &Self::Target {
        &self.world
    }
}

impl MapSummary {
    pub fn source_label(&self) -> &'static str {
        self.manifest.source.label()
    }
}

/// 地图库根目录：`<root>/maps`。
pub fn root(config: &AppConfig) -> PathBuf {
    PathBuf::from(&config.root_dir).join("maps")
}

pub fn map_dir(config: &AppConfig, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    let dir = root(config).join(id);
    let metadata = fs::symlink_metadata(&dir).map_err(|_| format!("地图不存在: {id}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("地图不存在: {id}"));
    }
    let library = fs::canonicalize(root(config)).map_err(|e| format!("解析地图库失败: {e}"))?;
    let resolved = fs::canonicalize(&dir).map_err(|e| format!("解析地图失败: {e}"))?;
    if resolved.parent() != Some(library.as_path()) {
        return Err(format!("地图路径不在地图库内: {id}"));
    }
    Ok(dir)
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(MANIFEST_NAME)
}

/// 创建地图库基础目录。列表/导入/启动前都可以安全调用。
pub fn ensure_root(config: &AppConfig) -> Result<PathBuf, String> {
    let root = root(config);
    for dir in [
        root.clone(),
        root.join(STAGING_DIR),
        root.join(TRASH_DIR),
        root.join(EXPORT_DIR),
    ] {
        fs::create_dir_all(&dir).map_err(|e| format!("创建地图目录失败 {}: {e}", dir.display()))?;
    }
    Ok(root)
}

/// 新地图 id：稳定、与显示名无关，可直接作为 quickPlay 的文件夹名。
pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

/// A pending record hides partially created worlds until they have been verified.
fn pending_creation(config: &AppConfig, id: &str) -> PathBuf {
    root(config)
        .join(".pending-creation")
        .join(format!("{id}.json"))
}

#[derive(Debug, Clone)]
pub struct PendingCreation {
    pub id: String,
    pub name: String,
    pub problem: Option<String>,
}

fn validate_created_world(dir: &Path) -> Result<WorldSummary, String> {
    let world = saves::validate_world_dir(dir)?;
    let settings =
        crate::nbt::NbtFile::read_gzip(&dir.join("data/minecraft/world_gen_settings.dat"))
            .map_err(|e| format!("世界生成设置不可读: {e}"))?;
    if !matches!(
        settings.root.get_path(&["data", "seed"]),
        Some(crate::nbt::Tag::Long(_))
    ) || settings
        .root
        .get_path(&[
            "data",
            "dimensions",
            "minecraft:overworld",
            "generator",
            "type",
        ])
        .and_then(|v| match v {
            crate::nbt::Tag::String(s) if !s.is_empty() => Some(s),
            _ => None,
        })
        .is_none()
    {
        return Err("世界生成设置不完整".into());
    }
    Ok(world)
}

pub fn list_pending_creations(config: &AppConfig) -> Result<Vec<PendingCreation>, String> {
    let directory = root(config).join(".pending-creation");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|e| format!("读取待恢复世界失败: {e}"))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let filename = entry.file_name();
        let Some(id) = filename.to_str().and_then(|s| s.strip_suffix(".json")) else {
            continue;
        };
        if Uuid::parse_str(id)
            .map(|u| u.to_string() != id)
            .unwrap_or(true)
        {
            continue;
        }
        let manifest = fs::read(entry.path())
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<MapManifest>(&bytes).map_err(|e| e.to_string())
            });
        let (name, problem) = match manifest {
            Ok(manifest)
                if manifest.id == id && matches!(manifest.source, MapSource::Generated) =>
            {
                let problem = map_dir(config, id)
                    .and_then(|dir| validate_created_world(&dir).map(|_| ()))
                    .err();
                (manifest.name, problem)
            }
            _ => (id.to_string(), Some("创建记录损坏或不匹配".into())),
        };
        result.push(PendingCreation {
            id: id.to_string(),
            name,
            problem,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Ok(result)
}

/// Explicit recovery is serialized with running games and validates files again.
pub fn recover_client_creation(config: &AppConfig, id: &str) -> Result<MapSummary, String> {
    let _guard = lock_library(config)?;
    finish_client_creation(config, id)
}

pub fn begin_client_creation(
    config: &AppConfig,
    id: &str,
    name: &str,
    generator: &GeneratorInfo,
) -> Result<(), String> {
    validate_id(id)?;
    if uuid::Uuid::parse_str(id).is_err() {
        return Err("新世界标识无效".into());
    }
    if root(config).join(id).exists() {
        return Err("世界目录已经存在".into());
    }
    let path = pending_creation(config, id);
    fs::create_dir_all(path.parent().ok_or("创建记录目录无效")?).map_err(|e| e.to_string())?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(
        file,
        &MapManifest {
            generator: Some(generator.clone()),
            ..MapManifest::new(id, name, MapSource::Generated)
        },
    )
    .map_err(|e| format!("写入世界创建记录失败: {e}"))
}

pub fn finish_client_creation(config: &AppConfig, id: &str) -> Result<MapSummary, String> {
    let dir = map_dir(config, id)?;
    let record = pending_creation(config, id);
    let mut manifest: MapManifest =
        serde_json::from_slice(&fs::read(&record).map_err(|e| format!("读取创建记录失败: {e}"))?)
            .map_err(|e| format!("创建记录损坏: {e}"))?;
    if manifest.id != id || !matches!(manifest.source, MapSource::Generated) {
        return Err("创建记录不匹配".into());
    }
    let world = validate_created_world(&dir)?;
    manifest.sync_world(&world);
    write_manifest(&dir, &manifest)?;
    fs::remove_file(record).map_err(|e| format!("移除创建记录失败: {e}"))?;
    Ok(MapSummary { manifest, world })
}

pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains(':')
        || id.contains('\0')
    {
        return Err(format!("非法地图 id: {id:?}"));
    }
    Ok(())
}

/// 列出地图库。只扫描包含 `level.dat` 的目录；点开头目录（回收站/staging）跳过。
pub fn list(config: &AppConfig) -> Result<Vec<MapSummary>, String> {
    let root = ensure_root(config)?;
    let entries = fs::read_dir(&root).map_err(|e| format!("读取地图库失败: {e}"))?;
    let mut maps = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if id.starts_with('.')
            || pending_creation(config, id).exists()
            || !dir.join("level.dat").is_file()
        {
            continue;
        }
        let Ok(world) = saves::parse_world(&dir) else {
            continue;
        };
        let mut manifest = match load_manifest(&dir) {
            Ok(manifest) => manifest,
            Err(_) => {
                let manifest = MapManifest::from_world(&world, MapSource::Imported);
                manifest
            }
        };
        manifest.id = id.to_string();
        maps.push(MapSummary { manifest, world });
    }
    maps.sort_by(|a, b| {
        b.world
            .last_played
            .cmp(&a.world.last_played)
            .then_with(|| a.world.id.cmp(&b.world.id))
    });
    Ok(maps)
}

fn load_manifest(dir: &Path) -> Result<MapManifest, String> {
    let path = manifest_path(dir);
    let text =
        fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("解析 {} 失败: {e}", path.display()))
}

fn write_manifest(dir: &Path, manifest: &MapManifest) -> Result<(), String> {
    let path = manifest_path(dir);
    let tmp = dir.join(format!("{MANIFEST_NAME}.tmp"));
    let text = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| format!("写入 {} 失败: {e}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .or_else(|_| {
            // Windows 上目标存在时 rename 会失败：先删再换。
            let _ = fs::remove_file(&path);
            fs::rename(&tmp, &path)
        })
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

/// 如果地图目录还没有 map.json，生成一个。用于导入旧世界/手工放进来的世界。
pub fn ensure_manifest(config: &AppConfig, id: &str) -> Result<MapManifest, String> {
    let _guard = lock_library(config)?;
    let dir = map_dir(config, id)?;
    if let Ok(manifest) = load_manifest(&dir) {
        return Ok(manifest);
    }
    let world = saves::parse_world(&dir)?;
    let manifest = MapManifest::from_world(&world, MapSource::Imported);
    write_manifest(&dir, &manifest)?;
    Ok(manifest)
}

pub fn rename(config: &AppConfig, id: &str, new_name: &str) -> Result<(), String> {
    let _guard = lock_library(config)?;
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("世界名称不能为空".to_string());
    }
    if new_name.chars().count() > 64 {
        return Err("世界名称过长（最多 64 字符）".to_string());
    }
    let dir = map_dir(config, id)?;
    saves::rename(&root(config), id, new_name)?;
    let mut manifest =
        load_manifest(&dir).unwrap_or_else(|_| MapManifest::new(id, new_name, MapSource::Imported));
    manifest.name = new_name.to_string();
    manifest.updated_at = now_ms();
    write_manifest(&dir, &manifest)
}

/// 复制地图：新 id、新文件夹、LevelName 加“副本”，重置 LastPlayed/session.lock。
pub fn duplicate(config: &AppConfig, id: &str, new_name: Option<&str>) -> Result<String, String> {
    let _guard = lock_library(config)?;
    let source = map_dir(config, id)?;
    let world = saves::parse_world(&source)?;
    let base_name = new_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} (副本)", world.name));

    let new_id = new_id();
    let target = root(config).join(&new_id);
    saves::copy_dir(&source, &target)?;
    let _ = fs::remove_file(target.join("session.lock"));

    let level_dat = target.join("level.dat");
    let mut nbt = NbtFile::read_gzip(&level_dat)?;
    {
        let data = nbt
            .root
            .as_compound_mut()
            .and_then(|root| root.get_mut("Data"))
            .ok_or_else(|| "level.dat 缺少 Data 标签".to_string())?
            .as_compound_mut()
            .ok_or_else(|| "level.dat 的 Data 不是 Compound".to_string())?;
        data.insert("LevelName".to_string(), Tag::String(base_name.clone()));
        data.insert("LastPlayed".to_string(), Tag::Long(now_ms()));
        if let Some(Tag::Compound(settings)) = data.get_mut("difficulty_settings") {
            settings.insert("locked".to_string(), Tag::Byte(0));
        }
    }
    nbt.write_gzip(&level_dat)?;

    let mut manifest = load_manifest(&source)
        .unwrap_or_else(|_| MapManifest::from_world(&world, MapSource::Imported));
    manifest.id = new_id.clone();
    manifest.name = base_name;
    manifest.created_at = now_ms();
    manifest.updated_at = now_ms();
    manifest.last_played = 0;
    write_manifest(&target, &manifest)?;
    Ok(new_id)
}

/// 移动到地图库回收站，返回新路径。
pub fn move_to_trash(config: &AppConfig, id: &str) -> Result<PathBuf, String> {
    let _guard = lock_library(config)?;
    let source = map_dir(config, id)?;
    let trash = root(config).join(TRASH_DIR);
    fs::create_dir_all(&trash).map_err(|e| format!("创建地图回收站失败: {e}"))?;
    let base = format!("{}-{id}", now_ms());
    let mut target = trash.join(&base);
    let mut n = 2;
    while target.exists() {
        target = trash.join(format!("{base}-{n}"));
        n += 1;
    }
    match fs::rename(&source, &target) {
        Ok(()) => Ok(target),
        Err(_) => {
            saves::copy_dir(&source, &target)?;
            fs::remove_dir_all(&source).map_err(|e| format!("删除原地图失败: {e}"))?;
            Ok(target)
        }
    }
}

pub fn empty_trash(config: &AppConfig) -> Result<usize, String> {
    let trash = root(config).join(TRASH_DIR);
    if !trash.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(&trash)
        .map_err(|e| format!("读取地图回收站失败: {e}"))?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path).map_err(|e| format!("删除 {} 失败: {e}", path.display()))?;
            count += 1;
        } else if path.is_file() {
            fs::remove_file(&path).map_err(|e| format!("删除 {} 失败: {e}", path.display()))?;
            count += 1;
        }
    }
    Ok(count)
}

fn backups_root(config: &AppConfig, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(PathBuf::from(&config.root_dir)
        .join("backups")
        .join("maps")
        .join(id))
}

/// 为单张地图创建一份完整快照，保留最近 5 份。
pub fn backup(config: &AppConfig, id: &str) -> Result<String, String> {
    let _guard = lock_library(config)?;
    backup_for_launch(config, id, &_guard)
}

pub(crate) fn backup_for_launch(
    config: &AppConfig,
    id: &str,
    _guard: &File,
) -> Result<String, String> {
    let source = map_dir(config, id)?;
    let root = backups_root(config, id)?;
    fs::create_dir_all(&root).map_err(|e| format!("创建地图备份目录失败: {e}"))?;

    let base = now_ms().to_string();
    let mut name = base.clone();
    let mut n = 2;
    while root.join(&name).exists() {
        name = format!("{base}-{n}");
        n += 1;
    }
    let target = root.join(&name);
    let stage = root.join(format!(".backup-{}", Uuid::new_v4()));
    let result = saves::copy_dir(&source, &stage)
        .and_then(|()| fs::rename(&stage, &target).map_err(|e| format!("完成地图备份失败: {e}")));
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result?;
    cleanup_old_backups(&root, 5)?;
    Ok(name)
}

pub fn list_backups(config: &AppConfig, id: &str) -> Result<Vec<String>, String> {
    let root = backups_root(config, id)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = fs::read_dir(&root)
        .map_err(|e| format!("读取地图备份目录失败: {e}"))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort_by(|a, b| b.cmp(a));
    Ok(names)
}

fn backup_dir(config: &AppConfig, id: &str, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.starts_with('.') {
        return Err(format!("非法备份名: {name:?}"));
    }
    let dir = backups_root(config, id)?.join(name);
    if !dir.is_dir() {
        return Err(format!("地图备份不存在: {name}"));
    }
    Ok(dir)
}

/// 用备份替换当前地图。先复制到 staging，成功后再原子换入；失败会回滚。
pub fn restore_backup(config: &AppConfig, id: &str, name: &str) -> Result<(), String> {
    let _guard = lock_library(config)?;
    let source = backup_dir(config, id, name)?;
    let target = map_dir(config, id)?;
    let staging_root = root(config).join(STAGING_DIR);
    fs::create_dir_all(&staging_root).map_err(|e| format!("创建恢复暂存目录失败: {e}"))?;
    let stage = staging_root.join(format!("restore-{}", new_id()));
    let old = staging_root.join(format!("replaced-{}", new_id()));

    let result = (|| {
        saves::copy_dir(&source, &stage)?;
        let world = saves::validate_world_dir(&stage)?;
        let mut manifest = load_manifest(&stage)
            .unwrap_or_else(|_| MapManifest::from_world(&world, MapSource::Imported));
        manifest.id = id.to_string();
        manifest.updated_at = now_ms();
        write_manifest(&stage, &manifest)?;

        fs::rename(&target, &old).map_err(|e| format!("暂存当前地图失败: {e}"))?;
        if let Err(e) = fs::rename(&stage, &target) {
            let _ = fs::rename(&old, &target);
            return Err(format!("替换地图失败: {e}"));
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    if old.exists() {
        let _ = fs::remove_dir_all(&old);
    }
    result
}

pub fn delete_backup(config: &AppConfig, id: &str, name: &str) -> Result<(), String> {
    let dir = backup_dir(config, id, name)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("删除地图备份失败: {e}"))
}

fn cleanup_old_backups(root: &Path, keep: usize) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(root)
        .map_err(|e| format!("读取地图备份目录失败: {e}"))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    if entries.len() <= keep {
        return Ok(());
    }
    entries.sort_by_key(|e| e.file_name());
    for entry in entries.iter().take(entries.len() - keep) {
        let _ = fs::remove_dir_all(entry.path());
    }
    Ok(())
}

/// 游戏退出后刷新地图元数据。地图库是 live save，所以这里只需要重新解析。
pub fn refresh_after_play(config: &AppConfig, id: &str) -> Result<MapSummary, String> {
    let _guard = lock_library(config)?;
    let dir = map_dir(config, id)?;
    let world = saves::parse_world(&dir)?;
    let mut manifest = load_manifest(&dir)
        .unwrap_or_else(|_| MapManifest::from_world(&world, MapSource::Imported));
    manifest.id = id.to_string();
    manifest.sync_world(&world);
    write_manifest(&dir, &manifest)?;
    Ok(MapSummary { manifest, world })
}

/// 导入文件或目录：目录按世界目录处理，zip 自动找 `level.dat`。
pub fn import_path(config: &AppConfig, path: &Path) -> Result<MapSummary, String> {
    if path.is_dir() {
        import_dir(config, path)
    } else if path.is_file() {
        import_zip(config, path)
    } else {
        Err(format!("路径不存在: {}", path.display()))
    }
}

pub fn import_dir(config: &AppConfig, path: &Path) -> Result<MapSummary, String> {
    import_dir_with_source(config, path, MapSource::Imported, None)
}

/// 导入目录并指定来源（旧存档迁移用）。
pub fn import_dir_with_source(
    config: &AppConfig,
    path: &Path,
    source: MapSource,
    migrated_from: Option<String>,
) -> Result<MapSummary, String> {
    let _guard = lock_library(config)?;
    ensure_root(config)?;
    let stage = staging_dir(config)?;
    let result = (|| {
        saves::copy_dir(path, &stage)?;
        let _ = fs::remove_file(stage.join("session.lock"));
        adopt_stage(config, &stage, source, migrated_from)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

pub fn import_zip(config: &AppConfig, path: &Path) -> Result<MapSummary, String> {
    let _guard = lock_library(config)?;
    ensure_root(config)?;
    let stage = staging_dir(config)?;
    let result = (|| {
        fs::create_dir_all(&stage).map_err(|e| format!("创建导入暂存目录失败: {e}"))?;
        extract_zip(path, &stage)?;
        let _ = fs::remove_file(stage.join("session.lock"));
        adopt_stage(config, &stage, MapSource::Imported, None)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

/// 把启动器生成好的世界目录直接纳入地图库（移动，不复制）。
pub fn adopt_generated_dir(
    config: &AppConfig,
    source: &Path,
    generator: Option<GeneratorInfo>,
) -> Result<MapSummary, String> {
    let _guard = lock_library(config)?;
    ensure_root(config)?;
    saves::validate_world_dir(source)?;
    let id = new_id();
    let target = root(config).join(&id);
    move_dir(source, &target)?;
    let result = (|| {
        let _ = fs::remove_file(target.join("session.lock"));
        let world = saves::parse_world(&target)?;
        let mut manifest = MapManifest::from_world(&world, MapSource::Generated);
        manifest.id = id.clone();
        manifest.generator = generator;
        manifest.updated_at = now_ms();
        write_manifest(&target, &manifest)?;
        Ok(MapSummary { manifest, world })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn staging_dir(config: &AppConfig) -> Result<PathBuf, String> {
    let dir = root(config).join(STAGING_DIR).join(new_id());
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建导入暂存目录失败: {e}"))?;
    }
    Ok(dir)
}

fn adopt_stage(
    config: &AppConfig,
    stage: &Path,
    source: MapSource,
    migrated_from: Option<String>,
) -> Result<MapSummary, String> {
    if !stage.join("level.dat").is_file() {
        return Err("没有找到 level.dat，无法识别为 Minecraft 世界".to_string());
    }
    saves::validate_world_dir(stage)?;
    let id = new_id();
    let target = root(config).join(&id);
    move_dir(stage, &target)?;
    let result = (|| {
        let world = saves::parse_world(&target)?;
        let mut manifest = MapManifest::from_world(&world, source);
        manifest.id = id.clone();
        manifest.migrated_from = migrated_from;
        manifest.updated_at = now_ms();
        write_manifest(&target, &manifest)?;
        Ok(MapSummary { manifest, world })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&target);
    }
    result
}

fn move_dir(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Err(format!("目标已存在: {}", target.display()));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目标父目录失败: {e}"))?;
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            saves::copy_dir(source, target)?;
            fs::remove_dir_all(source).map_err(|e| format!("删除暂存目录失败: {e}"))?;
            Ok(())
        }
    }
}

/// 导出 zip：地图目录内容直接放在 zip 根下，`map.json` 一并包含。
/// 返回实际写出的 zip 路径。
pub fn export_zip(config: &AppConfig, id: &str, out: Option<&Path>) -> Result<PathBuf, String> {
    let _guard = lock_library(config)?;
    let dir = map_dir(config, id)?;
    let world = saves::parse_world(&dir)?;
    let out = match out {
        Some(path) => path.to_path_buf(),
        None => {
            let file = format!("{}-{}.zip", safe_filename(&world.name), now_ms());
            root(config).join(EXPORT_DIR).join(file)
        }
    };
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建导出目录失败: {e}"))?;
    }
    let file = File::create(&out).map_err(|e| format!("创建 {} 失败: {e}", out.display()))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    add_dir_to_zip(&mut zip, &dir, &dir, options)?;
    zip.finish().map_err(|e| format!("完成 zip 失败: {e}"))?;
    Ok(out)
}

fn add_dir_to_zip(
    zip: &mut ZipWriter<File>,
    root: &Path,
    dir: &Path,
    options: SimpleFileOptions,
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("读取 {} 失败: {e}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "session.lock" || name.starts_with(".deepslate") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            zip.add_directory(format!("{rel}/"), options)
                .map_err(|e| format!("写入 zip 目录 {rel} 失败: {e}"))?;
            add_dir_to_zip(zip, root, &path, options)?;
        } else if meta.is_file() {
            zip.start_file(rel, options)
                .map_err(|e| format!("写入 zip 文件失败: {e}"))?;
            let mut file =
                File::open(&path).map_err(|e| format!("打开 {} 失败: {e}", path.display()))?;
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = file.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn extract_zip(path: &Path, stage: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("打开 zip 失败 {}: {e}", path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {e}"))?;

    // 找到最短层级的 level.dat，作为地图根。
    let mut candidates: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;
        let name = normalize_zip_name(entry.name());
        if name == "level.dat" || name.ends_with("/level.dat") {
            candidates.push(name);
        }
    }
    candidates.sort_by_key(|name| name.matches('/').count());
    candidates.dedup();
    let Some(level_dat) = candidates.first() else {
        return Err("zip 中没有 level.dat".to_string());
    };
    let prefix = level_dat
        .strip_suffix("level.dat")
        .unwrap_or("")
        .to_string();
    if candidates
        .iter()
        .filter(|name| name.matches('/').count() == level_dat.matches('/').count())
        .count()
        > 1
    {
        return Err("zip 中有多个同层级的世界，无法确定导入哪一个".to_string());
    }

    let mut total = 0u64;
    let mut count = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;
        count += 1;
        if count > MAX_IMPORT_ENTRIES {
            return Err("zip 条目过多，已拒绝导入".to_string());
        }
        let name = normalize_zip_name(entry.name());
        if !name.starts_with(&prefix) {
            continue;
        }
        let rel = &name[prefix.len()..];
        if rel.is_empty() {
            continue;
        }
        let checked = if entry.is_dir() {
            rel.strip_suffix('/').unwrap_or(rel)
        } else {
            rel
        };
        validate_relative(checked)?;

        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                return Err(format!("zip 中包含符号链接，已拒绝: {rel}"));
            }
        }
        total = total.saturating_add(entry.size());
        if total > MAX_IMPORT_BYTES {
            return Err("zip 解压后过大，已拒绝导入".to_string());
        }

        let target = stage.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| format!("创建目录失败 {}: {e}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
        }
        let mut out =
            File::create(&target).map_err(|e| format!("创建 {} 失败: {e}", target.display()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("解压 {} 失败: {e}", rel))?;
    }
    Ok(())
}

fn normalize_zip_name(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_string()
}

fn validate_relative(rel: &str) -> Result<(), String> {
    if rel.is_empty() || rel.contains('\0') {
        return Err("zip 中包含非法路径".to_string());
    }
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!("zip 中包含非法路径: {rel}"));
        }
        if part.contains(':') {
            return Err(format!("zip 中包含非法路径: {rel}"));
        }
    }
    Ok(())
}

fn safe_filename(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim().trim_matches('.');
    if name.is_empty() {
        "world".to_string()
    } else {
        name.chars().take(64).collect()
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn map_directory_cannot_be_a_symlink_outside_library() {
        let root = std::env::temp_dir().join(format!("deepslate-map-escape-{}", Uuid::new_v4()));
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().into_owned();
        ensure_root(&config).unwrap();
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, super::root(&config).join("escape")).unwrap();
        assert!(map_dir(&config, "escape").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_config() -> (AppConfig, PathBuf) {
        let root = std::env::temp_dir().join(format!("deepslate-maps-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let mut config = AppConfig::default();
        config.root_dir = root.to_string_lossy().to_string();
        (config, root)
    }

    fn fixture_world(root: &Path) -> PathBuf {
        let world = root.join("fixture-world");
        fs::create_dir_all(&world).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat");
        fs::copy(fixture, world.join("level.dat")).unwrap();
        world
    }

    #[test]
    fn pending_client_world_is_hidden_until_verified_and_retains_data_on_failure() {
        let (config, root) = temp_config();
        ensure_root(&config).unwrap();
        let id = new_id();
        let info = GeneratorInfo {
            kind: "flat".into(),
            seed: "42".into(),
        };
        begin_client_creation(&config, &id, "test", &info).unwrap();
        let dir = super::root(&config).join(&id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("level.dat"), b"incomplete").unwrap();
        assert!(finish_client_creation(&config, &id).is_err());
        assert!(pending_creation(&config, &id).exists());
        assert!(dir.join("level.dat").exists());
        assert!(list(&config).unwrap().is_empty());
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat"),
            dir.join("level.dat"),
        )
        .unwrap();
        let generator = crate::nbt::Tag::Compound(
            [(
                String::from("type"),
                crate::nbt::Tag::String("minecraft:flat".into()),
            )]
            .into(),
        );
        let overworld = crate::nbt::Tag::Compound([(String::from("generator"), generator)].into());
        let dimensions =
            crate::nbt::Tag::Compound([(String::from("minecraft:overworld"), overworld)].into());
        let data = crate::nbt::Tag::Compound(
            [
                (String::from("seed"), crate::nbt::Tag::Long(42)),
                (String::from("dimensions"), dimensions),
            ]
            .into(),
        );
        let nbt = crate::nbt::NbtFile {
            root_name: String::new(),
            root: crate::nbt::Tag::Compound([(String::from("data"), data)].into()),
        };
        fs::create_dir_all(dir.join("data/minecraft")).unwrap();
        nbt.write_gzip(&dir.join("data/minecraft/world_gen_settings.dat"))
            .unwrap();
        assert!(list_pending_creations(&config).unwrap()[0]
            .problem
            .is_none());
        let guard = lock_library(&config).unwrap();
        assert!(recover_client_creation(&config, &id).is_err());
        drop(guard);
        recover_client_creation(&config, &id).unwrap();
        assert!(!pending_creation(&config, &id).exists());
        assert_eq!(
            list(&config).unwrap()[0]
                .manifest
                .generator
                .as_ref()
                .unwrap()
                .seed,
            "42"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_import_preserves_nested_directories() {
        let (config, root) = temp_config();
        let world = fixture_world(&root);
        fs::create_dir_all(world.join("region")).unwrap();
        fs::write(world.join("region/r.0.0.mca"), b"data").unwrap();
        let imported = import_dir(&config, &world).unwrap();
        let archive = export_zip(&config, &imported.id, None).unwrap();
        let restored = import_zip(&config, &archive).unwrap();
        assert_eq!(
            fs::read(
                root.join("maps")
                    .join(&restored.id)
                    .join("region/r.0.0.mca")
            )
            .unwrap(),
            b"data"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn map_operations_refuse_live_library_lock() {
        let (config, root) = temp_config();
        let world = fixture_world(&root);
        let imported = import_dir(&config, &world).unwrap();
        let lock = lock_library(&config).unwrap();
        assert!(rename(&config, &imported.id, "new").is_err());
        assert!(duplicate(&config, &imported.id, None).is_err());
        assert!(move_to_trash(&config, &imported.id).is_err());
        assert!(export_zip(&config, &imported.id, None).is_err());
        drop(lock);
        assert!(rename(&config, &imported.id, "new").is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn map_library_lock_is_inherited_by_spawned_game() {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        let (config, root) = temp_config();
        let guard = lock_library(&config).unwrap();
        let inherited = duplicate_inheritable_lock(&guard).unwrap();
        let fd = inherited.as_raw_fd();
        let mut command = std::process::Command::new("sleep");
        command.arg("10");
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(inherited);
        drop(guard);
        assert!(lock_library(&config).is_err());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(lock_library(&config).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_backup_snapshots_world_before_game_starts() {
        let (config, root) = temp_config();
        let world = fixture_world(&root);
        let imported = import_dir(&config, &world).unwrap();
        let guard = lock_library(&config).unwrap();
        backup_for_launch(&config, &imported.id, &guard).unwrap();
        assert_eq!(list_backups(&config, &imported.id).unwrap().len(), 1);
        drop(guard);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_snapshot_does_not_appear_as_backup() {
        let (config, root) = temp_config();
        let world = fixture_world(&root);
        let imported = import_dir(&config, &world).unwrap();
        let backups = backups_root(&config, &imported.id).unwrap();
        fs::create_dir_all(backups.join(".backup-incomplete")).unwrap();
        assert!(list_backups(&config, &imported.id).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn import_list_rename_duplicate_export_trash() {
        let (config, root) = temp_config();
        let world = fixture_world(&root);
        let imported = import_dir(&config, &world).unwrap();
        assert_eq!(imported.id.len(), 36);
        assert!(root
            .join("maps")
            .join(&imported.id)
            .join(MANIFEST_NAME)
            .is_file());

        let listed = list(&config).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, imported.id);

        rename(&config, &imported.id, "重命名后的世界").unwrap();
        let renamed = list(&config).unwrap();
        assert_eq!(renamed[0].name, "重命名后的世界");
        assert_eq!(renamed[0].manifest.name, "重命名后的世界");

        let duplicate_id = duplicate(&config, &imported.id, Some("副本世界")).unwrap();
        let listed = list(&config).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|m| m.id == duplicate_id));
        assert!(listed.iter().any(|m| m.name == "副本世界"));

        let zip_path = export_zip(&config, &imported.id, None).unwrap();
        assert!(zip_path.is_file());
        let reimported = import_zip(&config, &zip_path).unwrap();
        assert_eq!(reimported.name, "重命名后的世界");

        let trashed = move_to_trash(&config, &duplicate_id).unwrap();
        assert!(trashed.is_dir());
        assert_eq!(list(&config).unwrap().len(), 2);
        assert_eq!(empty_trash(&config).unwrap(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_zip_without_level_dat() {
        let (config, root) = temp_config();
        let zip_path = root.join("bad.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("hello.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"hi").unwrap();
        zip.finish().unwrap();
        let err = import_zip(&config, &zip_path).unwrap_err();
        assert!(err.contains("level.dat"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_path_traversal_in_zip() {
        let (config, root) = temp_config();
        let zip_path = root.join("bad.zip");
        let file = File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("world/level.dat", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"not nbt").unwrap();
        zip.start_file("world/../../evil.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"evil").unwrap();
        zip.finish().unwrap();
        let err = import_zip(&config, &zip_path).unwrap_err();
        assert!(err.contains("非法路径"));
        assert!(!root.join("evil.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn adopts_generated_world_with_source_and_generator() {
        let (config, root) = temp_config();
        let source = root.join("generated");
        fs::create_dir_all(&source).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/level_26_3.dat");
        fs::copy(fixture, source.join("level.dat")).unwrap();

        let map = adopt_generated_dir(
            &config,
            &source,
            Some(GeneratorInfo {
                kind: "flat".to_string(),
                seed: "42".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(map.manifest.source, MapSource::Generated);
        assert_eq!(map.manifest.generator.as_ref().unwrap().kind, "flat");
        assert_eq!(map.manifest.generator.as_ref().unwrap().seed, "42");
        assert!(!source.exists());
        assert!(root.join("maps").join(&map.id).join("level.dat").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn backup_and_restore_map() {
        let (config, root) = temp_config();
        let world = fixture_world(&root);
        let map = import_dir(&config, &world).unwrap();
        let name = backup(&config, &map.id).unwrap();
        assert_eq!(list_backups(&config, &map.id).unwrap(), vec![name.clone()]);

        rename(&config, &map.id, "备份后改名").unwrap();
        assert_eq!(list(&config).unwrap()[0].name, "备份后改名");
        restore_backup(&config, &map.id, &name).unwrap();
        assert_ne!(list(&config).unwrap()[0].name, "备份后改名");

        delete_backup(&config, &map.id, &name).unwrap();
        assert!(list_backups(&config, &map.id).unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
