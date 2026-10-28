//! Fabric/Quilt 模组目录管理。
//!
//! - 列表：`.jar` / `.jar.disabled`，读取 `fabric.mod.json` 的 id/name/version/
//!   description/authors/environment；非 Fabric jar 也会列出（用文件名兜底）
//! - 启用/禁用：在 `.jar` 与 `.jar.disabled` 之间改名（Fabric 只加载 `.jar`）
//! - 删除：移动到 `mods/.deepslate-trash/`，可恢复
//!
//! 不做依赖解析与下载，只管理本地文件。

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use zip::ZipArchive;

pub const TRASH_DIR: &str = ".deepslate-trash";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInfo {
    /// 磁盘上的文件名（含 `.disabled`），用于改名/删除
    pub file_name: String,
    pub enabled: bool,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub environment: String,
    pub size_bytes: u64,
    pub path: PathBuf,
    pub metadata_error: Option<String>,
    /// 是否是带 fabric.mod.json 的 Fabric 模组
    pub has_fabric_meta: bool,
}

impl ModInfo {
    pub fn size_text(&self) -> String {
        let bytes = self.size_bytes;
        if bytes >= 1024 * 1024 {
            format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
        } else if bytes >= 1024 {
            format!("{:.1} KiB", bytes as f64 / 1024.0)
        } else {
            format!("{bytes} B")
        }
    }

    pub fn environment_label(&self) -> &'static str {
        match self.environment.as_str() {
            "client" => "客户端",
            "server" => "服务端",
            "*" | "" => "通用",
            _ => "未知",
        }
    }
}

pub fn mods_dir(config: &crate::config::AppConfig, _version: &str) -> PathBuf {
    if config.enhanced_experience {
        return crate::runtime::paths_for_mode(config, crate::runtime::RuntimeMode::Enhanced)
            .instance_dir
            .join("mods");
    }
    crate::runtime::paths_for_mode(config, crate::runtime::RuntimeMode::Normal)
        .instance_dir
        .join("mods")
}

pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 mods 目录失败 {}: {e}", dir.display()))
}

pub fn list(dir: &Path) -> Vec<ModInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut mods = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if file_name.starts_with('.') || !path.is_file() {
            continue;
        }
        let enabled = if file_name.ends_with(".jar") {
            true
        } else if file_name.ends_with(".jar.disabled") {
            false
        } else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let size_bytes = meta.len();
        let (fabric, error) = read_fabric_meta(&path);
        let fallback_name = file_name
            .trim_end_matches(".disabled")
            .trim_end_matches(".jar")
            .to_string();
        let info = match fabric {
            Some(meta) => ModInfo {
                file_name: file_name.to_string(),
                enabled,
                id: meta.id,
                name: if meta.name.is_empty() {
                    fallback_name
                } else {
                    meta.name
                },
                version: meta.version,
                description: meta.description,
                authors: meta.authors,
                environment: meta.environment,
                size_bytes,
                path,
                metadata_error: None,
                has_fabric_meta: true,
            },
            None => ModInfo {
                file_name: file_name.to_string(),
                enabled,
                id: String::new(),
                name: fallback_name,
                version: String::new(),
                description: String::new(),
                authors: Vec::new(),
                environment: String::new(),
                size_bytes,
                path,
                metadata_error: error,
                has_fabric_meta: false,
            },
        };
        mods.push(info);
    }
    mods.sort_by(|a, b| {
        b.enabled
            .cmp(&a.enabled)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    mods
}

struct FabricMeta {
    id: String,
    name: String,
    version: String,
    description: String,
    authors: Vec<String>,
    environment: String,
}

fn read_fabric_meta(path: &Path) -> (Option<FabricMeta>, Option<String>) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return (None, Some(format!("打开失败: {e}"))),
    };
    let mut archive = match ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => return (None, Some(format!("不是有效 jar: {e}"))),
    };
    let entry = match archive.by_name("fabric.mod.json") {
        Ok(e) => e,
        Err(_) => return (None, None),
    };
    const MAX_METADATA_BYTES: u64 = 1024 * 1024;
    let mut text = String::new();
    if let Err(e) = entry.take(MAX_METADATA_BYTES + 1).read_to_string(&mut text) {
        return (None, Some(format!("读取 fabric.mod.json 失败: {e}")));
    }
    if text.len() as u64 > MAX_METADATA_BYTES {
        return (None, Some("fabric.mod.json 过大".to_string()));
    }
    let value: Value = match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(v) => v,
        Err(e) => return (None, Some(format!("解析 fabric.mod.json 失败: {e}"))),
    };
    let get_str = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let authors = match value.get("authors") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    };
    (
        Some(FabricMeta {
            id: get_str("id"),
            name: get_str("name"),
            version: get_str("version"),
            description: get_str("description"),
            authors,
            environment: get_str("environment"),
        }),
        None,
    )
}

fn safe_file_name(file_name: &str) -> Result<(), String> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name == "."
        || file_name == ".."
    {
        return Err(format!("非法模组文件名: {file_name}"));
    }
    Ok(())
}

/// 启用/禁用：只改后缀，不改文件内容。
pub fn set_enabled(dir: &Path, file_name: &str, enabled: bool) -> Result<(), String> {
    safe_file_name(file_name)?;
    let current = dir.join(file_name);
    if !current.is_file() {
        return Err(format!("模组不存在: {file_name}"));
    }
    let target_name = if enabled {
        if file_name.ends_with(".jar") {
            return Ok(());
        }
        file_name
            .strip_suffix(".disabled")
            .ok_or_else(|| format!("无法识别的模组后缀: {file_name}"))?
            .to_string()
    } else {
        if file_name.ends_with(".disabled") {
            return Ok(());
        }
        format!("{file_name}.disabled")
    };
    let target = dir.join(&target_name);
    if target.exists() {
        return Err(format!("目标文件已存在: {target_name}"));
    }
    std::fs::rename(&current, &target)
        .map_err(|e| format!("重命名 {file_name} -> {target_name} 失败: {e}"))
}

pub fn delete_to_trash(dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    safe_file_name(file_name)?;
    let source = dir.join(file_name);
    if !source.is_file() {
        return Err(format!("模组不存在: {file_name}"));
    }
    let trash = dir.join(TRASH_DIR);
    std::fs::create_dir_all(&trash).map_err(|e| format!("创建模组回收站失败: {e}"))?;
    let target = trash.join(format!("{}-{file_name}", crate::saves::now_ms()));
    std::fs::rename(&source, &target).map_err(|e| format!("移动 {file_name} 到回收站失败: {e}"))?;
    Ok(target)
}

pub fn empty_trash(dir: &Path) -> Result<usize, String> {
    let trash = dir.join(TRASH_DIR);
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
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
        } else {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("deepslate-mods-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_mod(dir: &Path, file_name: &str, meta: Option<&str>) {
        let file = std::fs::File::create(dir.join(file_name)).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        if let Some(meta) = meta {
            zip.start_file("fabric.mod.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(meta.as_bytes()).unwrap();
        }
        zip.start_file("dummy.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn rejects_oversized_fabric_metadata() {
        let dir = temp_dir("large-meta");
        let text = format!(
            "{{\"id\":\"x\",\"description\":\"{}\"}}",
            "a".repeat(1024 * 1024)
        );
        write_mod(&dir, "large.jar", Some(&text));
        let (_, error) = read_fabric_meta(&dir.join("large.jar"));
        assert!(error.unwrap().contains("过大"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lists_and_toggles_fabric_mods() {
        let dir = temp_dir("list");
        write_mod(
            &dir,
            "example.jar",
            Some(
                r#"{"id":"example","name":"Example Mod","version":"1.2.3","description":"demo","authors":["A","B"],"environment":"client"}"#,
            ),
        );
        write_mod(&dir, "plain.jar", None);
        write_mod(&dir, "off.jar.disabled", None);

        let mods = list(&dir);
        assert_eq!(mods.len(), 3);
        let example = mods.iter().find(|m| m.id == "example").unwrap();
        assert_eq!(example.name, "Example Mod");
        assert_eq!(example.version, "1.2.3");
        assert_eq!(example.authors, vec!["A", "B"]);
        assert_eq!(example.environment_label(), "客户端");
        assert!(example.enabled);
        assert!(example.has_fabric_meta);

        let plain = mods.iter().find(|m| m.file_name == "plain.jar").unwrap();
        assert!(!plain.has_fabric_meta);
        assert_eq!(plain.name, "plain");
        let off = mods
            .iter()
            .find(|m| m.file_name == "off.jar.disabled")
            .unwrap();
        assert!(!off.enabled);
        assert_eq!(off.name, "off");

        set_enabled(&dir, "example.jar", false).unwrap();
        assert!(dir.join("example.jar.disabled").is_file());
        set_enabled(&dir, "example.jar.disabled", true).unwrap();
        assert!(dir.join("example.jar").is_file());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_moves_to_trash_and_can_be_emptied() {
        let dir = temp_dir("trash");
        write_mod(&dir, "bad.jar", None);
        let moved = delete_to_trash(&dir, "bad.jar").unwrap();
        assert!(moved.is_file());
        assert!(list(&dir).is_empty());
        assert_eq!(empty_trash(&dir).unwrap(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = temp_dir("safe");
        assert!(set_enabled(&dir, "../evil.jar", false).is_err());
        assert!(delete_to_trash(&dir, "../evil.jar").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
