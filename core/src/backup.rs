use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{self, AppConfig};

/// 每个版本保留的备份数量
const KEEP_BACKUPS: usize = 5;

/// 版本的实例目录（--gameDir）：版本隔离时为 root/versions/{v}，否则为 root/game。
/// 存档、options.txt、日志等实例数据都落在这里；jar/库/资源仍在共享的 game_dir。
fn instance_root(config: &AppConfig, version: &str) -> PathBuf {
    config::get_version_root(config, version)
}

/// 版本存档的实际存储位置：
/// - saves_isolation=true: root/saves/{version}（实例目录中的 saves 是指向这里的符号链接）
/// - saves_isolation=false: 实例目录下的真实 saves/
pub fn version_saves_dir(config: &AppConfig, version: &str) -> PathBuf {
    if config.saves_isolation {
        PathBuf::from(&config.root_dir).join("saves").join(version)
    } else {
        instance_root(config, version).join("saves")
    }
}

/// 建立或确认版本的 saves 符号链接（实验级隔离）
/// 将实例目录下的 saves/ 链接到 root/saves/{version}/
pub async fn ensure_saves_symlink(config: &AppConfig, version: &str) -> Result<(), String> {
    if !config.saves_isolation {
        return Ok(());
    }

    let saves_target = PathBuf::from(&config.root_dir).join("saves").join(version);
    let saves_link = instance_root(config, version).join("saves");

    fs::create_dir_all(&saves_target).map_err(|e| e.to_string())?;

    if let Ok(meta) = saves_link.symlink_metadata() {
        if meta.file_type().is_symlink() {
            // 已是正确链接则直接返回；否则只删链接本身（remove_dir_all 对符号链接会失败）
            if let Ok(target) = fs::read_link(&saves_link) {
                if target == saves_target {
                    return Ok(());
                }
            }
            fs::remove_file(&saves_link).map_err(|e| e.to_string())?;
        } else if saves_link.is_dir() {
            // 真实存档目录：把内容迁移到隔离目标后再替换，绝不直接删除
            migrate_dir_contents(&saves_link, &saves_target)
                .map_err(|e| format!("迁移已有存档失败: {}", e))?;
            fs::remove_dir(&saves_link).map_err(|e| e.to_string())?;
        } else {
            fs::remove_file(&saves_link).map_err(|e| e.to_string())?;
        }
    }

    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&saves_target, &saves_link).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(&saves_target, &saves_link).map_err(|e| e.to_string())?;

    println!(
        "已建立 saves 隔离链接: {} -> {:?}",
        saves_link.display(),
        saves_target.display()
    );
    Ok(())
}

/// 把 src 目录下的内容移动到 dst（rename），目标已存在同名条目时报错而不是覆盖
fn migrate_dir_contents(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = dst.join(entry.file_name());
        if target.exists() {
            return Err(format!("目标已存在 {}，请手动合并后重试", target.display()));
        }
        fs::rename(entry.path(), &target).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 准备版本存档隔离（建立符号链接），在启动/下载前调用
pub async fn prepare(version: &str) -> Result<(), String> {
    let config = config::load()?;
    ensure_saves_symlink(&config, version).await
}

/// 存档备份目录（root/saves/{version}_backups/）
fn backups_root(config: &AppConfig, version: &str) -> PathBuf {
    PathBuf::from(&config.root_dir)
        .join("saves")
        .join(format!("{}_backups", version))
}

/// 清理超过数量的旧备份
fn cleanup_old_backups(backup_dir: &Path, keep: usize) -> Result<(), String> {
    if !backup_dir.exists() {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(backup_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.len() <= keep {
        return Ok(());
    }

    // 按修改时间排序（旧的在前）
    entries.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());

    for entry in entries.iter().take(entries.len() - keep) {
        fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
        println!("已删除旧备份: {:?}", entry.path());
    }
    Ok(())
}

/// 跨平台复制存档目录内容到 dst（dst 为尚不存在的新目录）
fn copy_saves_dir(src: &Path, dst: &Path) -> Result<(), String> {
    let src_str = src.to_str().ok_or("源路径无效")?;
    let dst_str = dst.to_str().ok_or("目标路径无效")?;

    #[cfg(target_os = "linux")]
    {
        // 源路径带结尾 "/" 使 rsync 复制目录内容
        let src_trailing = format!("{}/", src_str);
        let status = std::process::Command::new("rsync")
            .args(["-a", &src_trailing, dst_str])
            .status()
            .map_err(|e| format!("无法启动 rsync（是否已安装？）: {}", e))?;
        if !status.success() {
            return Err("rsync 复制失败".to_string());
        }
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("xcopy")
            .args([src_str, dst_str, "/E", "/I", "/Y", "/Q"])
            .status()
            .map_err(|e| format!("无法启动 xcopy: {}", e))?;
        if !status.success() {
            return Err("xcopy 复制失败".to_string());
        }
        return Ok(());
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        copy_dir_recursive(src, dst)
    }
}

/// 创建存档备份（保留最近 KEEP_BACKUPS 份）
pub fn create(version: &str) -> Result<String, String> {
    let config = config::load()?;
    let saves_dir = version_saves_dir(&config, version);

    if !saves_dir.exists() {
        return Err(format!(
            "存档目录不存在: {}（先启动一次游戏生成存档）",
            saves_dir.display()
        ));
    }

    let backup_root = backups_root(&config, version);
    fs::create_dir_all(&backup_root).map_err(|e| e.to_string())?;

    // 时间戳命名；同一秒内重复创建时追加序号，避免把内容拷进已有备份
    let base = chrono_lite_timestamp();
    let mut name = base.clone();
    let mut n = 2;
    while backup_root.join(&name).exists() {
        name = format!("{}-{}", base, n);
        n += 1;
    }
    let backup_path = backup_root.join(&name);

    copy_saves_dir(&saves_dir, &backup_path)?;

    cleanup_old_backups(&backup_root, KEEP_BACKUPS)?;

    println!("存档备份已创建: {:?}", backup_path);
    Ok(name)
}

/// 列出存档备份（新的在前）
pub fn list(version: &str) -> Result<Vec<String>, String> {
    let config = config::load()?;
    let backup_root = backups_root(&config, version);

    if !backup_root.exists() {
        return Ok(Vec::new());
    }

    let mut backups: Vec<String> = fs::read_dir(&backup_root)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    backups.sort_by(|a, b| b.cmp(a));
    Ok(backups)
}

/// 恢复存档备份：先完整复制到临时目录，确认无误后再与当前存档原子交换
pub async fn restore(version: &str, backup_name: &str) -> Result<(), String> {
    let config = config::load()?;
    let backup_path = backups_root(&config, version).join(backup_name);
    if !backup_path.exists() {
        return Err("备份不存在".to_string());
    }

    let saves_dir = version_saves_dir(&config, version);
    let parent = saves_dir.parent().ok_or("无法定位存档父目录")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;

    let safe = sanitize(version);
    let tmp = parent.join(format!(".{}_restore_tmp", safe));
    let old = parent.join(format!(".{}_restore_old", safe));
    for stale in [&tmp, &old] {
        if stale.exists() {
            config::clear_dir_contents(stale)?;
            fs::remove_dir(stale).map_err(|e| e.to_string())?;
        }
    }

    // 阶段 1：备份完整复制到临时目录（此时当前存档未被动过）
    copy_saves_dir(&backup_path, &tmp)?;

    // 阶段 2：交换——旧存档改名暂存，临时目录顶上；失败则回滚
    if saves_dir.symlink_metadata().is_ok() {
        fs::rename(&saves_dir, &old).map_err(|e| format!("暂存当前存档失败: {}", e))?;
    }
    if let Err(e) = fs::rename(&tmp, &saves_dir) {
        if old.symlink_metadata().is_ok() {
            let _ = fs::rename(&old, &saves_dir);
        }
        return Err(format!("恢复失败: {}", e));
    }

    // 阶段 3：清理暂存的旧存档（失败不影响结果，仅留残留目录）
    if old.symlink_metadata().is_ok() {
        let _ = config::clear_dir_contents(&old);
        let _ = fs::remove_dir(&old);
    }

    ensure_saves_symlink(&config, version).await?;

    println!("存档已恢复: {}", backup_name);
    Ok(())
}

/// 删除存档备份
pub fn delete(version: &str, backup_name: &str) -> Result<(), String> {
    let config = config::load()?;
    let backup_path = backups_root(&config, version).join(backup_name);
    if backup_path.exists() {
        fs::remove_dir_all(&backup_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 版本名转文件名安全形式（用于临时目录命名）
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 当前时间的备份名
fn chrono_lite_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    timestamp_from_secs(secs)
}

/// 时间戳备份名（Howard Hinnant civil_from_days，纯 std 跨平台实现）
fn timestamp_from_secs(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// 递归复制目录（跨平台兜底）
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if src.is_dir() {
        fs::create_dir_all(dst).map_err(|e| e.to_string())?;
        let mut entries = fs::read_dir(src).map_err(|e| e.to_string())?;
        while let Some(entry_result) = entries.next() {
            let entry = entry_result.map_err(|e| e.to_string())?;
            let src_path = entry.path();
            let file_name: std::ffi::OsString = entry.file_name();
            let dst_path = dst.join(&file_name);
            copy_dir_recursive(&src_path, &dst_path)?;
        }
    } else {
        fs::copy(src, dst).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_matches_calendar() {
        // 覆盖闰年、世纪闰年、世纪平年、跨年等边界
        let cases: &[(u64, &str)] = &[
            (0, "1970-01-01"),
            (1704067200, "2024-01-01"),
            (1709078400, "2024-02-28"),
            (1709164800, "2024-02-29"),
            (1709251200, "2024-03-01"),
            (1718409600, "2024-06-15"),
            (1735603200, "2024-12-31"),
            (1686787200, "2023-06-15"),
            (1735689600, "2025-01-01"),
            (946684800, "2000-01-01"),
            (951782400, "2000-02-29"),
            (4107542400, "2100-03-01"),
            (1792972800, "2026-10-26"),
        ];
        for &(secs, expected) in cases {
            let got = &timestamp_from_secs(secs)[..10];
            assert_eq!(got, expected, "unix {}", secs);
        }
    }

    #[test]
    fn saves_dir_matrix() {
        let mut config = AppConfig {
            root_dir: "/r".to_string(),
            ..Default::default()
        };
        // 无任何隔离：实例目录 = root/game
        assert_eq!(
            version_saves_dir(&config, "1.20").display().to_string(),
            "/r/game/saves"
        );
        // 仅存档隔离：统一存储在 root/saves/{v}
        config.saves_isolation = true;
        assert_eq!(
            version_saves_dir(&config, "1.20").display().to_string(),
            "/r/saves/1.20"
        );
        // 版本隔离 + 存档隔离：存储仍在 root/saves/{v}
        config.version_isolation = true;
        assert_eq!(
            version_saves_dir(&config, "1.20").display().to_string(),
            "/r/saves/1.20"
        );
        // 仅版本隔离：实例目录下的真实 saves
        config.saves_isolation = false;
        assert_eq!(
            version_saves_dir(&config, "1.20").display().to_string(),
            "/r/versions/1.20/saves"
        );
    }

    #[test]
    fn sanitize_keeps_alnum() {
        assert_eq!(sanitize("1.20.4"), "1_20_4");
        assert_eq!(sanitize("a-b_c"), "a-b_c");
    }
}
