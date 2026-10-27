//! 标题界面左下角的启动器名称注入（vanilla）。
//!
//! 26.3 的 `TitleScreen` 用 `SharedConstants.getCurrentVersion().name()` 拼出
//! `Minecraft <name>`。`DetectedVersion.tryDetectVersion()` 通过
//! `DetectedVersion.class.getResourceAsStream("/version.json")` 读取，
//! 因此把一份改名后的 `version.json` 放在 classpath **最前面** 就能显示成
//! `Minecraft 26.3 | Deepslate`，无需修改 Mojang 的 jar。
//!
//! 已验证：
//! - vanilla 26.3：SDL app version / 标题版本文字变为自定义名称；
//! - Fabric 26.3：Knot 的游戏类加载器不暴露额外 classpath 资源，因此该覆盖
//!   不生效（标题仍显示原版版本）。这里保持“尽力而为”，不修改游戏 jar。

use std::io::Read;
use std::path::{Path, PathBuf};

use zip::ZipArchive;

const MAX_VERSION_JSON: u64 = 1024 * 1024;

/// 从客户端 jar 的 `version.json` 生成改名后的 JSON。
pub fn brand_version_json(original: &str, brand: &str) -> Result<String, String> {
    let mut value: serde_json::Value =
        serde_json::from_str(original).map_err(|e| format!("解析 version.json 失败: {e}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "version.json 根节点不是对象".to_string())?;
    let original_name = object
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| object.get("id").and_then(|v| v.as_str()))
        .unwrap_or("Minecraft")
        .to_string();
    let brand = brand.trim();
    let name = if brand.is_empty() {
        original_name
    } else {
        format!("{original_name} | {brand}")
    };
    object.insert("name".to_string(), serde_json::Value::String(name));
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

/// 生成 `<instance>/deepslate/branding/version.json`，返回要插入 classpath 的目录。
pub fn write_overlay(game_jar: &Path, instance_dir: &Path, brand: &str) -> Result<PathBuf, String> {
    if brand.trim().is_empty() {
        return Err("品牌名为空".to_string());
    }
    let original = read_zip_text(game_jar, "version.json")?;
    let branded = brand_version_json(&original, brand)?;

    let dir = instance_dir.join("deepslate").join("branding");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("创建品牌目录失败 {}: {e}", dir.display()))?;
    let path = dir.join("version.json");
    let unchanged = std::fs::read_to_string(&path)
        .map(|existing| existing == branded)
        .unwrap_or(false);
    if !unchanged {
        let tmp = dir.join("version.json.part");
        std::fs::write(&tmp, branded.as_bytes())
            .map_err(|e| format!("写入品牌 version.json 失败 {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("替换品牌 version.json 失败 {}: {e}", path.display()))?;
    }
    Ok(dir)
}

fn read_zip_text(jar: &Path, entry_name: &str) -> Result<String, String> {
    let file = std::fs::File::open(jar)
        .map_err(|e| format!("打开客户端 jar 失败 {}: {e}", jar.display()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| format!("读取客户端 jar 失败 {}: {e}", jar.display()))?;
    let mut entry = archive
        .by_name(entry_name)
        .map_err(|_| format!("客户端 jar 中没有 {entry_name}（{}）", jar.display()))?;
    if entry.size() > MAX_VERSION_JSON {
        return Err(format!("{entry_name} 过大（{} 字节）", entry.size()));
    }
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|e| format!("读取 {entry_name} 失败: {e}"))?;
    Ok(text)
}

/// 把品牌目录放到 classpath 最前面（目录本身就是合法的 classpath 条目）。
pub fn insert_branding_classpath(classpath: &mut Vec<PathBuf>, dir: PathBuf) {
    if !classpath.iter().any(|p| p == &dir) {
        classpath.insert(0, dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deepslate-branding-test-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn brands_name_without_touching_version_data() {
        let original = r#"{
            "id": "26.3",
            "name": "26.3",
            "world_version": 5023,
            "series_id": "main",
            "protocol_version": 777,
            "pack_version": {"resource_major": 97, "resource_minor": 1, "data_major": 121, "data_minor": 0},
            "build_time": "2026-09-15T11:20:48+00:00",
            "stable": true
        }"#;
        let branded: serde_json::Value =
            serde_json::from_str(&brand_version_json(original, "Deepslate").unwrap()).unwrap();
        assert_eq!(branded["name"], "26.3 | Deepslate");
        assert_eq!(branded["id"], "26.3");
        assert_eq!(branded["world_version"], 5023);
        assert_eq!(branded["protocol_version"], 777);
        assert_eq!(branded["pack_version"]["resource_major"], 97);

        let plain: serde_json::Value =
            serde_json::from_str(&brand_version_json(original, "  ").unwrap()).unwrap();
        assert_eq!(plain["name"], "26.3");
    }

    #[test]
    fn writes_overlay_from_jar_version_json() {
        let dir = temp_dir("jar");
        let jar = dir.join("client.jar");
        {
            let file = std::fs::File::create(&jar).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("version.json", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(br#"{"id":"26.3","name":"26.3","world_version":5023}"#)
                .unwrap();
            zip.finish().unwrap();
        }
        let instance = dir.join("instance");
        let overlay = write_overlay(&jar, &instance, "Deepslate").unwrap();
        assert_eq!(overlay, instance.join("deepslate").join("branding"));
        let text = std::fs::read_to_string(overlay.join("version.json")).unwrap();
        assert!(text.contains("26.3 | Deepslate"));
        // 重复生成是幂等的
        assert_eq!(
            write_overlay(&jar, &instance, "Deepslate").unwrap(),
            overlay
        );

        let mut cp = vec![PathBuf::from("/libs/a.jar")];
        insert_branding_classpath(&mut cp, overlay.clone());
        assert_eq!(cp[0], overlay);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
