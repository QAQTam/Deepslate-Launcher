use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::auth::{AuthMethod, OnlineProfile};

#[derive(Serialize, Deserialize, Clone, PartialEq)]
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

impl std::fmt::Debug for UserProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserProfile")
            .field("name", &self.name)
            .field("uuid", &self.uuid)
            .field("auth_method", &self.auth_method)
            .field("credentials", &"[已隐藏]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
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

    pub fn needs_refresh(&self, now: i64) -> bool {
        self.is_online()
            && (self.access_token.as_deref().map_or(true, str::is_empty)
                || self
                    .expires_at
                    .map_or(true, |expiry| expiry <= now.saturating_add(60)))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProfileStore {
    pub profiles: Vec<UserProfile>,
    pub last_used: Option<String>,
}

impl ProfileStore {
    pub fn current_profile(&self, profile: &UserProfile) -> Result<UserProfile, String> {
        self.profiles
            .iter()
            .find(|p| p.uuid == profile.uuid)
            .cloned()
            .ok_or_else(|| "档案已被删除，请重新选择账号".to_string())
    }
}

/// 档案文件路径（v2+: root_dir/config/，旧版降级到 ~/.config/deepslate-launcher/，兼容旧版 obsidian-launcher/）
pub fn path() -> Result<PathBuf, String> {
    if crate::config::exists() {
        let config = crate::config::load()?;
        if config.version >= 2 {
            return Ok(PathBuf::from(&config.root_dir)
                .join("config")
                .join("profiles.json"));
        }
    } else {
        let default_config = crate::config::load()?;
        let default_path = PathBuf::from(&default_config.root_dir)
            .join("config")
            .join("profiles.json");
        if default_path.exists() {
            return Ok(default_path);
        }
    }
    dirs::config_dir()
        .map(|p| {
            let primary = p.join("deepslate-launcher").join("profiles.json");
            let legacy = p.join("obsidian-launcher").join("profiles.json");
            if !primary.exists() && legacy.exists() {
                legacy
            } else {
                primary
            }
        })
        .ok_or_else(|| "无法获取配置目录".to_string())
}

pub fn load() -> Result<ProfileStore, String> {
    let path = path()?;
    if path.exists() {
        let content = read_profile_file(&path)?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    } else {
        Ok(ProfileStore {
            profiles: Vec::new(),
            last_used: None,
        })
    }
}

fn read_profile_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    secure_profile_parent(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let mut file = options.open(path).map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("档案路径不是普通文件".to_string());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| e.to_string())?;
        return Ok(content);
    }
    #[cfg(not(unix))]
    {
        if fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("档案路径不能是符号链接".to_string());
        }
        let mut file = options.open(path).map_err(|e| e.to_string())?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| e.to_string())?;
        Ok(content)
    }
}

fn secure_profile_parent(path: &std::path::Path) -> Result<(), String> {
    let parent = path.parent().ok_or("档案目录无效")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent)
            .map_err(|e| format!("打开档案目录失败: {e}"))?;
        directory
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        if fs::symlink_metadata(parent)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("档案目录不能是符号链接".to_string());
        }
    }
    Ok(())
}

pub fn save(store: &ProfileStore) -> Result<(), String> {
    use std::io::Write;
    let path = path()?;
    if let Some(parent) = path.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent).map_err(|e| e.to_string())?;
    }
    secure_profile_parent(&path)?;
    let content = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    let temp = path.with_file_name(format!(".profiles-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|e| e.to_string())?;
        file.write_all(content.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temp, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// 基于用户名生成离线 UUID（deterministic）
pub fn uuid_from_name(name: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(b"OfflinePlayer:");
    hasher.update(name.as_bytes());
    let mut bytes: [u8; 16] = hasher.finalize().into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn online_profile_refreshes_before_expiry() {
        let mut profile = UserProfile {
            name: "x".into(),
            uuid: "id".into(),
            auth_method: AuthMethod::Microsoft,
            access_token: Some("token".into()),
            refresh_token: Some("refresh".into()),
            expires_at: Some(120),
        };
        assert!(profile.needs_refresh(100));
        profile.expires_at = Some(1000);
        assert!(!profile.needs_refresh(100));
        profile.access_token = None;
        assert!(profile.needs_refresh(100));
        profile.auth_method = AuthMethod::Offline;
        assert!(!profile.needs_refresh(100));
    }

    #[test]
    fn new_offline_identity_matches_java_without_changing_stored_profiles() {
        assert_eq!(
            uuid_from_name("Steve"),
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
    }

    #[test]
    fn stored_profile_replaces_stale_in_memory_credentials() {
        let old = UserProfile {
            name: "x".into(),
            uuid: "id".into(),
            auth_method: AuthMethod::Microsoft,
            access_token: Some("old".into()),
            refresh_token: Some("old-refresh".into()),
            expires_at: Some(1),
        };
        let mut fresh = old.clone();
        fresh.access_token = Some("fresh".into());
        let store = ProfileStore {
            profiles: vec![fresh.clone()],
            last_used: None,
        };
        assert_eq!(store.current_profile(&old).unwrap(), fresh);
    }

    #[test]
    fn deleted_online_profile_cannot_fall_back_to_cached_credentials() {
        let old = UserProfile {
            name: "x".into(),
            uuid: "id".into(),
            auth_method: AuthMethod::Microsoft,
            access_token: Some("old".into()),
            refresh_token: Some("refresh".into()),
            expires_at: Some(1),
        };
        let store = ProfileStore {
            profiles: vec![],
            last_used: None,
        };
        assert!(store.current_profile(&old).is_err());
    }

    #[test]
    fn profile_debug_does_not_leak_credentials() {
        let profile = UserProfile {
            name: "x".into(),
            uuid: "id".into(),
            auth_method: AuthMethod::Microsoft,
            access_token: Some("access-secret".into()),
            refresh_token: Some("refresh-secret".into()),
            expires_at: Some(1),
        };
        let debug = format!("{profile:?}");
        assert!(!debug.contains("access-secret"));
        assert!(!debug.contains("refresh-secret"));
    }

    #[test]
    fn credential_storage_rejects_symlink_target() {
        let root = std::env::temp_dir().join(format!("profile-link-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target");
        fs::write(&target, "safe").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, root.join("profiles.json")).unwrap();
            assert!(read_profile_file(&root.join("profiles.json")).is_err());
            assert_eq!(fs::read_to_string(target).unwrap(), "safe");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
