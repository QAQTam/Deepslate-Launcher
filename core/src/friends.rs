//! Account-scoped authoritative friend data managed by the launcher.
//!
//! The launcher owns the friend graph and signs every relationship change with
//! an Ed25519 identity stored next to the account snapshot. `friendnet` moves
//! those signed messages directly between launchers (or through a mutual
//! friend when a direct route is unavailable). Bridge only mirrors snapshots
//! into Minecraft 26.3's native friends UI.

use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use uuid::Uuid;

const MAX_ENTRIES_PER_GROUP: usize = 1000;
const MAX_OUTBOX: usize = 256;
const MAX_TOMBSTONES: usize = 2048;
const MAX_NAME_CHARS: usize = 64;
const MAX_ENDPOINT_CHARS: usize = 255;
const MAX_STORE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IDENTITY_BYTES: u64 = 16 * 1024;
const IDENTITY_VERSION: u32 = 1;
const OUTBOX_MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

static FRIENDS_STORE_LOCK: Mutex<()> = Mutex::new(());
/// Identity files are created while the caller may already hold the snapshot
/// lock, so they use a separate lock to avoid re-entrancy deadlocks.
static IDENTITY_STORE_LOCK: Mutex<()> = Mutex::new(());
static IDENTITY_CACHE: LazyLock<Mutex<HashMap<String, Arc<FriendIdentity>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub const PRESENCE_ONLINE: &str = "online";
pub const PRESENCE_OFFLINE: &str = "offline";
pub const PRESENCE_STATES: &[&str] = &[
    "online",
    "playing_offline",
    "playing_realms",
    "playing_server",
    "playing_hosted_server",
    "offline",
];

/// Map the 26.3 Authlib `PresenceStatus` enum names sent by Bridge to the
/// launcher's stable presence vocabulary.
pub fn presence_from_authlib(status: &str) -> Option<&'static str> {
    match status.trim().to_ascii_uppercase().as_str() {
        "ONLINE" => Some(PRESENCE_ONLINE),
        "PLAYING_OFFLINE" => Some("playing_offline"),
        "PLAYING_REALMS" => Some("playing_realms"),
        "PLAYING_SERVER" => Some("playing_server"),
        "PLAYING_HOSTED_SERVER" => Some("playing_hosted_server"),
        "OFFLINE" => Some(PRESENCE_OFFLINE),
        _ => None,
    }
}

/// Message kinds accepted from the friend network. Keeping this list closed
/// prevents the store from accumulating arbitrary payloads.
pub const WIRE_KINDS: &[&str] = &[
    "friend_request",
    "friend_accept",
    "friend_decline",
    "friend_remove",
    "friend_cancel",
    "presence",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendEntry {
    pub profile_id: String,
    pub name: String,
    #[serde(default)]
    pub presence: Option<String>,
    /// When the peer is hosting a world via P2P tunnel, this holds the serialized room ticket.
    #[serde(default)]
    pub room_ticket: Option<String>,
    /// Ed25519 identity of the remote account (64 lowercase hex chars).
    #[serde(default)]
    pub key: Option<String>,
    /// Last address the peer advertises or was observed at (`host:port`).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Last authenticated packet from the peer, in Unix milliseconds.
    #[serde(default)]
    pub last_seen: Option<u64>,
    /// Lamport-style revision for the relationship edge. `updated_by` breaks
    /// ties deterministically when both sides act concurrently.
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    /// "direct", "relay", or "local" for entries created before networking.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// This account's Ed25519 public key (hex).
    #[serde(default)]
    pub identity: String,
    #[serde(default)]
    pub network: FriendNetworkState,
    /// Status shared with friends while the launcher is running.
    #[serde(default = "default_presence")]
    pub self_presence: String,
    /// When hosting a world via P2P tunnel, this holds the active room ticket.
    #[serde(default)]
    pub self_room_ticket: Option<String>,
    /// Signed relationship updates waiting for delivery or acknowledgement.
    #[serde(default)]
    pub outbox: Vec<OutboxItem>,
    /// High-water marks that stop delayed packets from resurrecting a removed
    /// relationship.
    #[serde(default)]
    pub tombstones: Vec<Tombstone>,
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
            identity: String::new(),
            network: FriendNetworkState::default(),
            self_presence: default_presence(),
            self_room_ticket: None,
            outbox: Vec::new(),
            tombstones: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FriendNetworkState {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub error: Option<String>,
    /// Number of currently connected friends (runtime only).
    #[serde(default)]
    pub online: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutboxItem {
    pub id: String,
    pub peer_key: String,
    pub kind: String,
    pub body: Value,
    /// Signature over `canonical_wire_bytes(kind, from, signed_at, id, body)`.
    pub signature: String,
    pub signed_at: u64,
    pub created_at: u64,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub next_attempt: u64,
    /// Last friend used as a relay, for diagnostics only.
    #[serde(default)]
    pub via: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tombstone {
    pub key: String,
    /// "removed", "declined", or "canceled".
    pub state: String,
    pub revision: u64,
    pub updated_by: String,
    pub updated_at: u64,
}

/// One authenticated relationship event. `friendnet` verifies the wire
/// signature before handing it to [`apply_remote_event_for`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteEvent {
    Request {
        request_id: String,
        /// Identity the request is addressed to. Relays cannot retarget it.
        to_key: String,
        profile_id: String,
        name: String,
        revision: u64,
        endpoint: Option<String>,
    },
    Accept {
        request_id: String,
        profile_id: String,
        name: String,
        revision: u64,
    },
    Decline {
        request_id: String,
        revision: u64,
    },
    Remove {
        revision: u64,
    },
    Cancel {
        request_id: String,
        revision: u64,
    },
    Presence {
        profile_id: String,
        name: String,
        state: String,
        endpoint: Option<String>,
        room_ticket: Option<String>,
        updated_at: u64,
    },
}

#[derive(Debug, Clone)]
pub struct RemoteOutcome {
    /// Present when the visible snapshot changed and should be pushed to the
    /// GUI and the running Bridge session.
    pub snapshot: Option<FriendSnapshot>,
    /// Whether the event was accepted (used for the acknowledgement status).
    pub accepted: bool,
    pub message: Option<String>,
    /// Ask the transport to close this peer (removal, identity change, ...).
    pub close_peer: bool,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInviteCode {
    pub public_key: String,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IdentityFile {
    version: u32,
    pkcs8: String,
}

/// Ed25519 signer shared by the store and the network runtime.
pub struct FriendIdentity {
    key_pair: Arc<Ed25519KeyPair>,
    public_key: [u8; 32],
    public_hex: String,
}

impl FriendIdentity {
    pub fn public_hex(&self) -> &str {
        &self.public_hex
    }

    pub fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.key_pair.sign(message).as_ref().to_vec()
    }

    pub fn sign_b64(&self, message: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(self.sign(message))
    }
}

fn not_synced() -> String {
    "not_synced".into()
}

fn default_presence() -> String {
    PRESENCE_ONLINE.into()
}

fn canonical_account(account_id: &str) -> Result<String, String> {
    Uuid::parse_str(account_id)
        .map(|id| id.hyphenated().to_string())
        .map_err(|_| "当前档案 UUID 无效，无法读取好友快照".to_string())
}

#[cfg(test)]
static TEST_CONFIG_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
#[cfg(test)]
pub(crate) static TEST_CONFIG_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn set_test_config_dir(path: Option<PathBuf>) {
    if let Ok(mut slot) = TEST_CONFIG_DIR.lock() {
        *slot = path;
    }
}

fn friends_config_dir() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(path) = TEST_CONFIG_DIR
        .lock()
        .map_err(|_| "测试配置目录锁已损坏".to_string())?
        .clone()
    {
        return Ok(path);
    }
    let profile_path = crate::profiles::path()?;
    profile_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "档案配置目录无效".to_string())
}

fn store_path(account_id: &str) -> Result<PathBuf, String> {
    let account = canonical_account(account_id)?;
    Ok(friends_config_dir()?
        .join("friends")
        .join(format!("{account}.json")))
}

fn identity_dir() -> Result<PathBuf, String> {
    Ok(friends_config_dir()?.join("friends").join("keys"))
}

fn identity_path(account_id: &str) -> Result<PathBuf, String> {
    let account = canonical_account(account_id)?;
    Ok(identity_dir()?.join(format!("{account}.json")))
}

/// Load (or create) the account's long-lived Ed25519 identity. The private key
/// never leaves this file and is created with owner-only permissions.
pub fn load_or_create_identity(account_id: &str) -> Result<Arc<FriendIdentity>, String> {
    let account = canonical_account(account_id)?;
    let path = identity_path(&account)?;
    // Scope the cache by the actual identity file so switching the launcher
    // root cannot reuse a key from another configuration directory.
    let cache_key = path.display().to_string();
    let _guard = IDENTITY_STORE_LOCK
        .lock()
        .map_err(|_| "好友身份存储锁已损坏".to_string())?;
    if let Some(identity) = IDENTITY_CACHE
        .lock()
        .map_err(|_| "好友身份缓存锁已损坏".to_string())?
        .get(&cache_key)
        .cloned()
    {
        return Ok(identity);
    }
    let parent = path
        .parent()
        .ok_or_else(|| "好友身份目录无效".to_string())?;
    create_private_dir(parent)?;
    reject_symlink(&path)?;
    let identity = if path.exists() {
        let metadata = fs::metadata(&path).map_err(|e| format!("读取好友身份失败: {e}"))?;
        if !metadata.is_file() || metadata.len() > MAX_IDENTITY_BYTES {
            return Err("好友身份文件无效或过大".into());
        }
        let bytes = fs::read(&path).map_err(|e| format!("读取好友身份失败: {e}"))?;
        let file: IdentityFile =
            serde_json::from_slice(&bytes).map_err(|e| format!("好友身份格式无效: {e}"))?;
        if file.version != IDENTITY_VERSION {
            return Err("好友身份版本不受支持".into());
        }
        let pkcs8 = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(file.pkcs8.as_bytes())
            .map_err(|_| "好友身份密钥编码无效".to_string())?;
        let key_pair =
            Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|_| "好友身份密钥无效".to_string())?;
        FriendIdentity::from_key_pair(key_pair)?
    } else {
        let rng = SystemRandom::new();
        let pkcs8 =
            Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| "无法生成好友身份".to_string())?;
        let content = serde_json::to_vec_pretty(&IdentityFile {
            version: IDENTITY_VERSION,
            pkcs8: base64::engine::general_purpose::STANDARD_NO_PAD.encode(pkcs8.as_ref()),
        })
        .map_err(|e| e.to_string())?;
        let temp = parent.join(format!(".friend-key-{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<(), String> {
            let mut file = crate::secure_fs::create_private_file(&temp)?;
            file.write_all(&content).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            fs::rename(&temp, &path).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
            .map_err(|_| "好友身份密钥无效".to_string())?;
        FriendIdentity::from_key_pair(key_pair)?
    };
    let identity = Arc::new(identity);
    IDENTITY_CACHE
        .lock()
        .map_err(|_| "好友身份缓存锁已损坏".to_string())?
        .insert(cache_key, identity.clone());
    Ok(identity)
}

#[cfg(test)]
impl FriendIdentity {
    /// In-memory identity for transport tests; never persisted.
    pub(crate) fn generate_for_test() -> Self {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        Self::from_key_pair(key_pair).unwrap()
    }
}

impl FriendIdentity {
    fn from_key_pair(key_pair: Ed25519KeyPair) -> Result<Self, String> {
        let public = key_pair.public_key().as_ref();
        if public.len() != 32 {
            return Err("好友身份公钥长度无效".into());
        }
        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(public);
        Ok(Self {
            key_pair: Arc::new(key_pair),
            public_key,
            public_hex: hex::encode(public_key),
        })
    }
}

/// Canonical bytes covered by a wire signature. Length prefixes keep the
/// fields unambiguous even if a JSON body contains arbitrary text.
pub fn canonical_wire_bytes(kind: &str, from: &str, ts: u64, nonce: &str, body: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(body).unwrap_or_default();
    let mut out = Vec::with_capacity(64 + body.len());
    out.extend_from_slice(b"DSF1");
    push_field(&mut out, from.as_bytes());
    push_field(&mut out, kind.as_bytes());
    push_field(&mut out, &ts.to_be_bytes());
    push_field(&mut out, nonce.as_bytes());
    push_field(&mut out, &body);
    out
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u32).to_be_bytes());
    out.extend_from_slice(field);
}

pub fn verify_signature_b64(
    public_hex: &str,
    message: &[u8],
    signature_b64: &str,
) -> Result<(), String> {
    let key = normalize_public_key(public_hex).ok_or_else(|| "好友公钥格式无效".to_string())?;
    let key = hex::decode(key).map_err(|_| "好友公钥格式无效".to_string())?;
    let signature = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(signature_b64.as_bytes())
        .map_err(|_| "好友消息签名编码无效".to_string())?;
    UnparsedPublicKey::new(&ED25519, &key)
        .verify(message, &signature)
        .map_err(|_| "好友消息签名无效".to_string())
}

pub fn normalize_public_key(input: &str) -> Option<String> {
    let key = input.trim().to_ascii_lowercase();
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(key)
}

/// Deterministic placeholder UUID used before a peer's Minecraft profile ID is
/// known. It keeps GUI/Bridge actions stable across attempts.
pub fn uuid_from_public_key(public_hex: &str) -> String {
    let normalized = normalize_public_key(public_hex).unwrap_or_default();
    let digest = ring::digest::digest(&ring::digest::SHA256, normalized.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest.as_ref()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).hyphenated().to_string()
}

pub fn create_invite_code(public_hex: &str, endpoint: Option<&str>) -> String {
    let public = normalize_public_key(public_hex).unwrap_or_default();
    let endpoint = endpoint
        .filter(|endpoint| validate_endpoint(endpoint))
        .map(|endpoint| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(endpoint.as_bytes())
        })
        .unwrap_or_default();
    format!("DL2-{}-{}", public.to_ascii_uppercase(), endpoint)
}

/// Parse a DL2 invite code. Legacy `DL1-` codes are intentionally rejected:
/// they cannot carry a public key or an address and therefore cannot be routed.
pub fn parse_invite_code(input: &str) -> Option<ParsedInviteCode> {
    let code = input.trim();
    let payload = code
        .strip_prefix("DL2-")
        .or_else(|| code.strip_prefix("dl2-"))?;
    let (public, endpoint) = payload.split_once('-')?;
    let public_key = normalize_public_key(public)?;
    if endpoint.is_empty() {
        return Some(ParsedInviteCode {
            public_key,
            endpoint: None,
        });
    }
    if endpoint.len() > 400 {
        return None;
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(endpoint.as_bytes())
        .ok()?;
    let endpoint = String::from_utf8(decoded).ok()?;
    if !validate_endpoint(&endpoint) {
        return None;
    }
    Some(ParsedInviteCode {
        public_key,
        endpoint: Some(endpoint),
    })
}

pub fn normalize_invite_code(input: &str) -> Option<String> {
    parse_invite_code(input)
        .map(|parsed| create_invite_code(&parsed.public_key, parsed.endpoint.as_deref()))
}

pub fn is_legacy_invite_code(input: &str) -> bool {
    input.trim().to_ascii_uppercase().starts_with("DL1-")
}

pub fn validate_endpoint(endpoint: &str) -> bool {
    if endpoint.is_empty()
        || endpoint.len() > MAX_ENDPOINT_CHARS
        || endpoint
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || endpoint.contains(['/', '\\', '@', '?', '#'])
    {
        return false;
    }
    let Some((host, port)) = endpoint.rsplit_once(':') else {
        return false;
    };
    if host.is_empty() || port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Ok(port) = port.parse::<u16>() else {
        return false;
    };
    if port == 0 {
        return false;
    }
    if host.starts_with('[') {
        if !host.ends_with(']') || host.len() < 3 {
            return false;
        }
        let inner = &host[1..host.len() - 1];
        return inner.parse::<std::net::Ipv6Addr>().is_ok();
    }
    if host.contains(':') {
        return false;
    }
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn detect_local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

/// Best-effort address advertised to friends. `friend_advertise` wins when it
/// is set (host or host:port); otherwise the primary local IPv4 plus the
/// configured port is used. Port forwarding is still the user's responsibility.
pub fn configured_endpoint(config: &crate::config::AppConfig) -> String {
    let port = if config.friend_port == 0 {
        crate::config::AppConfig::default().friend_port
    } else {
        config.friend_port
    };
    let advertise = config.friend_advertise.trim();
    if !advertise.is_empty() {
        let with_port = if advertise.contains(':') {
            advertise.to_string()
        } else {
            format!("{advertise}:{port}")
        };
        if validate_endpoint(&with_port) {
            return with_port;
        }
    }
    let host = detect_local_ip()
        .map(|ip| match ip {
            IpAddr::V6(address) => format!("[{address}]"),
            IpAddr::V4(address) => address.to_string(),
        })
        .unwrap_or_else(|| "127.0.0.1".to_string());
    format!("{host}:{port}")
}

fn configured_port(config: &crate::config::AppConfig) -> u16 {
    if config.friend_port == 0 {
        crate::config::AppConfig::default().friend_port
    } else {
        config.friend_port
    }
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
        let identity = load_or_create_identity(account_id)?;
        let config = crate::config::load().unwrap_or_default();
        let endpoint = configured_endpoint(&config);
        snapshot.identity = identity.public_hex().to_string();
        snapshot.network.port = configured_port(&config);
        snapshot.network.endpoint = endpoint.clone();
        snapshot.invite_code = create_invite_code(identity.public_hex(), Some(&endpoint));
        snapshot.updated_at = now_millis();
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
    let changed = migrate_snapshot(account_id, &mut snapshot)?;
    validate_snapshot(&snapshot)?;
    if changed {
        save_snapshot_unlocked(account_id, &snapshot)?;
    }
    Ok(snapshot)
}

/// Fill fields introduced by the decentralized design. Legacy DL1 entries are
/// kept as local-only records instead of being silently discarded.
fn migrate_snapshot(account_id: &str, snapshot: &mut FriendSnapshot) -> Result<bool, String> {
    let mut changed = false;
    let identity = load_or_create_identity(account_id)?;
    if snapshot.identity != identity.public_hex() {
        snapshot.identity = identity.public_hex().to_string();
        changed = true;
    }
    let config = crate::config::load().unwrap_or_default();
    if snapshot.network.port == 0 {
        snapshot.network.port = configured_port(&config);
        changed = true;
    }
    if !validate_endpoint(&snapshot.network.endpoint) {
        snapshot.network.endpoint = configured_endpoint(&config);
        changed = true;
    }
    let invite_ok = parse_invite_code(&snapshot.invite_code)
        .map(|parsed| parsed.public_key == snapshot.identity)
        .unwrap_or(false);
    if !invite_ok {
        snapshot.invite_code =
            create_invite_code(&snapshot.identity, Some(&snapshot.network.endpoint));
        changed = true;
    }
    if !PRESENCE_STATES.contains(&snapshot.self_presence.as_str()) {
        snapshot.self_presence = default_presence();
        changed = true;
    }
    for entry in snapshot
        .friends
        .iter_mut()
        .chain(snapshot.incoming_requests.iter_mut())
        .chain(snapshot.outgoing_requests.iter_mut())
    {
        if entry.source.is_none() {
            entry.source = Some(if entry.key.is_some() {
                "direct".into()
            } else {
                "local".into()
            });
            changed = true;
        }
    }
    let now = now_millis();
    let before = snapshot.outbox.len();
    snapshot
        .outbox
        .retain(|item| now.saturating_sub(item.created_at) <= OUTBOX_MAX_AGE_MS);
    if snapshot.outbox.len() != before {
        changed = true;
    }
    if changed {
        snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
    }
    Ok(changed)
}

pub fn save_snapshot_for(account_id: &str, snapshot: &FriendSnapshot) -> Result<(), String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    save_snapshot_unlocked(account_id, snapshot)?;
    drop(_guard);
    crate::friendnet::notify_store_changed();
    Ok(())
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
        let mut file = crate::secure_fs::create_private_file(&temp)?;
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

/// Public view of the Minecraft profile bound to an account, used for signed
/// presence and friend-request payloads.
pub fn self_profile_for(account_id: &str) -> (String, String) {
    self_profile(account_id)
}

fn self_profile(account_id: &str) -> (String, String) {
    let canonical = canonical_account(account_id).unwrap_or_else(|_| account_id.to_string());
    let name = crate::profiles::load()
        .ok()
        .and_then(|store| {
            store.profiles.into_iter().find(|profile| {
                Uuid::parse_str(&profile.uuid).ok() == Uuid::parse_str(account_id).ok()
            })
        })
        .map(|profile| profile.name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Minecraft 玩家".to_string());
    (canonical, name)
}

fn make_outbox(identity: &FriendIdentity, peer_key: &str, kind: &str, body: Value) -> OutboxItem {
    let id = Uuid::new_v4().to_string();
    let signed_at = now_millis();
    let signature = identity.sign_b64(&canonical_wire_bytes(
        kind,
        identity.public_hex(),
        signed_at,
        &id,
        &body,
    ));
    OutboxItem {
        id,
        peer_key: peer_key.to_string(),
        kind: kind.to_string(),
        body,
        signature,
        signed_at,
        created_at: signed_at,
        attempts: 0,
        next_attempt: 0,
        via: None,
    }
}

fn latest_revision(snapshot: &FriendSnapshot, key: &str) -> u64 {
    snapshot
        .friends
        .iter()
        .chain(snapshot.incoming_requests.iter())
        .chain(snapshot.outgoing_requests.iter())
        .filter(|entry| entry.key.as_deref() == Some(key))
        .map(|entry| entry.revision)
        .chain(
            snapshot
                .tombstones
                .iter()
                .filter(|tombstone| tombstone.key == key)
                .map(|tombstone| tombstone.revision),
        )
        .max()
        .unwrap_or(0)
}

fn next_revision(snapshot: &FriendSnapshot, key: &str) -> u64 {
    // Wall-clock milliseconds make a fresh request newer than old tombstones
    // even if the peers' logical counters diverged while one side was offline.
    // The local high-water mark still advances past any received revision.
    latest_revision(snapshot, key)
        .saturating_add(1)
        .max(now_millis())
}

fn set_tombstone(snapshot: &mut FriendSnapshot, key: &str, state: &str, revision: u64, by: &str) {
    let now = now_millis();
    if let Some(existing) = snapshot
        .tombstones
        .iter_mut()
        .find(|tombstone| tombstone.key == key)
    {
        existing.state = state.to_string();
        existing.revision = existing.revision.max(revision);
        existing.updated_by = by.to_string();
        existing.updated_at = now;
    } else {
        snapshot.tombstones.push(Tombstone {
            key: key.to_string(),
            state: state.to_string(),
            revision,
            updated_by: by.to_string(),
            updated_at: now,
        });
    }
    if snapshot.tombstones.len() > MAX_TOMBSTONES {
        snapshot
            .tombstones
            .sort_by_key(|tombstone| tombstone.revision);
        let excess = snapshot.tombstones.len() - MAX_TOMBSTONES;
        snapshot.tombstones.drain(0..excess);
    }
}

fn take_entry(entries: &mut Vec<FriendEntry>, profile_id: &str) -> Result<FriendEntry, String> {
    let id = Uuid::parse_str(profile_id).map_err(|_| "好友档案 ID 无效".to_string())?;
    let index = entries
        .iter()
        .position(|entry| Uuid::parse_str(&entry.profile_id).ok() == Some(id))
        .ok_or_else(|| "找不到这条好友记录".to_string())?;
    Ok(entries.remove(index))
}

fn take_entry_by_key(entries: &mut Vec<FriendEntry>, key: &str) -> Option<FriendEntry> {
    entries
        .iter()
        .position(|entry| entry.key.as_deref() == Some(key))
        .map(|index| entries.remove(index))
}

/// Match a pending entry by peer and (when present) request id. A mismatched
/// request id means the packet belongs to an older request and must not cancel
/// or accept a newer one.
fn pending_index(entries: &[FriendEntry], key: &str, request_id: &str) -> Option<usize> {
    entries.iter().position(|entry| {
        entry.key.as_deref() == Some(key)
            && (request_id.is_empty()
                || entry.request_id.is_none()
                || entry.request_id.as_deref() == Some(request_id))
    })
}

/// Apply one launcher or in-game UI action and persist the resulting snapshot.
pub fn apply_action_for(account_id: &str, action: FriendAction) -> Result<FriendSnapshot, String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    let identity = load_or_create_identity(account_id)?;
    let (self_uuid, self_name) = self_profile(account_id);
    let mut outbox_changed = false;
    match action {
        FriendAction::AddRequest { invite_code } => {
            let parsed = parse_invite_code(&invite_code).ok_or_else(|| {
                if is_legacy_invite_code(&invite_code) {
                    "这是旧版 DL1 好友码，无法联网；请让对方在「好友」页复制新的 DL2 好友码"
                        .to_string()
                } else {
                    "好友码格式无效（应形如 DL2-…）".to_string()
                }
            })?;
            if parsed.public_key == identity.public_hex() {
                return Err("不能添加自己的好友码".into());
            }
            if snapshot
                .friends
                .iter()
                .chain(snapshot.incoming_requests.iter())
                .chain(snapshot.outgoing_requests.iter())
                .any(|entry| entry.key.as_deref() == Some(parsed.public_key.as_str()))
            {
                return Err("这个好友码已在列表中".into());
            }
            if snapshot.outgoing_requests.len() >= MAX_ENTRIES_PER_GROUP {
                return Err("待处理好友申请已达到上限".into());
            }
            let revision = next_revision(&snapshot, &parsed.public_key);
            let request_id = Uuid::new_v4().to_string();
            let endpoint = if validate_endpoint(&snapshot.network.endpoint) {
                snapshot.network.endpoint.clone()
            } else {
                configured_endpoint(&crate::config::load().unwrap_or_default())
            };
            let body = json!({
                "request_id": request_id,
                "to": parsed.public_key,
                "profile_id": self_uuid,
                "name": self_name,
                "revision": revision,
                "endpoint": endpoint,
                "created_at": now_millis(),
            });
            let item = make_outbox(&identity, &parsed.public_key, "friend_request", body);
            snapshot.outgoing_requests.push(FriendEntry {
                profile_id: uuid_from_public_key(&parsed.public_key),
                name: format!("好友 {}", &parsed.public_key[..8].to_ascii_uppercase()),
                presence: None,
                room_ticket: None,
                key: Some(parsed.public_key.clone()),
                endpoint: parsed.endpoint,
                last_seen: None,
                revision,
                updated_by: Some(identity.public_hex().to_string()),
                request_id: Some(request_id),
                source: Some("direct".into()),
            });
            snapshot.outbox.push(item);
            outbox_changed = true;
        }
        FriendAction::Accept { profile_id } => {
            let entry = take_entry(&mut snapshot.incoming_requests, &profile_id)?;
            if snapshot.friends.len() >= MAX_ENTRIES_PER_GROUP {
                return Err("好友列表已达到上限".into());
            }
            let key = entry.key.clone();
            let revision = key
                .as_deref()
                .map(|key| next_revision(&snapshot, key).max(entry.revision.saturating_add(1)))
                .unwrap_or(entry.revision);
            let mut friend = FriendEntry {
                presence: None,
                last_seen: None,
                revision,
                updated_by: Some(identity.public_hex().to_string()),
                source: Some(if key.is_some() {
                    "direct".into()
                } else {
                    "local".into()
                }),
                ..entry
            };
            if let Some(key) = key.as_deref() {
                let body = json!({
                    "request_id": friend.request_id,
                    "profile_id": self_uuid,
                    "name": self_name,
                    "revision": revision,
                });
                snapshot
                    .outbox
                    .push(make_outbox(&identity, key, "friend_accept", body));
                outbox_changed = true;
                friend.last_seen = Some(now_millis());
            }
            snapshot.friends.push(friend);
        }
        FriendAction::Decline { profile_id } => {
            let entry = take_entry(&mut snapshot.incoming_requests, &profile_id)?;
            if let Some(key) = entry.key.as_deref() {
                let revision = next_revision(&snapshot, key).max(entry.revision.saturating_add(1));
                let body = json!({
                    "request_id": entry.request_id,
                    "revision": revision,
                });
                snapshot
                    .outbox
                    .push(make_outbox(&identity, key, "friend_decline", body));
                set_tombstone(
                    &mut snapshot,
                    key,
                    "declined",
                    revision,
                    identity.public_hex(),
                );
                outbox_changed = true;
            }
        }
        FriendAction::Remove { profile_id } => {
            let entry = take_entry(&mut snapshot.friends, &profile_id)?;
            if let Some(key) = entry.key.as_deref() {
                let revision = next_revision(&snapshot, key).max(entry.revision.saturating_add(1));
                let body = json!({ "revision": revision });
                snapshot
                    .outbox
                    .push(make_outbox(&identity, key, "friend_remove", body));
                set_tombstone(
                    &mut snapshot,
                    key,
                    "removed",
                    revision,
                    identity.public_hex(),
                );
                snapshot
                    .incoming_requests
                    .retain(|other| other.key.as_deref() != Some(key));
                snapshot
                    .outgoing_requests
                    .retain(|other| other.key.as_deref() != Some(key));
                outbox_changed = true;
            }
        }
        FriendAction::Cancel { profile_id } => {
            let entry = take_entry(&mut snapshot.outgoing_requests, &profile_id)?;
            if let Some(key) = entry.key.as_deref() {
                let revision = next_revision(&snapshot, key).max(entry.revision.saturating_add(1));
                let body = json!({
                    "request_id": entry.request_id,
                    "revision": revision,
                });
                snapshot
                    .outbox
                    .push(make_outbox(&identity, key, "friend_cancel", body));
                set_tombstone(
                    &mut snapshot,
                    key,
                    "canceled",
                    revision,
                    identity.public_hex(),
                );
                outbox_changed = true;
            }
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
    drop(_guard);
    crate::friendnet::notify_store_changed();
    if outbox_changed {
        crate::friendnet::notify_outbox();
    }
    Ok(snapshot)
}

/// Apply a signature-verified event received from the friend network.
pub fn apply_remote_event_for(
    account_id: &str,
    from_key: &str,
    event: RemoteEvent,
    source_endpoint: Option<String>,
) -> Result<RemoteOutcome, String> {
    let from_key = normalize_public_key(from_key).ok_or_else(|| "对方身份密钥无效".to_string())?;
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    if from_key == snapshot.identity {
        return Err("不能处理自己发出的好友消息".into());
    }
    let (self_uuid, self_name) = self_profile(account_id);
    let mut changed = false;
    let mut visible = false;
    let mut accepted = true;
    let mut message = None;
    let mut close_peer = false;

    match event {
        RemoteEvent::Request {
            request_id,
            to_key,
            profile_id,
            name,
            revision,
            endpoint,
        } => {
            let profile_id = Uuid::parse_str(&profile_id)
                .map_err(|_| "好友申请中的玩家 ID 无效".to_string())?
                .hyphenated()
                .to_string();
            let name = sanitize_name(&name, "Minecraft 玩家");
            let endpoint = endpoint
                .filter(|endpoint| validate_endpoint(endpoint))
                .or(source_endpoint);
            if normalize_public_key(&to_key).as_deref() != Some(snapshot.identity.as_str()) {
                accepted = false;
                message = Some("好友申请的目标账户不匹配".into());
            } else if !snapshot.enabled || !snapshot.allow_requests {
                accepted = false;
                message = Some("对方未开启好友申请".into());
            } else if snapshot
                .friends
                .iter()
                .any(|entry| entry.key.as_deref() == Some(&from_key))
            {
                // Already friends; acknowledge idempotently.
            } else if snapshot.incoming_requests.iter().any(|entry| {
                entry.key.as_deref() == Some(&from_key)
                    && entry.request_id.as_deref() == Some(request_id.as_str())
            }) {
                // Duplicate delivery.
            } else if let Some(index) = snapshot
                .outgoing_requests
                .iter()
                .position(|entry| entry.key.as_deref() == Some(&from_key))
            {
                // Crossed requests: both sides asked at the same time. Keep the
                // friendship without waiting for the user to accept again.
                let outgoing = snapshot.outgoing_requests.remove(index);
                let revision = next_revision(&snapshot, &from_key)
                    .max(revision)
                    .max(outgoing.revision.saturating_add(1));
                let body = json!({
                    "request_id": request_id,
                    "profile_id": self_uuid,
                    "name": self_name,
                    "revision": revision,
                });
                let identity = load_or_create_identity(account_id)?;
                snapshot
                    .outbox
                    .push(make_outbox(&identity, &from_key, "friend_accept", body));
                snapshot
                    .incoming_requests
                    .retain(|entry| entry.key.as_deref() != Some(&from_key));
                snapshot.friends.push(FriendEntry {
                    profile_id,
                    name,
                    presence: None,
                    room_ticket: None,
                    key: Some(from_key.clone()),
                    endpoint,
                    last_seen: Some(now_millis()),
                    revision,
                    updated_by: Some(from_key.clone()),
                    request_id: Some(request_id),
                    source: Some("direct".into()),
                });
                changed = true;
                visible = true;
            } else {
                // A newer request replaces an older pending one; an equal or
                // older revision is a replay and must not create a duplicate
                // edge (also rejected by snapshot validation).
                let high = latest_revision(&snapshot, &from_key);
                if revision <= high {
                    accepted = false;
                    message = Some("好友申请已过期".into());
                } else {
                    snapshot
                        .incoming_requests
                        .retain(|entry| entry.key.as_deref() != Some(&from_key));
                    snapshot.incoming_requests.push(FriendEntry {
                        profile_id,
                        name,
                        presence: None,
                        room_ticket: None,
                        key: Some(from_key.clone()),
                        endpoint,
                        last_seen: Some(now_millis()),
                        revision,
                        updated_by: Some(from_key.clone()),
                        request_id: Some(request_id),
                        source: Some("direct".into()),
                    });
                    changed = true;
                    visible = true;
                }
            }
        }
        RemoteEvent::Accept {
            request_id,
            profile_id,
            name,
            revision,
        } => {
            if snapshot
                .friends
                .iter()
                .any(|entry| entry.key.as_deref() == Some(&from_key))
            {
                // Duplicate acceptance after a reconnect/retry.
            } else if let Some(index) = snapshot
                .outgoing_requests
                .iter()
                .position(|entry| entry.key.as_deref() == Some(&from_key))
            {
                let matches = request_id.is_empty()
                    || snapshot.outgoing_requests[index].request_id.is_none()
                    || snapshot.outgoing_requests[index].request_id.as_deref()
                        == Some(request_id.as_str());
                if !matches {
                    accepted = false;
                    message = Some("好友确认与当前申请不匹配".into());
                } else {
                    let outgoing = snapshot.outgoing_requests.remove(index);
                    let profile_id = Uuid::parse_str(&profile_id)
                        .map_err(|_| "好友确认中的玩家 ID 无效".to_string())?
                        .hyphenated()
                        .to_string();
                    let name = sanitize_name(&name, "Minecraft 玩家");
                    let revision = revision
                        .max(outgoing.revision)
                        .max(next_revision(&snapshot, &from_key));
                    let request_id = if request_id.is_empty() {
                        outgoing.request_id
                    } else {
                        Some(request_id)
                    };
                    snapshot.friends.push(FriendEntry {
                        profile_id,
                        name,
                        presence: None,
                        room_ticket: None,
                        key: Some(from_key.clone()),
                        endpoint: outgoing.endpoint,
                        last_seen: Some(now_millis()),
                        revision,
                        updated_by: Some(from_key.clone()),
                        request_id,
                        source: Some("direct".into()),
                    });
                    changed = true;
                    visible = true;
                }
            } else {
                accepted = false;
                message = Some("没有对应的待确认好友申请".into());
            }
        }
        RemoteEvent::Decline {
            request_id,
            revision,
        } => {
            if let Some(index) = pending_index(&snapshot.outgoing_requests, &from_key, &request_id)
            {
                let outgoing = snapshot.outgoing_requests.remove(index);
                let revision = revision
                    .max(outgoing.revision.saturating_add(1))
                    .max(next_revision(&snapshot, &from_key));
                set_tombstone(&mut snapshot, &from_key, "declined", revision, &from_key);
                changed = true;
                visible = true;
            }
        }
        RemoteEvent::Remove { revision } => {
            if let Some(index) = snapshot
                .friends
                .iter()
                .position(|entry| entry.key.as_deref() == Some(&from_key))
            {
                let entry = snapshot.friends.remove(index);
                if revision < entry.revision {
                    accepted = false;
                    message = Some("好友关系更新已过期".into());
                } else {
                    let revision = revision
                        .max(entry.revision.saturating_add(1))
                        .max(next_revision(&snapshot, &from_key));
                    snapshot
                        .incoming_requests
                        .retain(|other| other.key.as_deref() != Some(&from_key));
                    snapshot
                        .outgoing_requests
                        .retain(|other| other.key.as_deref() != Some(&from_key));
                    set_tombstone(&mut snapshot, &from_key, "removed", revision, &from_key);
                    changed = true;
                    visible = true;
                    close_peer = true;
                }
            } else {
                // Removal is idempotent; keep a high-water mark so an older
                // accept cannot resurrect the edge.
                let revision = revision.max(next_revision(&snapshot, &from_key));
                set_tombstone(&mut snapshot, &from_key, "removed", revision, &from_key);
                changed = true;
            }
        }
        RemoteEvent::Cancel {
            request_id,
            revision,
        } => {
            if let Some(index) = pending_index(&snapshot.incoming_requests, &from_key, &request_id)
            {
                let incoming = snapshot.incoming_requests.remove(index);
                let revision = revision
                    .max(incoming.revision.saturating_add(1))
                    .max(next_revision(&snapshot, &from_key));
                set_tombstone(&mut snapshot, &from_key, "canceled", revision, &from_key);
                changed = true;
                visible = true;
            }
        }
        RemoteEvent::Presence {
            profile_id,
            name,
            state,
            endpoint,
            room_ticket,
            updated_at,
        } => {
            if !PRESENCE_STATES.contains(&state.as_str()) {
                accepted = false;
                message = Some("在线状态无效".into());
            } else if let Some(entry) = snapshot
                .friends
                .iter_mut()
                .find(|entry| entry.key.as_deref() == Some(&from_key))
            {
                let same_profile =
                    Uuid::parse_str(&entry.profile_id).ok() == Uuid::parse_str(&profile_id).ok();
                if !same_profile {
                    accepted = false;
                    message = Some("在线状态的玩家 ID 与好友记录不一致".into());
                } else {
                    let name = sanitize_name(&name, &entry.name);
                    let endpoint = endpoint.filter(|endpoint| validate_endpoint(endpoint));
                    let presence_changed = entry.presence.as_deref() != Some(state.as_str());
                    let name_changed = entry.name != name;
                    let endpoint_changed = endpoint.is_some() && entry.endpoint != endpoint;
                    let room_ticket_changed = entry.room_ticket != room_ticket;
                    let now = now_millis();
                    // Clamp future timestamps so a broken peer clock cannot keep
                    // presence "online" forever.
                    let seen = now.max(updated_at.min(now.saturating_add(300_000)));
                    let last_seen_changed = entry
                        .last_seen
                        .map(|previous| seen.saturating_sub(previous) >= 10_000)
                        .unwrap_or(true);
                    entry.presence = Some(state);
                    entry.room_ticket = room_ticket;
                    entry.last_seen = Some(seen);
                    if let Some(endpoint) = endpoint {
                        entry.endpoint = Some(endpoint);
                    }
                    entry.name = name;
                    if presence_changed || name_changed || endpoint_changed || room_ticket_changed {
                        visible = true;
                    }
                    if visible || last_seen_changed {
                        changed = true;
                    }
                }
            } else {
                accepted = false;
                message = Some("在线状态来自非好友节点".into());
            }
        }
    }

    if changed {
        if visible {
            snapshot.service_state = "success".into();
            snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
        }
        save_snapshot_unlocked(account_id, &snapshot)?;
    }
    Ok(RemoteOutcome {
        snapshot: if visible { Some(snapshot) } else { None },
        accepted,
        message,
        close_peer,
    })
}

/// The peer an outbox item is addressed to, used to reject spoofed
/// acknowledgements from other friends or relays.
pub fn outbox_peer_for(account_id: &str, item_id: &str) -> Result<Option<String>, String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let snapshot = load_for_unlocked(account_id)?;
    Ok(snapshot
        .outbox
        .iter()
        .find(|item| item.id == item_id)
        .map(|item| item.peer_key.clone()))
}

pub fn ack_outbox_for(
    account_id: &str,
    item_id: &str,
    accepted: bool,
) -> Result<Option<FriendSnapshot>, String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    let Some(index) = snapshot.outbox.iter().position(|item| item.id == item_id) else {
        return Ok(None);
    };
    let item = snapshot.outbox.remove(index);
    if !accepted && item.kind == "friend_request" {
        if let Some(outgoing) = take_entry_by_key(&mut snapshot.outgoing_requests, &item.peer_key) {
            let revision =
                next_revision(&snapshot, &item.peer_key).max(outgoing.revision.saturating_add(1));
            set_tombstone(
                &mut snapshot,
                &item.peer_key,
                "declined",
                revision,
                &item.peer_key,
            );
        }
    }
    snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
    save_snapshot_unlocked(account_id, &snapshot)?;
    Ok(Some(snapshot))
}

/// Record a failed delivery attempt so the hub can back off and try a relay.
pub fn mark_outbox_attempt(
    account_id: &str,
    item_id: &str,
    attempts: u32,
    next_attempt: u64,
    via: Option<&str>,
) -> Result<(), String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    let Some(item) = snapshot.outbox.iter_mut().find(|item| item.id == item_id) else {
        return Ok(());
    };
    item.attempts = attempts.min(1000);
    item.next_attempt = next_attempt;
    if let Some(via) = via {
        item.via = Some(via.to_string());
    }
    save_snapshot_unlocked(account_id, &snapshot)
}

pub fn set_network_status_for(
    account_id: &str,
    running: bool,
    port: u16,
    endpoint: Option<&str>,
    error: Option<&str>,
    online: u32,
) -> Result<FriendSnapshot, String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    let mut changed = snapshot.network.running != running
        || snapshot.network.port != port
        || snapshot.network.error.as_deref() != error
        || snapshot.network.online != online;
    snapshot.network.running = running;
    snapshot.network.port = port;
    snapshot.network.online = online;
    snapshot.network.error = error.map(|error| error.chars().take(512).collect());
    if let Some(endpoint) = endpoint.filter(|endpoint| validate_endpoint(endpoint)) {
        if snapshot.network.endpoint != endpoint {
            snapshot.network.endpoint = endpoint.to_string();
            snapshot.invite_code = create_invite_code(&snapshot.identity, Some(endpoint));
            changed = true;
        }
    }
    if changed {
        snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
        save_snapshot_unlocked(account_id, &snapshot)?;
    }
    Ok(snapshot)
}

pub fn set_self_presence_for(
    account_id: &str,
    presence: &str,
) -> Result<Option<FriendSnapshot>, String> {
    set_self_presence_with_room_for(account_id, presence, None)
}

pub fn set_self_presence_with_room_for(
    account_id: &str,
    presence: &str,
    room_ticket: Option<&str>,
) -> Result<Option<FriendSnapshot>, String> {
    if !PRESENCE_STATES.contains(&presence) {
        return Err("在线状态无效".into());
    }
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    let presence_changed = snapshot.self_presence != presence;
    let room_changed = snapshot.self_room_ticket.as_deref() != room_ticket;
    if !presence_changed && !room_changed {
        return Ok(None);
    }
    snapshot.self_presence = presence.to_string();
    snapshot.self_room_ticket = room_ticket.map(|t| t.to_string());
    snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
    save_snapshot_unlocked(account_id, &snapshot)?;
    Ok(Some(snapshot))
}

/// Mark friends whose heartbeat stopped as offline. Returns a snapshot when a
/// visible state changed.
pub fn expire_presence_for(
    account_id: &str,
    cutoff_ms: u64,
) -> Result<Option<FriendSnapshot>, String> {
    let _guard = FRIENDS_STORE_LOCK
        .lock()
        .map_err(|_| "好友数据存储锁已损坏".to_string())?;
    let mut snapshot = load_for_unlocked(account_id)?;
    let mut changed = false;
    for entry in snapshot.friends.iter_mut() {
        if entry
            .last_seen
            .map(|last_seen| last_seen < cutoff_ms)
            .unwrap_or(false)
            && entry.presence.as_deref() != Some(PRESENCE_OFFLINE)
        {
            entry.presence = Some(PRESENCE_OFFLINE.into());
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    snapshot.updated_at = now_millis().max(snapshot.updated_at.saturating_add(1));
    save_snapshot_unlocked(account_id, &snapshot)?;
    Ok(Some(snapshot))
}

/// Peer keys the transport should keep connected: accepted friends and pending
/// invitations with a usable address.
pub fn connection_targets(snapshot: &FriendSnapshot) -> Vec<(String, String)> {
    let mut targets = Vec::new();
    for entry in snapshot
        .friends
        .iter()
        .chain(snapshot.outgoing_requests.iter())
    {
        let (Some(key), Some(endpoint)) = (entry.key.as_deref(), entry.endpoint.as_deref()) else {
            continue;
        };
        if validate_endpoint(endpoint) {
            targets.push((key.to_string(), endpoint.to_string()));
        }
    }
    targets
}

pub fn is_friend_key(snapshot: &FriendSnapshot, key: &str) -> bool {
    snapshot
        .friends
        .iter()
        .any(|entry| entry.key.as_deref() == Some(key))
}

pub fn sanitize_name(input: &str, fallback: &str) -> String {
    let name = input.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        fallback.chars().take(MAX_NAME_CHARS).collect()
    } else {
        name.to_string()
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
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
    if !snapshot.invite_code.is_empty() {
        let parsed = parse_invite_code(&snapshot.invite_code).ok_or("本账号好友码无效")?;
        if !snapshot.identity.is_empty() && parsed.public_key != snapshot.identity {
            return Err("好友码与本账号身份不匹配".into());
        }
    }
    if !snapshot.identity.is_empty()
        && normalize_public_key(&snapshot.identity).as_deref() != Some(snapshot.identity.as_str())
    {
        return Err("本账号好友身份无效".into());
    }
    if !PRESENCE_STATES.contains(&snapshot.self_presence.as_str()) {
        return Err("本账号在线状态无效".into());
    }
    if let Some(endpoint) = snapshot.network.error.as_deref() {
        if endpoint.chars().count() > 512 || endpoint.chars().any(char::is_control) {
            return Err("好友网络错误信息无效".into());
        }
    }
    let mut keys = HashSet::new();
    for entries in [
        &snapshot.friends,
        &snapshot.incoming_requests,
        &snapshot.outgoing_requests,
    ] {
        if entries.len() > MAX_ENTRIES_PER_GROUP {
            return Err("好友列表超过支持上限".into());
        }
        let mut ids = HashSet::new();
        for friend in entries {
            validate_entry(friend)?;
            if !ids.insert(&friend.profile_id) {
                return Err("好友快照包含重复的玩家 ID".into());
            }
            if let Some(key) = friend.key.as_deref() {
                if !keys.insert(key) {
                    return Err("好友快照包含重复的身份密钥".into());
                }
            }
        }
    }
    if snapshot.outbox.len() > MAX_OUTBOX {
        return Err("待发送好友消息超过上限".into());
    }
    for item in &snapshot.outbox {
        if Uuid::parse_str(&item.id).is_err()
            || !WIRE_KINDS.contains(&item.kind.as_str())
            || normalize_public_key(&item.peer_key).is_none()
            || !item.body.is_object()
            || item.signature.is_empty()
            || item.signature.len() > 256
            || item.attempts > 1000
        {
            return Err("待发送好友消息包含无效记录".into());
        }
    }
    if snapshot.tombstones.len() > MAX_TOMBSTONES {
        return Err("好友关系水位记录超过上限".into());
    }
    for tombstone in &snapshot.tombstones {
        if normalize_public_key(&tombstone.key).is_none()
            || !matches!(
                tombstone.state.as_str(),
                "removed" | "declined" | "canceled"
            )
            || normalize_public_key(&tombstone.updated_by).is_none()
        {
            return Err("好友关系水位记录无效".into());
        }
    }
    Ok(())
}

fn validate_entry(friend: &FriendEntry) -> Result<(), String> {
    if Uuid::parse_str(&friend.profile_id).is_err()
        || friend.name.trim().is_empty()
        || friend.name.chars().count() > MAX_NAME_CHARS
        || friend.name.chars().any(char::is_control)
        || friend
            .presence
            .as_ref()
            .is_some_and(|presence| !PRESENCE_STATES.contains(&presence.as_str()))
        || friend
            .key
            .as_ref()
            .is_some_and(|key| normalize_public_key(key).as_deref() != Some(key.as_str()))
        || friend
            .endpoint
            .as_ref()
            .is_some_and(|endpoint| !validate_endpoint(endpoint))
        || friend
            .request_id
            .as_ref()
            .is_some_and(|request_id| Uuid::parse_str(request_id).is_err())
        || friend
            .source
            .as_ref()
            .is_some_and(|source| !matches!(source.as_str(), "direct" | "relay" | "local"))
        || friend
            .updated_by
            .as_ref()
            .is_some_and(|key| normalize_public_key(key).is_none())
    {
        return Err("好友快照包含无效记录".into());
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    reject_symlink(path)?;
    crate::secure_fs::create_private_dir(path)?;
    if !path.is_dir() {
        return Err("好友数据路径不是目录".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_code_round_trips_public_key_and_endpoint() {
        let key = "AB".repeat(32).to_ascii_lowercase();
        let code = create_invite_code(&key, Some("192.168.1.20:27731"));
        let parsed = parse_invite_code(&code).expect("valid DL2 code");
        assert_eq!(parsed.public_key, key);
        assert_eq!(parsed.endpoint.as_deref(), Some("192.168.1.20:27731"));
        assert_eq!(normalize_invite_code(&code).as_deref(), Some(code.as_str()));
    }

    #[test]
    fn invite_code_rejects_legacy_and_malformed_values() {
        assert!(parse_invite_code("DL1-0123456789ABCDEF0123456789ABCDEF").is_none());
        assert!(is_legacy_invite_code("dl1-0123"));
        assert!(parse_invite_code("DL2-1234-").is_none());
        assert!(parse_invite_code("DL2-").is_none());
        let key = "ab".repeat(32);
        assert!(parse_invite_code(&format!("DL2-{key}-%%%")).is_none());
    }

    #[test]
    fn endpoint_validation_bounds_host_and_port() {
        assert!(validate_endpoint("127.0.0.1:27731"));
        assert!(validate_endpoint("mc.example.com:25565"));
        assert!(validate_endpoint("[::1]:27731"));
        assert!(!validate_endpoint("127.0.0.1"));
        assert!(!validate_endpoint("127.0.0.1:0"));
        assert!(!validate_endpoint("host:99999"));
        assert!(!validate_endpoint("a b:1234"));
        assert!(!validate_endpoint("http://example.com:80"));
    }

    #[test]
    fn deterministic_placeholder_uuid_is_stable() {
        let key = "cd".repeat(32);
        assert_eq!(uuid_from_public_key(&key), uuid_from_public_key(&key));
        assert_ne!(
            uuid_from_public_key(&key),
            uuid_from_public_key(&"ef".repeat(32))
        );
        assert!(Uuid::parse_str(&uuid_from_public_key(&key)).is_ok());
    }

    #[test]
    fn wire_signature_binds_every_field() {
        let key = "11".repeat(32);
        let body = json!({"request_id": "x", "revision": 1});
        let bytes = canonical_wire_bytes("friend_request", &key, 7, "n", &body);
        assert_ne!(
            bytes,
            canonical_wire_bytes("friend_accept", &key, 7, "n", &body)
        );
        assert_ne!(
            bytes,
            canonical_wire_bytes("friend_request", &key, 8, "n", &body)
        );
    }

    struct TestConfigDir(std::path::PathBuf);

    impl Drop for TestConfigDir {
        fn drop(&mut self) {
            super::set_test_config_dir(None);
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn two_accounts_converge_request_accept_presence_and_remove() -> Result<(), String> {
        let _env = super::TEST_CONFIG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = std::env::temp_dir().join(format!("deepslate-friends-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        super::set_test_config_dir(Some(root.clone()));
        let _cleanup = TestConfigDir(root);

        let alice = Uuid::new_v4().to_string();
        let bob = Uuid::new_v4().to_string();
        for account in [&alice, &bob] {
            apply_action_for(
                account,
                FriendAction::Settings {
                    enabled: true,
                    allow_requests: true,
                },
            )?;
        }
        let bob_code = load_for(&bob)?.invite_code;
        let alice_snapshot = apply_action_for(
            &alice,
            FriendAction::AddRequest {
                invite_code: bob_code,
            },
        )?;
        let request_item = alice_snapshot
            .outbox
            .iter()
            .find(|item| item.kind == "friend_request")
            .expect("queued request")
            .clone();
        let request_event =
            crate::friendnet::parse_remote_event(&request_item.kind, &request_item.body)?;
        let wrong_target = RemoteEvent::Request {
            request_id: Uuid::new_v4().to_string(),
            to_key: "00".repeat(32),
            profile_id: alice.clone(),
            name: "Alice".into(),
            revision: 1,
            endpoint: None,
        };
        let rejected = apply_remote_event_for(&bob, &alice_snapshot.identity, wrong_target, None)?;
        assert!(!rejected.accepted);
        let delivered =
            apply_remote_event_for(&bob, &alice_snapshot.identity, request_event.clone(), None)?;
        assert!(delivered.accepted);

        let bob_snapshot = load_for(&bob)?;
        assert_eq!(bob_snapshot.incoming_requests.len(), 1);
        let incoming_id = bob_snapshot.incoming_requests[0].profile_id.clone();
        let bob_snapshot = apply_action_for(
            &bob,
            FriendAction::Accept {
                profile_id: incoming_id,
            },
        )?;
        let accept_item = bob_snapshot
            .outbox
            .iter()
            .find(|item| item.kind == "friend_accept")
            .expect("queued accept")
            .clone();
        let accept_event =
            crate::friendnet::parse_remote_event(&accept_item.kind, &accept_item.body)?;
        let accepted = apply_remote_event_for(&alice, &bob_snapshot.identity, accept_event, None)?;
        assert!(accepted.accepted);

        let alice_snapshot = load_for(&alice)?;
        assert!(alice_snapshot
            .friends
            .iter()
            .any(|friend| friend.key.as_deref() == Some(bob_snapshot.identity.as_str())));

        let presence = RemoteEvent::Presence {
            profile_id: bob.clone(),
            name: "Bob".into(),
            state: "playing_server".into(),
            endpoint: Some("127.0.0.1:27731".into()),
            room_ticket: None,
            updated_at: now_millis(),
        };
        let presence_outcome =
            apply_remote_event_for(&alice, &bob_snapshot.identity, presence, None)?;
        assert!(presence_outcome.accepted);
        let alice_snapshot = load_for(&alice)?;
        assert_eq!(
            alice_snapshot.friends[0].presence.as_deref(),
            Some("playing_server")
        );

        let bob_snapshot = apply_action_for(
            &bob,
            FriendAction::Remove {
                profile_id: alice.clone(),
            },
        )?;
        let remove_item = bob_snapshot
            .outbox
            .iter()
            .find(|item| item.kind == "friend_remove")
            .expect("queued removal")
            .clone();
        let remove_event =
            crate::friendnet::parse_remote_event(&remove_item.kind, &remove_item.body)?;
        let removed = apply_remote_event_for(&alice, &bob_snapshot.identity, remove_event, None)?;
        assert!(removed.accepted);
        assert!(removed.close_peer);
        assert!(load_for(&alice)?.friends.is_empty());

        // A delayed copy of the original request must not resurrect the edge.
        let replay = apply_remote_event_for(&bob, &alice_snapshot.identity, request_event, None)?;
        assert!(!replay.accepted);
        assert!(load_for(&bob)?.friends.is_empty());
        Ok(())
    }

    #[test]
    fn snapshot_validation_rejects_unsupported_presence() {
        let mut snapshot = FriendSnapshot {
            invite_code: create_invite_code(&"ab".repeat(32), Some("127.0.0.1:1234")),
            identity: "ab".repeat(32),
            self_presence: "online".into(),
            ..FriendSnapshot::default()
        };
        snapshot.friends.push(FriendEntry {
            profile_id: "00000000-0000-4000-8000-000000000001".into(),
            name: "Friend".into(),
            presence: Some("dancing".into()),
            room_ticket: None,
            key: Some("cd".repeat(32)),
            endpoint: Some("127.0.0.1:1234".into()),
            last_seen: None,
            revision: 1,
            updated_by: Some("cd".repeat(32)),
            request_id: None,
            source: Some("direct".into()),
        });
        assert!(validate_snapshot(&snapshot).is_err());
        snapshot.friends[0].presence = Some("online".into());
        assert!(validate_snapshot(&snapshot).is_ok());
    }
}
