//! Cached snapshots of the Minecraft 26.3 FriendsService state.
//!
//! The game remains the authority: Bridge reads PlayerSocialManager and sends
//! snapshots to the launcher. The launcher cache is account-scoped and is not
//! used to create or mutate Minecraft/Xbox friendships.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_ENTRIES_PER_GROUP: usize = 1000;
const MAX_NAME_CHARS: usize = 64;
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendEntry {
    pub profile_id: String,
    pub name: String,
    #[serde(default)]
    pub presence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendSnapshot {
    #[serde(default = "not_synced")]
    pub service_state: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allow_requests: bool,
    #[serde(default)]
    pub friends: Vec<FriendEntry>,
    #[serde(default)]
    pub incoming_requests: Vec<FriendEntry>,
    #[serde(default)]
    pub outgoing_requests: Vec<FriendEntry>,
    #[serde(default)]
    pub updated_at: u64,
}

impl Default for FriendSnapshot {
    fn default() -> Self {
        Self {
            service_state: not_synced(),
            enabled: false,
            allow_requests: false,
            friends: Vec::new(),
            incoming_requests: Vec::new(),
            outgoing_requests: Vec::new(),
            updated_at: 0,
        }
    }
}

fn not_synced() -> String {
    "not_synced".into()
}

fn store_path(account_id: &str) -> Result<PathBuf, String> {
    let account = Uuid::parse_str(account_id)
        .map_err(|_| "当前档案 UUID 无效，无法读取好友快照".to_string())?
        .hyphenated()
        .to_string();
    let profile_path = crate::profiles::path()?;
    let config_dir = profile_path
        .parent()
        .ok_or_else(|| "档案配置目录无效".to_string())?;
    Ok(config_dir.join("friends").join(format!("{account}.json")))
}

pub fn load_for(account_id: &str) -> Result<FriendSnapshot, String> {
    let path = store_path(account_id)?;
    if let Some(parent) = path.parent() {
        reject_symlink(parent)?;
    }
    if !path.exists() {
        return Ok(FriendSnapshot::default());
    }
    reject_symlink(&path)?;
    let metadata = fs::metadata(&path).map_err(|e| format!("读取好友缓存失败: {e}"))?;
    if !metadata.is_file() || metadata.len() > MAX_STORE_BYTES {
        return Err("好友缓存文件无效或过大".into());
    }
    let bytes = fs::read(&path).map_err(|e| format!("读取好友缓存失败: {e}"))?;
    let snapshot: FriendSnapshot =
        serde_json::from_slice(&bytes).map_err(|e| format!("好友缓存格式无效: {e}"))?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn save_snapshot_for(account_id: &str, snapshot: &FriendSnapshot) -> Result<(), String> {
    validate_snapshot(snapshot)?;
    let path = store_path(account_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "好友配置目录无效".to_string())?;
    create_private_dir(parent)?;
    reject_symlink(parent)?;
    reject_symlink(&path)?;
    let content = serde_json::to_vec_pretty(snapshot).map_err(|e| e.to_string())?;
    let temp = parent.join(format!(".friends-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|e| e.to_string())?;
        file.write_all(&content).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temp, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn validate_snapshot(snapshot: &FriendSnapshot) -> Result<(), String> {
    const STATES: &[&str] = &[
        "not_synced",
        "loading",
        "upgrade_needed",
        "connection_issue",
        "user_may_lack_active_profile",
        "unauthorized",
        "temporary_unavailable",
        "generic_error",
        "success",
    ];
    if !STATES.contains(&snapshot.service_state.as_str()) {
        return Err("好友服务状态无效".into());
    }
    for entries in [
        &snapshot.friends,
        &snapshot.incoming_requests,
        &snapshot.outgoing_requests,
    ] {
        if entries.len() > MAX_ENTRIES_PER_GROUP {
            return Err("好友列表超过支持上限".into());
        }
        let mut ids = std::collections::HashSet::new();
        for friend in entries {
            if Uuid::parse_str(&friend.profile_id).is_err()
                || friend.name.trim().is_empty()
                || friend.name.chars().count() > MAX_NAME_CHARS
                || friend.name.chars().any(char::is_control)
                || !ids.insert(&friend.profile_id)
                || friend.presence.as_ref().is_some_and(|presence| {
                    !matches!(
                        presence.as_str(),
                        "online"
                            | "playing_offline"
                            | "playing_realms"
                            | "playing_server"
                            | "playing_hosted_server"
                            | "offline"
                    )
                })
            {
                return Err("好友快照包含无效记录".into());
            }
        }
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("好友配置路径不能是符号链接".into())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
