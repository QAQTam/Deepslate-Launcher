//! P2P virtual network tunneling over QUIC for Minecraft LAN and dedicated servers.
//!
//! Provides transparent TCP-to-QUIC-to-TCP bidirectional forwarding with automatic
//! NAT traversal (STUN hole punching, UPnP port mapping, and relay fallback).
//! Iroh endpoint keys stay ephemeral; friends-only rooms additionally require the
//! guest to sign a per-room challenge with the account's Ed25519 DL2 identity.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use iroh::{
    endpoint::{presets, Connection, RecvStream, SendStream},
    Endpoint, EndpointAddr,
};
use ring::signature;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, RwLock};

use crate::friends::{self, FriendIdentity};

pub const TUNNEL_ALPN: &[u8] = b"deepslate-tunnel/v1";
pub const ROOM_TICKET_PREFIX: &str = "DSR-";
const AUTH_VERSION: u8 = 1;
const AUTH_MAX_BYTES: u64 = 8 * 1024;
const AUTH_TIMEOUT: Duration = Duration::from_secs(6);

/// Access control policy for the hosted room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoomAccessPolicy {
    /// Only confirmed friends in the local store may connect. Guests prove
    /// possession of their DL2 Ed25519 identity key over the tunnel.
    #[default]
    FriendsOnly,
    /// Anyone possessing the room ticket may connect. The ticket is a bearer
    /// capability; only use it when the ticket is shared out of band.
    PublicWithTicket,
}

/// Serialized room invitation ticket shared with peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomTicket {
    /// Room display title or world name.
    pub title: String,
    /// Host game mode (e.g. survival, creative).
    pub mode: String,
    /// Host player display name.
    pub host_name: String,
    /// Maximum allowed players (0 = unlimited).
    pub max_players: u32,
    /// Access control policy.
    pub access: RoomAccessPolicy,
    /// Per-room challenge. Friends-only guests sign
    /// `DSF1-TUNNEL-AUTH|nonce|public_key` so a bearer ticket alone is not
    /// enough to join.
    #[serde(default)]
    pub nonce: String,
    /// Timestamp when this room was published (epoch ms).
    pub created_at: u64,
    /// Target endpoint network addressing information (Iroh EndpointAddr).
    pub addr: EndpointAddr,
}

impl RoomTicket {
    /// Encode room ticket into a compact URL-safe string.
    pub fn encode(&self) -> Result<String, String> {
        let json = serde_json::to_vec(self).map_err(|e| format!("序列化房间票据失败: {e}"))?;
        let b64 = URL_SAFE_NO_PAD.encode(json);
        Ok(format!("{}{}", ROOM_TICKET_PREFIX, b64))
    }

    /// Decode room ticket from a raw string.
    pub fn decode(raw: &str) -> Result<Self, String> {
        let s = raw.trim();
        let payload = s.strip_prefix(ROOM_TICKET_PREFIX).unwrap_or(s);
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|e| format!("房间票据 Base64URL 解码失败: {e}"))?;
        let ticket: RoomTicket =
            serde_json::from_slice(&bytes).map_err(|e| format!("房间票据 JSON 反序列化失败: {e}"))?;
        Ok(ticket)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TunnelAuth {
    v: u8,
    key: String,
    nonce: String,
    sig: String,
}

fn auth_message(nonce: &str, public_key: &str) -> Vec<u8> {
    format!("DSF1-TUNNEL-AUTH|{nonce}|{public_key}").into_bytes()
}

fn verify_friend_signature(public_key: &str, message: &[u8], signature_b64: &str) -> bool {
    let Ok(key_bytes) = hex::decode(public_key) else {
        return false;
    };
    if key_bytes.len() != 32 {
        return false;
    }
    let Ok(signature_bytes) = STANDARD_NO_PAD.decode(signature_b64.as_bytes()) else {
        return false;
    };
    signature::UnparsedPublicKey::new(&signature::ED25519, key_bytes)
        .verify(message, &signature_bytes)
        .is_ok()
}

fn new_room_nonce() -> Result<String, String> {
    let rng = ring::rand::SystemRandom::new();
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&rng, &mut bytes)
        .map_err(|_| "生成房间挑战失败".to_string())?;
    Ok(hex::encode(bytes))
}

async fn reject_auth(send: &mut SendStream, reason: &str) -> String {
    let _ = send.write_all(b"DENY").await;
    let _ = send.finish();
    reason.to_string()
}

async fn accept_auth(send: &mut SendStream) -> Result<(), String> {
    send.write_all(b"OK\0\0").await.map_err(|e| e.to_string())?;
    let _ = send.finish();
    Ok(())
}

/// Host-side challenge/response. The guest must know its DL2 private key and
/// that key must already be a confirmed friend of the host.
async fn authenticate_guest(
    conn: &Connection,
    nonce: &str,
    allowed: Option<&HashSet<String>>,
) -> Result<String, String> {
    let (mut send, mut recv) = tokio::time::timeout(AUTH_TIMEOUT, conn.accept_bi())
        .await
        .map_err(|_| "等待隧道认证超时".to_string())?
        .map_err(|e| format!("读取隧道认证流失败: {e}"))?;

    let mut buffer = Vec::new();
    let read = tokio::time::timeout(AUTH_TIMEOUT, async {
        let mut limited = (&mut recv).take(AUTH_MAX_BYTES);
        limited.read_to_end(&mut buffer).await
    })
    .await
    .map_err(|_| "读取隧道认证请求超时".to_string())?;

    if let Err(error) = read {
        return Err(reject_auth(
            &mut send,
            &format!("读取隧道认证请求失败: {error}"),
        )
        .await);
    }
    let request: TunnelAuth = match serde_json::from_slice(&buffer) {
        Ok(request) => request,
        Err(_) => return Err(reject_auth(&mut send, "隧道认证请求格式无效").await),
    };
    if request.v != AUTH_VERSION {
        return Err(reject_auth(&mut send, "隧道认证版本不匹配").await);
    }
    if request.nonce != nonce {
        return Err(reject_auth(&mut send, "隧道认证挑战不匹配").await);
    }
    let Some(key) = friends::normalize_public_key(&request.key) else {
        return Err(reject_auth(&mut send, "隧道认证公钥无效").await);
    };
    if !allowed.is_some_and(|keys| keys.contains(&key)) {
        return Err(reject_auth(&mut send, "隧道认证不是房主的好友").await);
    }
    let message = auth_message(nonce, &key);
    if !verify_friend_signature(&key, &message, &request.sig) {
        return Err(reject_auth(&mut send, "隧道认证签名无效").await);
    }
    accept_auth(&mut send).await?;
    Ok(key)
}

/// Guest-side challenge response. Must run before the first data stream when
/// the room is friends-only.
async fn authenticate_client(
    conn: &Connection,
    nonce: &str,
    identity: &FriendIdentity,
) -> Result<(), String> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("打开隧道认证流失败: {e}"))?;
    let key = identity.public_hex().to_string();
    let request = TunnelAuth {
        v: AUTH_VERSION,
        key: key.clone(),
        nonce: nonce.to_string(),
        sig: identity.sign_b64(&auth_message(nonce, &key)),
    };
    let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    send.write_all(&bytes)
        .await
        .map_err(|e| format!("发送隧道认证请求失败: {e}"))?;
    let _ = send.finish();

    let mut reply = [0u8; 4];
    tokio::time::timeout(AUTH_TIMEOUT, recv.read_exact(&mut reply))
        .await
        .map_err(|_| "等待隧道认证结果超时".to_string())?
        .map_err(|e| format!("读取隧道认证结果失败: {e}"))?;
    if &reply != b"OK\0\0" {
        return Err("好友房间拒绝了隧道认证".into());
    }
    Ok(())
}

/// Information about a connected guest peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedPeerInfo {
    pub peer_id: String,
    pub remote_addr: String,
    pub is_direct: bool,
    pub rtt_ms: Option<u64>,
    pub connected_at: u64,
    pub streams_active: u32,
}

/// Host-side server session exposing a local Minecraft port over P2P.
pub struct HostSession {
    ticket: RoomTicket,
    ticket_str: String,
    endpoint: Endpoint,
    local_target_port: u16,
    access_policy: RoomAccessPolicy,
    peers: Arc<RwLock<HashMap<String, ConnectedPeerInfo>>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    is_running: Arc<AtomicBool>,
}

impl HostSession {
    /// Start hosting a local Minecraft port (LAN or Dedicated server).
    pub async fn start(
        title: String,
        mode: String,
        host_name: String,
        max_players: u32,
        local_target_port: u16,
        access_policy: RoomAccessPolicy,
        allowed_keys: Option<HashSet<String>>,
    ) -> Result<Self, String> {
        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![TUNNEL_ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| format!("绑定 P2P Host 端点失败: {e}"))?;

        // Best-effort wait for Home Relay connection to guarantee NAT traversal
        let _ = tokio::time::timeout(Duration::from_secs(10), endpoint.online()).await;

        let mut addr = endpoint.addr();
        // In local environments, populate loopback interface to bypass hairpins
        for sock in endpoint.bound_sockets() {
            if sock.is_ipv4() {
                addr = addr.with_ip_addr(SocketAddr::new(
                    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                    sock.port(),
                ));
            }
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let ticket = RoomTicket {
            title,
            mode,
            host_name,
            max_players,
            access: access_policy,
            nonce: new_room_nonce()?,
            created_at: now_ms,
            addr,
        };
        let ticket_str = ticket.encode()?;

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let is_running = Arc::new(AtomicBool::new(true));
        let peers = Arc::new(RwLock::new(HashMap::new()));

        let ep_clone = endpoint.clone();
        let is_running_clone = is_running.clone();
        let peers_clone = peers.clone();
        let allowed_keys_arc = allowed_keys.map(Arc::new);
        let ticket_nonce = ticket.nonce.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    incoming = ep_clone.accept() => {
                        let Some(incoming) = incoming else { break };
                        if !is_running_clone.load(Ordering::SeqCst) {
                            break;
                        }

                        let peers_inner = peers_clone.clone();
                        let allowed_keys_inner = allowed_keys_arc.clone();
                        let nonce_inner = ticket_nonce.clone();

                        tokio::spawn(async move {
                            match incoming.await {
                                Ok(conn) => {
                                    // Friends-only rooms require the guest to
                                    // prove possession of its DL2 identity key.
                                    // The Iroh endpoint key is ephemeral and is
                                    // NOT the friend identity, so comparing
                                    // conn.remote_id() to the friend list would
                                    // reject every legitimate friend.
                                    let peer_id = if access_policy == RoomAccessPolicy::FriendsOnly {
                                        match authenticate_guest(
                                            &conn,
                                            &nonce_inner,
                                            allowed_keys_inner.as_deref(),
                                        )
                                        .await
                                        {
                                            Ok(key) => key,
                                            Err(_) => {
                                                conn.close(1u8.into(), b"auth_failed");
                                                return;
                                            }
                                        }
                                    } else {
                                        conn.remote_id().to_string()
                                    };

                                    let is_direct = conn.paths().iter().any(|p| p.is_selected() && p.is_ip());
                                    let rtt = conn.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt().as_millis() as u64);
                                    let remote_addr = conn.paths().iter().find(|p| p.is_selected()).map(|p| p.remote_addr().to_string()).unwrap_or_default();

                                    let now = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();

                                    {
                                        let mut guard = peers_inner.write().await;
                                        if max_players > 0 && guard.len() >= max_players as usize {
                                            drop(guard);
                                            conn.close(2u8.into(), b"room_full");
                                            return;
                                        }
                                        guard.insert(peer_id.clone(), ConnectedPeerInfo {
                                            peer_id: peer_id.clone(),
                                            remote_addr,
                                            is_direct,
                                            rtt_ms: rtt,
                                            connected_at: now,
                                            streams_active: 0,
                                        });
                                    }

                                    handle_incoming_host_conn(conn, local_target_port, peer_id.clone(), peers_inner.clone()).await;

                                    let mut guard = peers_inner.write().await;
                                    guard.remove(&peer_id);
                                }
                                Err(_) => {}
                            }
                        });
                    }
                }
            }
        });

        Ok(Self {
            ticket,
            ticket_str,
            endpoint,
            local_target_port,
            access_policy,
            peers,
            shutdown_tx: Some(shutdown_tx),
            is_running,
        })
    }

    /// Room ticket representation.
    pub fn ticket(&self) -> &RoomTicket {
        &self.ticket
    }

    /// Serialized room invitation code (`DSR-...`).
    pub fn ticket_str(&self) -> &str {
        &self.ticket_str
    }

    /// Local target port being forwarded.
    pub fn local_target_port(&self) -> u16 {
        self.local_target_port
    }

    /// Access policy configured for this session.
    pub fn access_policy(&self) -> RoomAccessPolicy {
        self.access_policy
    }

    /// List currently connected peers.
    pub async fn connected_peers(&self) -> Vec<ConnectedPeerInfo> {
        let guard = self.peers.read().await;
        guard.values().cloned().collect()
    }

    /// Stop hosting session and close all active tunnels.
    pub async fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.endpoint.close().await;
    }
}

async fn handle_incoming_host_conn(
    conn: Connection,
    local_target_port: u16,
    peer_id: String,
    peers: Arc<RwLock<HashMap<String, ConnectedPeerInfo>>>,
) {
    loop {
        match conn.accept_bi().await {
            Ok((send, recv)) => {
                {
                    let mut guard = peers.write().await;
                    if let Some(info) = guard.get_mut(&peer_id) {
                        info.streams_active += 1;
                    }
                }

                let peers_inner = peers.clone();
                let peer_id_inner = peer_id.clone();
                tokio::spawn(async move {
                    let target_addr: SocketAddr = format!("127.0.0.1:{}", local_target_port)
                        .parse()
                        .unwrap();
                    if let Ok(tcp) = TcpStream::connect(target_addr).await {
                        let _ = bridge_stream(tcp, send, recv).await;
                    }

                    let mut guard = peers_inner.write().await;
                    if let Some(info) = guard.get_mut(&peer_id_inner) {
                        if info.streams_active > 0 {
                            info.streams_active -= 1;
                        }
                    }
                });
            }
            Err(_) => break,
        }
    }
}

/// Client-side proxy session binding a local loopback port for the Minecraft client.
pub struct ClientSession {
    local_bind_port: u16,
    endpoint: Endpoint,
    remote_ticket: RoomTicket,
    shutdown_tx: Option<oneshot::Sender<()>>,
    is_running: Arc<AtomicBool>,
}

impl ClientSession {
    /// Connect to a remote room and bind a local loopback port for Minecraft.
    ///
    /// If `desired_bind_port` is 0, an ephemeral free port is chosen.
    pub async fn start(
        remote_ticket: RoomTicket,
        desired_bind_port: u16,
        identity: Option<Arc<FriendIdentity>>,
    ) -> Result<Self, String> {
        if remote_ticket.access == RoomAccessPolicy::FriendsOnly && identity.is_none() {
            return Err("好友房间需要当前账户的好友身份".into());
        }
        let endpoint = Endpoint::builder(presets::N0)
            .alpns(vec![TUNNEL_ALPN.to_vec()])
            .bind()
            .await
            .map_err(|e| format!("绑定 P2P Client 端点失败: {e}"))?;

        let _ = tokio::time::timeout(Duration::from_secs(8), endpoint.online()).await;

        let bind_addr: SocketAddr = format!("127.0.0.1:{}", desired_bind_port)
            .parse()
            .map_err(|e| format!("解析回环绑定地址失败: {e}"))?;
        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|e| format!("客机代理无法监听本地端口 {}: {e}", bind_addr))?;

        let local_bind_port = listener
            .local_addr()
            .map_err(|e| format!("获取监听端口失败: {e}"))?
            .port();
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let is_running = Arc::new(AtomicBool::new(true));

        let endpoint_arc = Arc::new(endpoint.clone());
        let target_addr = remote_ticket.addr.clone();
        let access = remote_ticket.access;
        let nonce = remote_ticket.nonce.clone();
        let is_running_clone = is_running.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    incoming = listener.accept() => {
                        let Ok((tcp, _)) = incoming else { break };
                        if !is_running_clone.load(Ordering::SeqCst) {
                            break;
                        }

                        let ep = endpoint_arc.clone();
                        let target = target_addr.clone();
                        let identity = identity.clone();
                        let nonce = nonce.clone();

                        tokio::spawn(async move {
                            let Ok(conn) = ep.connect(target, TUNNEL_ALPN).await else {
                                return;
                            };
                            if access == RoomAccessPolicy::FriendsOnly {
                                let Some(identity) = identity else { return };
                                if authenticate_client(&conn, &nonce, &identity).await.is_err() {
                                    return;
                                }
                            }
                            if let Ok((send, recv)) = conn.open_bi().await {
                                let _ = bridge_stream(tcp, send, recv).await;
                            }
                        });
                    }
                }
            }
        });

        Ok(Self {
            local_bind_port,
            endpoint,
            remote_ticket,
            shutdown_tx: Some(shutdown_tx),
            is_running,
        })
    }

    /// Local port listening on 127.0.0.1 (can be passed to Minecraft JoinServer).
    pub fn local_port(&self) -> u16 {
        self.local_bind_port
    }

    /// Local server address formatted as `127.0.0.1:<port>`.
    pub fn local_address(&self) -> String {
        format!("127.0.0.1:{}", self.local_bind_port)
    }

    /// Remote room metadata.
    pub fn remote_ticket(&self) -> &RoomTicket {
        &self.remote_ticket
    }

    /// Stop client proxy.
    pub async fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.endpoint.close().await;
    }
}

/// Bidirectional data bridge between local TCP and remote QUIC stream.
async fn bridge_stream(tcp: TcpStream, mut quic_send: SendStream, mut quic_recv: RecvStream) -> Result<(), String> {
    let (mut tcp_read, mut tcp_write) = tcp.into_split();

    let quic_to_tcp = async {
        let _ = tokio::io::copy(&mut quic_recv, &mut tcp_write).await;
        let _ = tcp_write.shutdown().await;
    };

    let tcp_to_quic = async {
        let _ = tokio::io::copy(&mut tcp_read, &mut quic_send).await;
        let _ = quic_send.finish();
    };

    tokio::join!(quic_to_tcp, tcp_to_quic);
    Ok(())
}

// ── Global Runtime Session Manager ──────────────────────────────────────────

static ACTIVE_HOST: tokio::sync::Mutex<Option<HostSession>> = tokio::sync::Mutex::const_new(None);
static ACTIVE_CLIENT: tokio::sync::Mutex<Option<ClientSession>> = tokio::sync::Mutex::const_new(None);

/// Publish a local Minecraft singleplayer LAN or dedicated server as a P2P room.
///
/// Automatically creates an Iroh tunnel host, updates self-presence to
/// `playing_hosted_server`, and broadcasts the room ticket to all online friends.
pub async fn publish_world_room(
    account_id: &str,
    world_name: &str,
    mode: &str,
    host_name: &str,
    max_players: u32,
    local_port: u16,
    access_policy: RoomAccessPolicy,
) -> Result<String, String> {
    let mut guard = ACTIVE_HOST.lock().await;
    if let Some(mut existing) = guard.take() {
        existing.stop().await;
    }

    let allowed_keys: Option<HashSet<String>> = match access_policy {
        RoomAccessPolicy::FriendsOnly => {
            let snapshot = crate::friends::load_for(account_id).unwrap_or_default();
            let keys: HashSet<String> = snapshot
                .friends
                .into_iter()
                .filter_map(|f| f.key)
                .filter_map(|key| friends::normalize_public_key(&key))
                .collect();
            Some(keys)
        }
        RoomAccessPolicy::PublicWithTicket => None,
    };

    let session = HostSession::start(
        world_name.to_string(),
        mode.to_string(),
        host_name.to_string(),
        max_players,
        local_port,
        access_policy,
        allowed_keys,
    )
    .await?;

    let ticket_str = session.ticket_str().to_string();

    // Update self-presence in the friend store
    let _ = crate::friends::set_self_presence_with_room_for(
        account_id,
        "playing_hosted_server",
        Some(&ticket_str),
    );

    // Broadcast new presence over the friend network immediately
    crate::friendnet::notify_presence_changed();

    *guard = Some(session);
    Ok(ticket_str)
}

/// Unpublish currently hosted world room, notifying friends and closing active tunnels.
pub async fn unpublish_world_room(account_id: &str) -> Result<(), String> {
    let mut guard = ACTIVE_HOST.lock().await;
    if let Some(mut session) = guard.take() {
        session.stop().await;
    }

    // Reset self-presence
    let _ = crate::friends::set_self_presence_with_room_for(
        account_id,
        crate::friends::PRESENCE_ONLINE,
        None,
    );

    crate::friendnet::notify_presence_changed();
    Ok(())
}

/// Join a friend's hosted world room by its ticket string (`DSR-...`).
///
/// Starts a local loopback forward proxy. Returns the local `127.0.0.1:<port>` address
/// that can be supplied directly to Minecraft's `JoinServer`.
pub async fn join_remote_room(
    ticket_str: &str,
    preferred_port: u16,
    account_id: &str,
) -> Result<String, String> {
    let mut guard = ACTIVE_CLIENT.lock().await;
    if let Some(mut existing) = guard.take() {
        existing.stop().await;
    }

    let ticket = RoomTicket::decode(ticket_str)?;
    let identity = if ticket.access == RoomAccessPolicy::FriendsOnly {
        Some(crate::friends::load_or_create_identity(account_id)?)
    } else {
        None
    };
    let session = ClientSession::start(ticket, preferred_port, identity).await?;
    let local_addr = session.local_address();

    *guard = Some(session);
    Ok(local_addr)
}

/// Disconnect and stop the client proxy session.
pub async fn leave_remote_room() -> Result<(), String> {
    let mut guard = ACTIVE_CLIENT.lock().await;
    if let Some(mut session) = guard.take() {
        session.stop().await;
    }
    Ok(())
}

/// Inspect the active hosting session status and connected guest count.
pub async fn active_host_info() -> Option<(RoomTicket, Vec<ConnectedPeerInfo>)> {
    let guard = ACTIVE_HOST.lock().await;
    if let Some(host) = guard.as_ref() {
        let ticket = host.ticket().clone();
        let peers = host.connected_peers().await;
        Some((ticket, peers))
    } else {
        None
    }
}

/// Best-effort synchronous check used when reconciling stale presence after a
/// launcher restart. A held lock is treated as active to avoid clearing a room
/// that is currently being published or stopped.
pub fn host_is_active() -> bool {
    match ACTIVE_HOST.try_lock() {
        Ok(guard) => guard.is_some(),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn start_echo_server() -> Result<u16, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Bind echo failed: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Get port failed: {e}"))?
            .port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 512];
                    if let Ok(n) = socket.read(&mut buf).await {
                        if n > 0 {
                            let _ = socket.write_all(b"PONG: ").await;
                            let _ = socket.write_all(&buf[..n]).await;
                            let _ = socket.shutdown().await;
                        }
                    }
                });
            }
        });
        Ok(port)
    }

    #[tokio::test]
    async fn test_tunnel_end_to_end() -> Result<(), String> {
        let echo_port = start_echo_server().await?;

        // Start Host session forwarding to echo_port
        let mut host = HostSession::start(
            "Test World".into(),
            "survival".into(),
            "HostPlayer".into(),
            8,
            echo_port,
            RoomAccessPolicy::PublicWithTicket,
            None,
        )
        .await?;

        let ticket_str = host.ticket_str().to_string();
        let decoded_ticket = RoomTicket::decode(&ticket_str)?;

        // Start Client session
        let mut client = ClientSession::start(decoded_ticket, 0, None).await?;
        let proxy_port = client.local_port();

        // Connect local TCP client to proxy_port
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", proxy_port))
            .await
            .map_err(|e| format!("Connect proxy failed: {e}"))?;
        stream
            .write_all(b"Minecraft Handshake\n")
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        stream
            .shutdown()
            .await
            .map_err(|e| format!("Shutdown failed: {e}"))?;

        let mut buf = vec![0u8; 128];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("Read failed: {e}"))?;
        let resp = String::from_utf8_lossy(&buf[..n]);

        assert!(resp.starts_with("PONG: Minecraft Handshake"));

        // Clean up
        client.stop().await;
        host.stop().await;
        Ok(())
    }

    #[tokio::test]
    async fn test_managed_room_lifecycle() -> Result<(), String> {
        let echo_port = start_echo_server().await?;
        let test_account = "00000000-0000-4000-8000-000000000001";

        let ticket_str = publish_world_room(
            test_account,
            "Survival Season 1",
            "survival",
            "Alice",
            4,
            echo_port,
            RoomAccessPolicy::PublicWithTicket,
        )
        .await?;

        assert!(ticket_str.starts_with(ROOM_TICKET_PREFIX));

        let local_addr = join_remote_room(&ticket_str, 0, test_account).await?;
        assert!(local_addr.starts_with("127.0.0.1:"));

        let mut stream = TcpStream::connect(&local_addr)
            .await
            .map_err(|e| format!("Connect to managed proxy failed: {e}"))?;
        stream
            .write_all(b"Hello Managed Tunnel\n")
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        stream.shutdown().await.map_err(|e| format!("Shutdown failed: {e}"))?;

        let mut buf = vec![0u8; 128];
        let n = stream.read(&mut buf).await.map_err(|e| format!("Read failed: {e}"))?;
        let resp = String::from_utf8_lossy(&buf[..n]);
        assert!(resp.contains("Hello Managed Tunnel"));

        // Clean up
        leave_remote_room().await?;
        unpublish_world_room(test_account).await?;
        Ok(())
    }

    #[test]
    fn friend_signature_binds_nonce_and_public_key() {
        let identity = FriendIdentity::generate_for_test();
        let message = auth_message("room-nonce", identity.public_hex());
        let signature = identity.sign_b64(&message);
        assert!(verify_friend_signature(
            identity.public_hex(),
            &message,
            &signature
        ));
        assert!(!verify_friend_signature(
            identity.public_hex(),
            &auth_message("other-nonce", identity.public_hex()),
            &signature
        ));
        let other = FriendIdentity::generate_for_test();
        assert!(!verify_friend_signature(
            other.public_hex(),
            &message,
            &signature
        ));
    }

    #[tokio::test]
    async fn friends_only_room_accepts_a_known_identity() -> Result<(), String> {
        let echo_port = start_echo_server().await?;
        let guest = Arc::new(FriendIdentity::generate_for_test());
        let allowed = HashSet::from([guest.public_hex().to_string()]);
        let mut host = HostSession::start(
            "Friends Only".into(),
            "survival".into(),
            "HostPlayer".into(),
            8,
            echo_port,
            RoomAccessPolicy::FriendsOnly,
            Some(allowed),
        )
        .await?;
        assert!(!host.ticket().nonce.is_empty());

        let mut client =
            ClientSession::start(host.ticket().clone(), 0, Some(guest)).await?;
        let mut stream = TcpStream::connect(client.local_address())
            .await
            .map_err(|e| format!("Connect friends-only proxy failed: {e}"))?;
        stream
            .write_all(b"Friends Only Handshake\n")
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        stream
            .shutdown()
            .await
            .map_err(|e| format!("Shutdown failed: {e}"))?;
        let mut buf = vec![0u8; 128];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("Read failed: {e}"))?;
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("PONG: Friends Only Handshake"));

        // A different identity must not get through the same ticket.
        let stranger = Arc::new(FriendIdentity::generate_for_test());
        let mut wrong = ClientSession::start(host.ticket().clone(), 0, Some(stranger)).await?;
        let mut rejected = TcpStream::connect(wrong.local_address())
            .await
            .map_err(|e| format!("Connect rejected proxy failed: {e}"))?;
        rejected
            .write_all(b"not allowed\n")
            .await
            .map_err(|e| format!("Write failed: {e}"))?;
        let _ = rejected.shutdown().await;
        let mut rejected_buf = [0u8; 16];
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            rejected.read(&mut rejected_buf),
        )
        .await;
        assert!(
            matches!(result, Ok(Ok(0)) | Ok(Err(_))),
            "unexpected response from rejected guest: {result:?}"
        );
        wrong.stop().await;

        client.stop().await;
        host.stop().await;
        Ok(())
    }
}
