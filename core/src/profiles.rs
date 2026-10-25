use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::auth::{AuthMethod, OnlineProfile};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct UserProfile {
    pub name: String,
    pub uuid: String,
    #[serde(default)]
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl std::fmt::Display for UserProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl UserProfile {
    pub fn is_online(&self) -> bool {
        matches!(self.auth_method, AuthMethod::Microsoft)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProfileStore {
    pub profiles: Vec<UserProfile>,
    pub last_used: Option<String>,
}

/// 档案文件路径（v2+: root_dir/config/，旧版降级到 ~/.config/obsidian-launcher/）
pub fn path() -> Result<PathBuf, String> {
    let config_path = crate::config::config_path()?.join("config.json");
    if config_path.exists() {
        if let Ok(config) = crate::config::load() {
            if config.version >= 2 {
                return Ok(PathBuf::from(&config.root_dir).join("config").join("profiles.json"));
            }
        }
    }
    dirs::config_dir()
        .map(|p| p.join("obsidian-launcher").join("profiles.json"))
        .ok_or_else(|| "无法获取配置目录".to_string())
}

pub fn load() -> Result<ProfileStore, String> {
    let path = path()?;
    if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    } else {
        Ok(ProfileStore {
            profiles: Vec::new(),
            last_used: None,
        })
    }
}

pub fn save(store: &ProfileStore) -> Result<(), String> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// 基于用户名生成离线 UUID（deterministic）
pub fn uuid_from_name(name: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(name.as_bytes());
    let result = hasher.finalize();
    let hash_str = hex::encode(result);
    format!(
        "{}-{}-{}-{}-{}",
        &hash_str[0..8],
        &hash_str[8..12],
        &hash_str[12..16],
        &hash_str[16..20],
        &hash_str[20..32]
    )
}

/// 生成随机 UUID
pub fn random_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// 创建新离线档案（同名不允许重复）
pub fn create(name: String) -> Result<UserProfile, String> {
    if name.trim().is_empty() {
        return Err("用户名不能为空".to_string());
    }
    let profile = UserProfile {
        uuid: uuid_from_name(&name),
        auth_method: AuthMethod::Offline,
        access_token: None,
        refresh_token: None,
        expires_at: None,
        name,
    };

    let mut store = load()?;
    if store.profiles.iter().any(|p| p.name == profile.name) {
        return Err("档案已存在".to_string());
    }
    store.profiles.push(profile.clone());
    store.last_used = Some(profile.name.clone());
    save(&store)?;

    Ok(profile)
}

/// 写入/更新 Microsoft 在线档案（同一账户按 uuid 去重）
pub fn upsert_online(online: OnlineProfile) -> Result<UserProfile, String> {
    let profile = UserProfile {
        name: online.username,
        uuid: online.uuid,
        auth_method: AuthMethod::Microsoft,
        access_token: Some(online.access_token),
        refresh_token: Some(online.refresh_token),
        expires_at: Some(online.expires_at),
    };

    let mut store = load()?;
    if let Some(existing) = store.profiles.iter_mut().find(|p| p.uuid == profile.uuid) {
        existing.name = profile.name.clone();
        existing.auth_method = profile.auth_method.clone();
        existing.access_token = profile.access_token.clone();
        existing.refresh_token = profile.refresh_token.clone();
        existing.expires_at = profile.expires_at;
    } else {
        store.profiles.push(profile.clone());
    }
    store.last_used = Some(profile.name.clone());
    save(&store)?;

    Ok(profile)
}

/// 删除档案
pub fn delete(name: &str) -> Result<(), String> {
    let mut store = load()?;
    store.profiles.retain(|p| p.name != name);
    if store.last_used.as_ref() == Some(&name.to_string()) {
        store.last_used = None;
    }
    save(&store)
}

/// 重置 UUID（为指定档案生成新的随机 UUID）
pub fn reset_uuid(name: &str) -> Result<UserProfile, String> {
    let mut store = load()?;
    let profile = store
        .profiles
        .iter_mut()
        .find(|p| p.name == name)
        .ok_or("档案不存在")?;

    profile.uuid = random_uuid();
    let result = profile.clone();
    save(&store)?;

    Ok(result)
}
