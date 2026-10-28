//! Account-scoped authoritative friend data managed by the launcher.
//!
//! The launcher stores contacts and requests. Bridge sends snapshots to the game
//! and routes actions from Minecraft's native friends UI back through this module.
//! Cross-device discovery is not wired yet.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_ENTRIES_PER_GROUP: usize = 1000;
const MAX_NAME_CHARS: usize = 64;
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;
static FRIENDS_STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendEntry {
    pub profile_id: String,
    pub name: String,
    #[serde(default)]
    pub presence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendSnapshot {
    #[serde(default)]
    pub invite_code: String,
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
            invite_code: String::new(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FriendAction {
    AddRequest { invite_code: String },
    Accept { profile_id: String },
    Decline { profile_id: String },
    Remove { profile_id: String },
    Cancel { profile_id: String },
    Settings { enabled: bool, allow_requests: bool },
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
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    load_for_unlocked(account_id)
}

fn load_for_unlocked(account_id: &str) -> Result<FriendSnapshot, String> {
    let path = store_path(account_id)?;
    if let Some(parent) = path.parent() {
        reject_symlink(parent)?;
    }
    if !path.exists() {
        let mut snapshot = FriendSnapshot::default();
        snapshot.invite_code = create_invite_code();
        save_snapshot_unlocked(account_id, &snapshot)?;
        return Ok(snapshot);
    }
    reject_symlink(&path)?;
    let metadata = fs::metadata(&path).map_err(|e| format!("读取好友缓存失败: {e}"))?;
    if !metadata.is_file() || metadata.len() > MAX_STORE_BYTES {
        return Err("好友缓存文件无效或过大".into());
    }
    let bytes = fs::read(&path).map_err(|e| format!("读取好友缓存失败: {e}"))?;
    let mut snapshot: FriendSnapshot =
        serde_json::from_slice(&bytes).map_err(|e| format!("好友缓存格式无效: {e}"))?;
    // Upgrade snapshots written before launcher-owned invite codes existed.
    if snapshot.invite_code.is_empty() {
        snapshot.invite_code = create_invite_code();
        snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
        save_snapshot_unlocked(account_id, &snapshot)?;
    }
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn save_snapshot_for(account_id: &str, snapshot: &FriendSnapshot) -> Result<(), String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    save_snapshot_unlocked(account_id, snapshot)
}

fn save_snapshot_unlocked(account_id: &str, snapshot: &FriendSnapshot) -> Result<(), String> {
    validate_snapshot(snapshot)?;
    if snapshot.invite_code.is_empty() {
        return Err("好友数据缺少本账号好友码".into());
    }
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

/// Apply one launcher or in-game UI action and persist the resulting snapshot.
pub fn apply_action_for(account_id: &str, action: FriendAction) -> Result<FriendSnapshot, String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    match action {
        FriendAction::AddRequest { invite_code } => {
            let code =
                normalize_invite_code(&invite_code).ok_or_else(|| "好友码格式无效".to_string())?;
            if code.eq_ignore_ascii_case(&snapshot.invite_code) {
                return Err("不能添加自己的好友码".into());
            }
            if snapshot
                .friends
                .iter()
                .chain(&snapshot.incoming_requests)
                .chain(&snapshot.outgoing_requests)
                .any(|entry| entry.name.eq_ignore_ascii_case(&code))
            {
                return Err("该好友码已在列表中".into());
            }
            if snapshot.outgoing_requests.len() >= MAX_ENTRIES_PER_GROUP {
                return Err("待处理好友申请已达到上限".into());
            }
            snapshot.outgoing_requests.push(FriendEntry {
                profile_id: Uuid::new_v4().to_string(),
                name: code,
                presence: None,
            });
        }
        FriendAction::Accept { profile_id } => {
            let entry = take_entry(&mut snapshot.incoming_requests, &profile_id)?;
            if snapshot.friends.len() >= MAX_ENTRIES_PER_GROUP {
                return Err("好友列表已达到上限".into());
            }
            snapshot.friends.push(FriendEntry {
                presence: None,
                ..entry
            });
        }
        FriendAction::Decline { profile_id } => {
            take_entry(&mut snapshot.incoming_requests, &profile_id)?;
        }
        FriendAction::Remove { profile_id } => {
            take_entry(&mut snapshot.friends, &profile_id)?;
        }
        FriendAction::Cancel { profile_id } => {
            take_entry(&mut snapshot.outgoing_requests, &profile_id)?;
        }
        FriendAction::Settings {
            enabled,
            allow_requests,
        } => {
            snapshot.enabled = enabled;
            snapshot.allow_requests = allow_requests;
        }
    }
    snapshot.service_state = "success".into();
    snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
    save_snapshot_unlocked(account_id, &snapshot)?;
    Ok(snapshot)
}

fn take_entry(entries: &mut Vec<FriendEntry>, profile_id: &str) -> Result<FriendEntry, String> {
    let id = Uuid::parse_str(profile_id).map_err(|_| "好友档案 ID 无效".to_string())?;
    let index = entries
        .iter()
        .position(|entry| Uuid::parse_str(&entry.profile_id).ok() == Some(id))
        .ok_or_else(|| "找不到这条好友记录".to_string())?;
    Ok(entries.remove(index))
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn create_invite_code() -> String {
    format!(
        "DL1-{}",
        Uuid::new_v4().simple().to_string().to_ascii_uppercase()
    )
}

fn normalize_invite_code(input: &str) -> Option<String> {
    let code = input.trim().to_ascii_uppercase();
    let payload = code.strip_prefix("DL1-")?;
    if payload.len() != 32 || !payload.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(code)
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
    if !snapshot.invite_code.is_empty() && normalize_invite_code(&snapshot.invite_code).is_none() {
        return Err("本账号好友码无效".into());
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
    reject_symlink(path)?;
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path).map_err(|e| e.to_string())?;
    }
    if !path.is_dir() {
        return Err("好友数据路径不是目录".into());
    }
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
