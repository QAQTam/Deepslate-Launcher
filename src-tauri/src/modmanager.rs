use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Mod 信息，由 mod.json 解析得到
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModInfo {
    pub id: String,
    pub version: String,
    pub entry: String,
    pub file_name: String,
    pub enabled: bool,
}

/// 扫描 game_dir/mods/ 目录，列出所有已安装的 mod
#[tauri::command]
pub fn list_mods(game_dir: String) -> Result<Vec<ModInfo>, String> {
    let mods_dir = Path::new(&game_dir).join("mods");
    if !mods_dir.exists() {
        fs::create_dir_all(&mods_dir).map_err(|e| format!("Failed to create mods dir: {}", e))?;
        return Ok(Vec::new());
    }

    let mut mods = Vec::new();

    // 扫描 .jar 和 .jar.disabled 文件
    let entries = fs::read_dir(&mods_dir)
        .map_err(|e| format!("Failed to read mods dir: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();

        let (jar_path, enabled) = if file_name.ends_with(".jar.disabled") {
            (path.clone(), false)
        } else if file_name.ends_with(".jar") {
            (path.clone(), true)
        } else {
            continue;
        };

        // 读取 jar 中的 mod.json
        match read_mod_manifest(&jar_path) {
            Ok((id, version, entry_class)) => {
                mods.push(ModInfo {
                    id,
                    version,
                    entry: entry_class,
                    file_name,
                    enabled,
                });
            }
            Err(e) => {
                eprintln!("[GhastLoader] Skipping {} — {}", file_name, e);
            }
        }
    }

    Ok(mods)
}

/// 启用 / 禁用一个 mod（通过重命名 .jar ↔ .jar.disabled）
#[tauri::command]
pub fn toggle_mod(game_dir: String, mod_id: String, enabled: bool) -> Result<(), String> {
    let mods_dir = Path::new(&game_dir).join("mods");
    let _old_suffix = if enabled { ".jar.disabled" } else { ".jar" };
    let new_suffix = if enabled { ".jar" } else { ".jar.disabled" };

    // 找到对应文件
    let entries = fs::read_dir(&mods_dir).map_err(|e| format!("{}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();

        if (enabled && file_name.ends_with(".jar.disabled"))
            || (!enabled && file_name.ends_with(".jar") && !file_name.ends_with(".jar.disabled"))
        {
            // 检查 mod.json 中的 id
            if let Ok((id, _, _)) = read_mod_manifest(&path) {
                if id == mod_id {
                    let new_path = path.with_extension("");
                    let new_name = new_path.file_name().unwrap().to_string_lossy().to_string()
                        + new_suffix;
                    let new_full = path.parent().unwrap().join(new_name);
                    fs::rename(&path, &new_full)
                        .map_err(|e| format!("Failed to rename: {}", e))?;
                    return Ok(());
                }
            }
        }
    }

    Err(format!("Mod '{}' not found", mod_id))
}

/// 从 jar 中读取 mod.json 清单
fn read_mod_manifest(jar_path: &Path) -> Result<(String, String, String), String> {
    let file = fs::File::open(jar_path).map_err(|e| format!("Cannot open: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Not a valid zip/jar: {}", e))?;

    let entry = archive
        .by_name("mod.json")
        .map_err(|_| "No mod.json found".to_string())?;

    let json: serde_json::Value =
        serde_json::from_reader(entry).map_err(|e| format!("Invalid mod.json: {}", e))?;

    let id = json["id"]
        .as_str()
        .ok_or("mod.json missing 'id'")?
        .to_string();
    let version = json["version"].as_str().unwrap_or("unknown").to_string();
    let entry = json["entry"]
        .as_str()
        .ok_or("mod.json missing 'entry'")?
        .to_string();

    Ok((id, version, entry))
}

/// 安装 GhastLoader agent jar 到 game_dir/ghastloader/
/// 在启动器首次运行时调用，或作为资源部署的一部分
#[tauri::command]
pub fn install_ghastloader(game_dir: String) -> Result<String, String> {
    // 从启动器资源目录复制 agent jar
    let target_dir = Path::new(&game_dir).join("ghastloader");
    fs::create_dir_all(&target_dir).map_err(|e| format!("{}", e))?;

    let target_jar = target_dir.join("ghastloader.jar");

    // 如果已存在且版本相同，跳过
    if target_jar.exists() {
        return Ok("already_installed".to_string());
    }

    // 尝试从几个可能的位置获取 agent jar
    let candidates = [
        // 开发环境：构建产物
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("..")
            .join("GhastLoader")
            .join("build")
            .join("libs")
            .join("ghastloader-1.0.0.jar"),
        // 生产环境：与可执行文件同目录
        std::env::current_exe()
            .unwrap_or_default()
            .parent()
            .unwrap_or(Path::new("."))
            .join("ghastloader.jar"),
        // 生产环境：在 resources 中
        std::env::current_exe()
            .unwrap_or_default()
            .parent()
            .unwrap_or(Path::new("."))
            .join("resources")
            .join("ghastloader.jar"),
    ];

    for src in &candidates {
        if src.exists() {
            fs::copy(src, &target_jar)
                .map_err(|e| format!("Copy failed: {}", e))?;
            println!("[GhastLoader] Installed: {} → {}", src.display(), target_jar.display());
            return Ok("installed".to_string());
        }
    }

    Err(format!(
        "GhastLoader jar not found. Searched: {:?}",
        candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()
    ))
}
