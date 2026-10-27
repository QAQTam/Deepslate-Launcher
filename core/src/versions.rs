//! 本地版本扫描：给 GUI 提供「已安装」列表，并识别加载器类型。
//!
//! 只读取 `versions/*/*.json`，不下载任何东西。加载器版本（Fabric/Quilt/Forge）
//! 通常没有自己的 jar，启动时沿 `inheritsFrom` 回溯到原版 jar，因此这里也用
//! 同样的方式判断「是否可启动」。

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::config::{resolve_game_dir, AppConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVersion {
    pub id: String,
    /// "vanilla" | "fabric" | "quilt" | "forge" | "neoforge" | "unknown"
    pub kind: String,
    /// 基础游戏版本（来自 inheritsFrom，原版则为自身 id）
    pub game_version: String,
    /// Fabric/Quilt/Forge 的加载器版本
    pub loader_version: Option<String>,
    /// 给 GUI 的展示名，例如 "1.21.1 · Fabric 0.16.14"
    pub display_name: String,
    pub main_class: String,
    /// 是否能解析到客户端 jar（自己或继承链上的父版本）
    pub has_jar: bool,
    /// 安装目录最后修改时间（秒），用于排序
    pub installed_at: u64,
}

impl LocalVersion {
    /// 是否带 mod 加载器（决定「mods 目录」是否有意义）
    pub fn has_loader(&self) -> bool {
        self.kind != "vanilla" && self.kind != "unknown"
    }
}

pub fn scan_config(config: &AppConfig) -> Vec<LocalVersion> {
    scan(&resolve_game_dir(config))
}

/// 扫描 `<game_dir>/versions/`，按安装时间倒序返回。
pub fn scan(game_dir: &Path) -> Vec<LocalVersion> {
    let versions_dir = game_dir.join("versions");
    let Ok(entries) = std::fs::read_dir(&versions_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let json_path = dir.join(format!("{dir_name}.json"));
        let Ok(text) = std::fs::read_to_string(&json_path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };

        let id = json
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(dir_name)
            .to_string();
        let main_class = json
            .get("mainClass")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let inherits_from = json
            .get("inheritsFrom")
            .and_then(Value::as_str)
            .map(str::to_string);

        let kind = detect_kind(&id, &main_class);
        let game_version = inherits_from.clone().unwrap_or_else(|| id.clone());
        let loader_version = loader_version_of(&id, &kind, &game_version);
        let display_name = display_name(&kind, &game_version, &id, loader_version.as_deref());
        let has_jar = jar_resolvable(game_dir, &id, &mut std::collections::HashSet::new());
        let installed_at = std::fs::metadata(&json_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        out.push(LocalVersion {
            id,
            kind,
            game_version,
            loader_version,
            display_name,
            main_class,
            has_jar,
            installed_at,
        });
    }

    out.sort_by(|a, b| {
        b.installed_at
            .cmp(&a.installed_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

fn detect_kind(id: &str, main_class: &str) -> String {
    let id_lower = id.to_lowercase();
    let main_lower = main_class.to_lowercase();

    if id_lower.starts_with("fabric-loader-") || main_lower.contains("fabricmc.loader") {
        "fabric".to_string()
    } else if id_lower.starts_with("quilt-loader-") || main_lower.contains("quiltmc") {
        "quilt".to_string()
    } else if id_lower.starts_with("neoforge-") || main_lower.contains("neoforged") {
        "neoforge".to_string()
    } else if id_lower.starts_with("forge-") || main_lower.contains("minecraftforge") {
        "forge".to_string()
    } else if main_lower == "net.minecraft.client.main.main" {
        "vanilla".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Fabric/Quilt 的 id 形如 `fabric-loader-<loader>-<game>`；game 里也可能有 `-`，
/// 因此优先用 inheritsFrom 反推，而不是粗暴 split。
fn loader_version_of(id: &str, kind: &str, game_version: &str) -> Option<String> {
    let prefix = match kind {
        "fabric" => "fabric-loader-",
        "quilt" => "quilt-loader-",
        _ => return None,
    };
    let rest = id.strip_prefix(prefix)?;
    let suffix = format!("-{game_version}");
    Some(rest.strip_suffix(&suffix).unwrap_or(rest).to_string())
}

fn display_name(kind: &str, game_version: &str, id: &str, loader: Option<&str>) -> String {
    match (kind, loader) {
        ("fabric", Some(v)) => format!("{game_version} · Fabric {v}"),
        ("quilt", Some(v)) => format!("{game_version} · Quilt {v}"),
        ("forge", _) => format!("{game_version} · Forge"),
        ("neoforge", _) => format!("{game_version} · NeoForge"),
        _ => id.to_string(),
    }
}

fn jar_resolvable(game_dir: &Path, id: &str, seen: &mut std::collections::HashSet<String>) -> bool {
    if !seen.insert(id.to_string()) {
        return false;
    }
    let dir: PathBuf = game_dir.join("versions").join(id);
    if dir.join(format!("{id}.jar")).is_file() {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(dir.join(format!("{id}.json"))) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    if let Some(jar) = json.get("jar").and_then(Value::as_str) {
        if jar_resolvable(game_dir, jar, seen) {
            return true;
        }
    }
    if let Some(parent) = json.get("inheritsFrom").and_then(Value::as_str) {
        return jar_resolvable(game_dir, parent, seen);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deepslate-versions-test-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_version(dir: &Path, id: &str, json: &str) {
        let vdir = dir.join("versions").join(id);
        fs::create_dir_all(&vdir).unwrap();
        fs::write(vdir.join(format!("{id}.json")), json).unwrap();
    }

    #[test]
    fn scans_vanilla_and_fabric() {
        let dir = temp_dir("scan");
        write_version(
            &dir,
            "1.21.1",
            r#"{"id":"1.21.1","mainClass":"net.minecraft.client.main.Main","type":"release"}"#,
        );
        fs::write(dir.join("versions/1.21.1/1.21.1.jar"), b"jar").unwrap();
        write_version(
            &dir,
            "fabric-loader-0.16.14-1.21.1",
            r#"{
                "id":"fabric-loader-0.16.14-1.21.1",
                "inheritsFrom":"1.21.1",
                "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                "type":"release"
            }"#,
        );

        let versions = scan(&dir);
        assert_eq!(versions.len(), 2);

        let fabric = versions.iter().find(|v| v.kind == "fabric").unwrap();
        assert_eq!(fabric.game_version, "1.21.1");
        assert_eq!(fabric.loader_version.as_deref(), Some("0.16.14"));
        assert_eq!(fabric.display_name, "1.21.1 · Fabric 0.16.14");
        assert!(fabric.has_jar, "应沿 inheritsFrom 找到原版 jar");
        assert!(fabric.has_loader());

        let vanilla = versions.iter().find(|v| v.id == "1.21.1").unwrap();
        assert_eq!(vanilla.kind, "vanilla");
        assert!(vanilla.has_jar);
        assert!(!vanilla.has_loader());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fabric_without_base_jar_is_not_launchable() {
        let dir = temp_dir("nojar");
        write_version(
            &dir,
            "fabric-loader-0.16.14-1.21.1",
            r#"{
                "id":"fabric-loader-0.16.14-1.21.1",
                "inheritsFrom":"1.21.1",
                "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient"
            }"#,
        );
        write_version(
            &dir,
            "1.21.1",
            r#"{"id":"1.21.1","mainClass":"net.minecraft.client.main.Main"}"#,
        );
        let versions = scan(&dir);
        let fabric = versions.iter().find(|v| v.kind == "fabric").unwrap();
        assert!(!fabric.has_jar);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loader_version_handles_dashed_game_versions() {
        assert_eq!(
            loader_version_of("fabric-loader-0.16.14-1.21.1-pre1", "fabric", "1.21.1-pre1")
                .as_deref(),
            Some("0.16.14")
        );
        assert_eq!(
            loader_version_of("fabric-loader-0.17.0-beta.1-1.21.1", "fabric", "1.21.1").as_deref(),
            Some("0.17.0-beta.1")
        );
        assert_eq!(loader_version_of("1.21.1", "vanilla", "1.21.1"), None);
    }
}
