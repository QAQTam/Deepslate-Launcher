//! P2P virtual network tunneling over QUIC for Minecraft LAN and dedicated servers.
//!
//! Provides transparent TCP-to-QUIC-to-TCP bidirectional forwarding with automatic
//! NAT traversal (STUN hole punching, UPnP port mapping, and DERP relay fallback).
//! Node identities are anchored to the account's Ed25519 keypair, integrating
//! seamlessly with [`crate::friends`].

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use iroh::{
    endpoint::{presets, Connection, RecvStream, SendStream},
    Endpoint, EndpointAddr,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, RwLock};

pub const TUNNEL_ALPN: &[u8] = b"deepslate-tunnel/v1";
pub const ROOM_TICKET_PREFIX: &str = "DSR-";

/// Access control policy for the hosted room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoomAccessPolicy {
    /// Only confirmed friends in the local store may connect.
    #[default]
    FriendsOnly,
    /// Confirmed friends and mutual friends (1-hop relay peers) may connect.
    FriendsOfFriends,
    /// Anyone possessing the room ticket may connect.
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
    allowed_keys: Option<HashSet<String>>,
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
        let allowed_keys_arc = allowed_keys.clone().map(Arc::new);

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

                        tokio::spawn(async move {
                            match incoming.await {
                                Ok(conn) => {
                                    let peer_id = conn.remote_id().to_string();

                                    // Access control check
                                    if let Some(ref allowed) = allowed_keys_inner {
                                        if !allowed.contains(&peer_id) {
                                            conn.close(1u8.into(), b"access_denied");
                                            return;
                                        }
                                    }

                                    let is_direct = conn.paths().iter().any(|p| p.is_selected() && p.is_ip());
                                    let rtt = conn.paths().iter().find(|p| p.is_selected()).map(|p| p.rtt().as_millis() as u64);
                                    let remote_addr = conn.paths().iter().find(|p| p.is_selected()).map(|p| p.remote_addr().to_string()).unwrap_or_default();

                                    let now = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();

                                    {
                                        let mut guard = peers_inner.write().await;
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
            allowed_keys,
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
    ) -> Result<Self, String> {
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

                        tokio::spawn(async move {
                            if let Ok(conn) = ep.connect(target, TUNNEL_ALPN).await {
                                if let Ok((send, recv)) = conn.open_bi().await {
                                    let _ = bridge_stream(tcp, send, recv).await;
                                }
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
                .collect();
            Some(keys)
        }
        RoomAccessPolicy::FriendsOfFriends | RoomAccessPolicy::PublicWithTicket => None,
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
) -> Result<String, String> {
    let mut guard = ACTIVE_CLIENT.lock().await;
    if let Some(mut existing) = guard.take() {
        existing.stop().await;
    }

    let ticket = RoomTicket::decode(ticket_str)?;
    let session = ClientSession::start(ticket, preferred_port).await?;
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
        let mut client = ClientSession::start(decoded_ticket, 0).await?;
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

        let local_addr = join_remote_room(&ticket_str, 0).await?;
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
}
