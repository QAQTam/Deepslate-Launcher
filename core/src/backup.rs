use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{self, AppConfig};

/// 建立或确认版本的 saves 符号链接（实验级隔离）
/// 将版本目录下的 saves/ 链接到 root_dir/saves/{version}/
pub async fn ensure_saves_symlink(config: &AppConfig, version: &str) -> Result<(), String> {
    if !config.saves_isolation {
        return Ok(());
    }

    let root = PathBuf::from(&config.root_dir);
    let saves_target = root.join("saves").join(version);
    let version_root = config::get_version_root(config, version);
    let saves_link = version_root.join("saves");

    // 创建存档目标目录
    fs::create_dir_all(&saves_target).map_err(|e| e.to_string())?;

    // 如果已经是正确的符号链接，无需操作
    if saves_link.symlink_metadata().is_ok() {
        if let Ok(meta) = saves_link.symlink_metadata() {
            if meta.file_type().is_symlink() {
                // 检查链接目标是否正确
                if let Ok(target) = fs::read_link(&saves_link) {
                    if target == saves_target {
                        return Ok(()); // 已正确链接
                    }
                }
            }
            // 不是正确链接，删除
            if saves_link.is_dir() {
                fs::remove_dir_all(&saves_link).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(&saves_link).map_err(|e| e.to_string())?;
            }
        }
    }

    // 创建符号链接
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&saves_target, &saves_link)
        .map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(&saves_target, &saves_link)
        .map_err(|e| e.to_string())?;

    println!("已建立 saves 隔离链接: {} -> {:?}", saves_link.display(), saves_target.display());
    Ok(())
}

/// 准备版本存档隔离（建立符号链接），在启动/下载前调用
pub async fn prepare(version: &str) -> Result<(), String> {
    let config = config::load()?;
    ensure_saves_symlink(&config, version).await
}

/// 存档备份目录（root_dir/saves/{version}_backups/）
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

/// 创建存档备份（保留最近 5 份）
pub fn create(version: &str) -> Result<String, String> {
    let config = config::load()?;
    let saves_dir = PathBuf::from(&config.root_dir).join("saves").join(version);

    if !saves_dir.exists() {
        return Err("存档目录不存在，无法备份".to_string());
    }

    let backup_root = backups_root(&config, version);
    fs::create_dir_all(&backup_root).map_err(|e| e.to_string())?;

    // 时间戳命名
    let now = chrono_lite_timestamp();
    let backup_path = backup_root.join(&now);

    // 跨平台复制
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("rsync")
            .args(["-a", "--delete", saves_dir.to_str().unwrap(), backup_path.to_str().unwrap()])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("rsync 备份失败".to_string());
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("xcopy")
            .args([saves_dir.to_str().unwrap(), backup_path.to_str().unwrap(), "/E", "/I", "/Y", "/Q"])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("xcopy 备份失败".to_string());
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        copy_dir_recursive(&saves_dir, &backup_path)?;
    }

    // 清理旧备份，保留最近 5 个
    cleanup_old_backups(&backup_root, 5)?;

    println!("存档备份已创建: {:?}", backup_path);
    Ok(now)
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

/// 恢复存档备份
pub async fn restore(version: &str, backup_name: &str) -> Result<(), String> {
    let config = config::load()?;
    let backup_path = backups_root(&config, version).join(backup_name);
    let saves_dir = PathBuf::from(&config.root_dir).join("saves").join(version);

    if !backup_path.exists() {
        return Err("备份不存在".to_string());
    }

    // 先清空当前存档
    config::clear_dir_contents(&saves_dir)?;
    fs::create_dir_all(&saves_dir).map_err(|e| e.to_string())?;

    // 复制备份内容到存档目录
    #[cfg(target_os = "linux")]
    {
        // 源路径带结尾 "/" 使 rsync 复制目录内容而非目录本身
        let src = format!("{}/", backup_path.to_str().unwrap());
        let status = std::process::Command::new("rsync")
            .args(["-a", &src, saves_dir.to_str().unwrap()])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("rsync 恢复失败".to_string());
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("xcopy")
            .args([backup_path.to_str().unwrap(), saves_dir.to_str().unwrap(), "/E", "/I", "/Y", "/Q"])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("xcopy 恢复失败".to_string());
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        copy_dir_recursive(&backup_path, &saves_dir)?;
    }

    // 重建符号链接（如果启用了 saves_isolation）
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

/// 简单的日期时间戳（纯 std::time 实现，跨平台）
fn chrono_lite_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 手动计算日期时间（平年，非负秒）
    let mut secs = now;
    let (year, month, day, hour, minute, second) = {
        const SECS_PER_MINUTE: u64 = 60;
        const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
        const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;
        const DAYS_PER_400Y: u64 = 365 * 400 + 97;
        const DAYS_PER_100Y: u64 = 365 * 100 + 24;
        const DAYS_PER_4Y: u64 = 365 * 4 + 1;

        // 1970-01-01 是周四，索引 0 = 周四
        const DAYS_FROM_1970: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        const DAYS_FROM_1970_LEAP: [i32; 12] = [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];

        let days = (secs / SECS_PER_DAY) as i64;
        secs %= SECS_PER_DAY;
        let hour = (secs / SECS_PER_HOUR) as u32;
        secs %= SECS_PER_HOUR;
        let minute = (secs / SECS_PER_MINUTE) as u32;
        let second = (secs % SECS_PER_MINUTE) as u32;

        // 计算年份（相对 1970）
        let mut year = 1970;
        let mut days_left = days;

        // 400 年周期
        let cyc400 = days_left / DAYS_PER_400Y as i64;
        days_left -= cyc400 * DAYS_PER_400Y as i64;
        year += (cyc400 * 400) as i32;

        // 100 年周期
        let cyc100 = days_left / DAYS_PER_100Y as i64;
        if cyc100 == 4 {
            // 最后一个 100 年周期（400 年周期的最后一天）
            days_left -= 3 * DAYS_PER_100Y as i64;
            year += 300;
        } else {
            days_left -= cyc100 * DAYS_PER_100Y as i64;
            year += (cyc100 * 100) as i32;
        }

        // 4 年周期
        let cyc4 = days_left / DAYS_PER_4Y as i64;
        if cyc4 == 25 {
            // 最后一个 4 年周期（100 年周期的最后一天）
            days_left -= 24 * DAYS_PER_4Y as i64;
            year += 96;
        } else {
            days_left -= cyc4 * DAYS_PER_4Y as i64;
            year += (cyc4 * 4) as i32;
        }

        // 年周期（1-4 年）
        let mut yday = days_left as i32;
        let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        if is_leap {
            yday += 1;
        }

        // 确定月份
        let (month_idx, mday) = if is_leap {
            let idx = DAYS_FROM_1970_LEAP.iter().position(|&d| yday < d).unwrap_or(12);
            (idx, yday - DAYS_FROM_1970_LEAP[idx - 1])
        } else {
            let idx = DAYS_FROM_1970.iter().position(|&d| yday < d).unwrap_or(12);
            (idx, yday - DAYS_FROM_1970[idx - 1])
        };

        (year, month_idx as u32, (mday + 1) as u32, hour, minute, second)
    };

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        year, month, day, hour, minute, second
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
