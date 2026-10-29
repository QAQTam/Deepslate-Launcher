//! Decentralized friend transport managed by the launcher core.
//!
//! Each launcher listens on a TCP port. Peers authenticate with their Ed25519
//! identity, perform an ephemeral X25519 key agreement, and then exchange
//! ChaCha20-Poly1305 protected JSON frames. Relationship changes are signed at
//! rest in the friend store and only reach Minecraft through
//! [`crate::bridge`], so the game never owns or rewrites the friend graph.
//!
//! When two peers cannot reach each other directly, a signed request can be
//! forwarded by a mutual friend. Relays only see hop-by-hop metadata: the inner
//! payload stays signed by its original author, and relays refuse multi-hop
//! forwarding.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::events::CoreEvent;
use crate::friends::{self, FriendIdentity, FriendSnapshot, RemoteEvent, RemoteOutcome};

const WIRE_VERSION: u8 = 1;
const HANDSHAKE_MAX: usize = 8 * 1024;
const FRAME_MAX: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(6);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(75);
const PRESENCE_INTERVAL: Duration = Duration::from_secs(30);
const PRESENCE_RELAY_INTERVAL: Duration = Duration::from_secs(120);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(12);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(5);
const OFFLINE_AFTER_MS: u64 = 90_000;
const MAX_CONNECTIONS: usize = 64;
const OUTBOX_WIRE_BUDGET: usize = FRAME_MAX - 1024;

static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);
static SERVICE_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static EVENT_SINK: std::sync::OnceLock<mpsc::Sender<CoreEvent>> = std::sync::OnceLock::new();

/// Register the GUI's core-event channel. Must be called before the service is
/// started; later snapshots are dropped rather than blocking the transport.
pub fn install_event_sink(sender: mpsc::Sender<CoreEvent>) {
    let _ = EVENT_SINK.set(sender);
}

fn emit_snapshot(account_id: &str, snapshot: FriendSnapshot) {
    if let Some(sink) = EVENT_SINK.get() {
        let _ = sink.try_send(CoreEvent::FriendsSnapshot {
            profile_id: account_id.to_string(),
            snapshot,
        });
    }
}

struct Runtime {
    account_id: String,
    port: u16,
    endpoint: String,
    commands: mpsc::Sender<ServiceCommand>,
    task: tokio::task::JoinHandle<()>,
}

enum ServiceCommand {
    Wake,
    PresenceChanged,
    Stop(oneshot::Sender<()>),
}

/// Ask the running hub to re-read the store (GUI/Bridge actions).
pub fn notify_store_changed() {
    send_command(ServiceCommand::Wake);
}

/// Wake the outbox after a relationship change was persisted.
pub fn notify_outbox() {
    send_command(ServiceCommand::Wake);
}

fn send_command(command: ServiceCommand) {
    let Ok(runtime) = RUNTIME.lock() else {
        return;
    };
    if let Some(runtime) = runtime.as_ref() {
        let _ = runtime.commands.try_send(command);
    }
}

/// Start, restart, or stop the friend network according to the persisted
/// settings. Binding errors are stored in the snapshot and returned so the GUI
/// can surface them without crashing the app.
pub async fn sync_service(account_id: &str, config: &AppConfig) -> Result<(), String> {
    let _service = SERVICE_LOCK.lock().await;
    let snapshot = friends::load_for(account_id)?;
    let account = Uuid::parse_str(account_id)
        .map(|id| id.hyphenated().to_string())
        .unwrap_or_else(|_| account_id.to_string());
    let port = if config.friend_port == 0 {
        AppConfig::default().friend_port
    } else {
        config.friend_port
    };
    let endpoint = friends::configured_endpoint(config);

    if !snapshot.enabled {
        stop_service_inner().await;
        let snapshot =
            friends::set_network_status_for(account_id, false, port, Some(&endpoint), None, 0)?;
        emit_snapshot(account_id, snapshot);
        return Ok(());
    }

    {
        let guard = match RUNTIME.lock() {
            Ok(guard) => guard,
            Err(_) => return Err("好友网络运行时锁已损坏".into()),
        };
        if let Some(runtime) = guard.as_ref() {
            if runtime.account_id == account
                && runtime.port == port
                && runtime.endpoint == endpoint
                && !runtime.task.is_finished()
            {
                let _ = runtime.commands.try_send(ServiceCommand::Wake);
                drop(guard);
                let snapshot = friends::set_network_status_for(
                    account_id,
                    true,
                    port,
                    Some(&endpoint),
                    None,
                    snapshot.network.online,
                )?;
                emit_snapshot(account_id, snapshot);
                return Ok(());
            }
        }
    }

    stop_service_inner().await;
    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(listener) => listener,
        Err(error) => {
            let message = format!("好友网络端口 {port} 无法监听: {error}");
            let snapshot = friends::set_network_status_for(
                account_id,
                false,
                port,
                Some(&endpoint),
                Some(&message),
                0,
            )?;
            emit_snapshot(account_id, snapshot);
            return Err(message);
        }
    };
    // Clear a stale in-game presence from a previous crash before the hub
    // starts advertising it.
    if matches!(
        snapshot.self_presence.as_str(),
        "playing_offline" | "playing_realms" | "playing_server" | "playing_hosted_server"
    ) {
        let _ = friends::set_self_presence_for(account_id, friends::PRESENCE_ONLINE);
    }
    let identity = friends::load_or_create_identity(account_id)?;
    let (events_tx, events_rx) = mpsc::channel(256);
    let (commands_tx, commands_rx) = mpsc::channel(64);
    let hub = Hub {
        account_id: account.clone(),
        identity,
        port,
        endpoint: endpoint.clone(),
        events: events_tx,
        receiver: events_rx,
        commands: commands_rx,
        connections: HashMap::new(),
        last_connect: HashMap::new(),
        last_presence: HashMap::new(),
        sent: HashMap::new(),
        last_relay: HashMap::new(),
        last_snapshot: Instant::now(),
    };
    // Persist the new endpoint/invite code before the hub can send presence.
    let snapshot =
        friends::set_network_status_for(account_id, true, port, Some(&endpoint), None, 0)?;
    emit_snapshot(account_id, snapshot);
    let task = tokio::spawn(hub.run(listener));
    if let Ok(mut runtime) = RUNTIME.lock() {
        *runtime = Some(Runtime {
            account_id: account,
            port,
            endpoint,
            commands: commands_tx,
            task,
        });
    }
    Ok(())
}

pub async fn stop_service() {
    let _service = SERVICE_LOCK.lock().await;
    stop_service_inner().await;
}

async fn stop_service_inner() {
    let runtime = RUNTIME.lock().ok().and_then(|mut guard| guard.take());
    let Some(runtime) = runtime else {
        return;
    };
    let (reply, wait) = oneshot::channel();
    let _ = runtime.commands.send(ServiceCommand::Stop(reply)).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), wait).await;
    let mut task = runtime.task;
    if tokio::time::timeout(Duration::from_secs(2), &mut task)
        .await
        .is_err()
    {
        task.abort();
    }
    let _ =
        friends::set_network_status_for(&runtime.account_id, false, runtime.port, None, None, 0)
            .map(|snapshot| emit_snapshot(&runtime.account_id, snapshot));
}

/// Update the status that friends see. The value is persisted so a restart
/// keeps the last known activity; the running hub is asked to push it now.
pub async fn set_self_presence(account_id: &str, presence: &str) -> Result<(), String> {
    if let Some(snapshot) = friends::set_self_presence_for(account_id, presence)? {
        emit_snapshot(account_id, snapshot);
    }
    send_command(ServiceCommand::PresenceChanged);
    Ok(())
}

// ── Wire format ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WireMessage {
    pub v: u8,
    pub from: String,
    pub kind: String,
    pub ts: u64,
    pub nonce: String,
    pub body: Value,
    pub sig: String,
}

fn build_wire(identity: &FriendIdentity, kind: &str, body: Value) -> WireMessage {
    let nonce = Uuid::new_v4().to_string();
    let ts = friends::now_millis();
    let sig = identity.sign_b64(&friends::canonical_wire_bytes(
        kind,
        identity.public_hex(),
        ts,
        &nonce,
        &body,
    ));
    WireMessage {
        v: WIRE_VERSION,
        from: identity.public_hex().to_string(),
        kind: kind.to_string(),
        ts,
        nonce,
        body,
        sig,
    }
}

fn wire_from_outbox(from: &str, item: &crate::friends::OutboxItem) -> WireMessage {
    WireMessage {
        v: WIRE_VERSION,
        from: from.to_string(),
        kind: item.kind.clone(),
        ts: item.signed_at,
        nonce: item.id.clone(),
        body: item.body.clone(),
        sig: item.signature.clone(),
    }
}

fn verify_wire(message: &WireMessage) -> Result<(), String> {
    if message.v != WIRE_VERSION
        || friends::normalize_public_key(&message.from).as_deref() != Some(message.from.as_str())
        || message.kind.is_empty()
        || message.kind.len() > 32
        || message.nonce.is_empty()
        || message.nonce.len() > 64
        || message.nonce.chars().any(char::is_control)
        || !message.body.is_object()
        || message.sig.is_empty()
        || message.sig.len() > 256
        || !transport_kind(&message.kind)
    {
        return Err("好友消息格式无效".into());
    }
    let bytes = friends::canonical_wire_bytes(
        &message.kind,
        &message.from,
        message.ts,
        &message.nonce,
        &message.body,
    );
    friends::verify_signature_b64(&message.from, &bytes, &message.sig)
}

fn transport_kind(kind: &str) -> bool {
    friends::WIRE_KINDS.contains(&kind)
        || matches!(kind, "ack" | "relay" | "relay_ack" | "ping" | "pong")
}

fn relationship_kind(kind: &str) -> bool {
    friends::WIRE_KINDS.contains(&kind)
}

pub(crate) fn parse_remote_event(kind: &str, body: &Value) -> Result<RemoteEvent, String> {
    let string = |key: &str| -> Result<String, String> {
        body.get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("好友消息缺少字段 {key}"))
    };
    let optional_string = |key: &str| -> Option<String> {
        body.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let revision = || {
        body.get("revision")
            .and_then(Value::as_u64)
            .ok_or_else(|| "好友消息缺少版本号".to_string())
    };
    match kind {
        "friend_request" => Ok(RemoteEvent::Request {
            request_id: string("request_id")?,
            to_key: string("to")?,
            profile_id: string("profile_id")?,
            name: string("name").unwrap_or_else(|_| "Minecraft 玩家".into()),
            revision: revision()?,
            endpoint: optional_string("endpoint"),
        }),
        "friend_accept" => Ok(RemoteEvent::Accept {
            request_id: optional_string("request_id").unwrap_or_default(),
            profile_id: string("profile_id")?,
            name: string("name").unwrap_or_else(|_| "Minecraft 玩家".into()),
            revision: revision()?,
        }),
        "friend_decline" => Ok(RemoteEvent::Decline {
            request_id: optional_string("request_id").unwrap_or_default(),
            revision: revision()?,
        }),
        "friend_remove" => Ok(RemoteEvent::Remove {
            revision: revision()?,
        }),
        "friend_cancel" => Ok(RemoteEvent::Cancel {
            request_id: optional_string("request_id").unwrap_or_default(),
            revision: revision()?,
        }),
        "presence" => Ok(RemoteEvent::Presence {
            profile_id: string("profile_id")?,
            name: string("name").unwrap_or_else(|_| "Minecraft 玩家".into()),
            state: string("state")?,
            endpoint: optional_string("endpoint"),
            updated_at: body
                .get("updated_at")
                .and_then(Value::as_u64)
                .unwrap_or_else(friends::now_millis),
        }),
        _ => Err("未知好友消息类型".into()),
    }
}

fn presence_body(account_id: &str, snapshot: &FriendSnapshot) -> Value {
    let (profile_id, name) = friends::self_profile_for(account_id);
    json!({
        "profile_id": profile_id,
        "name": name,
        "state": snapshot.self_presence,
        "endpoint": snapshot.network.endpoint,
        "updated_at": friends::now_millis(),
    })
}

// ── Hub ──────────────────────────────────────────────────────────────────────

enum HubEvent {
    Connected {
        peer_key: String,
        conn_id: u64,
        addr: SocketAddr,
        outbound: bool,
        preferred: bool,
        tx: mpsc::Sender<WireMessage>,
    },
    Message {
        peer_key: String,
        conn_id: u64,
        message: WireMessage,
        addr: SocketAddr,
    },
    Closed {
        peer_key: String,
        conn_id: u64,
    },
}

struct Connection {
    id: u64,
    preferred: bool,
    #[allow(dead_code)]
    addr: SocketAddr,
    #[allow(dead_code)]
    outbound: bool,
    connected_at: Instant,
    tx: mpsc::Sender<WireMessage>,
}

struct Hub {
    account_id: String,
    identity: Arc<FriendIdentity>,
    port: u16,
    endpoint: String,
    events: mpsc::Sender<HubEvent>,
    receiver: mpsc::Receiver<HubEvent>,
    commands: mpsc::Receiver<ServiceCommand>,
    connections: HashMap<String, Connection>,
    last_connect: HashMap<String, Instant>,
    last_presence: HashMap<String, Instant>,
    /// Outbox item id → (connection id, last send time).
    sent: HashMap<String, (u64, Instant)>,
    /// (outbox item id, relay key) → last relay attempt.
    last_relay: HashMap<(String, String), Instant>,
    /// Last full snapshot broadcast. Recovers GUI/game state after a dropped
    /// event without needing a visible change.
    last_snapshot: Instant,
}

impl Hub {
    async fn run(mut self, listener: TcpListener) {
        let accept = spawn_accept_loop(listener, self.identity.clone(), self.events.clone());
        let mut ticker = tokio::time::interval(MAINTENANCE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            enum Wake {
                Command(ServiceCommand),
                Event(HubEvent),
                Tick,
            }
            let wake = tokio::select! {
                Some(command) = self.commands.recv() => Wake::Command(command),
                Some(event) = self.receiver.recv() => Wake::Event(event),
                _ = ticker.tick() => Wake::Tick,
            };
            match wake {
                Wake::Command(command) => {
                    if self.handle_command(command) {
                        break;
                    }
                }
                Wake::Event(event) => self.handle_event(event),
                Wake::Tick => {
                    if !self.maintenance() {
                        break;
                    }
                }
            }
        }
        accept.abort();
        self.broadcast_offline();
        // Give connection tasks a moment to flush the queued offline presence
        // before their writer channels close.
        tokio::time::sleep(Duration::from_millis(250)).await;
        self.connections.clear();
        self.publish_network(false, None);
    }

    /// Tell connected friends immediately that this launcher is going away;
    /// otherwise they only notice after the heartbeat timeout.
    fn broadcast_offline(&self) {
        let Ok(snapshot) = friends::load_for(&self.account_id) else {
            return;
        };
        let mut body = presence_body(&self.account_id, &snapshot);
        body["state"] = json!("offline");
        let wire = build_wire(&self.identity, "presence", body);
        for key in self.connections.keys() {
            if friends::is_friend_key(&snapshot, key) {
                let _ = self.send_to(key, wire.clone());
            }
        }
    }

    fn handle_command(&mut self, command: ServiceCommand) -> bool {
        match command {
            ServiceCommand::Wake => {
                self.flush_outbox();
                // A local accept/remove/settings change can alter who is a
                // friend. Send presence now (rate-limited) instead of waiting
                // for the next 30 s maintenance tick.
                if let Ok(snapshot) = friends::load_for(&self.account_id) {
                    self.push_periodic_presence(&snapshot);
                    self.push_relayed_presence(&snapshot);
                }
            }
            ServiceCommand::PresenceChanged => {
                self.last_presence.clear();
                self.push_presence();
            }
            ServiceCommand::Stop(reply) => {
                self.publish_network(false, None);
                let _ = reply.send(());
                return true;
            }
        }
        false
    }

    fn handle_event(&mut self, event: HubEvent) {
        match event {
            HubEvent::Connected {
                peer_key,
                conn_id,
                addr,
                outbound,
                preferred,
                tx,
            } => {
                if peer_key == self.identity.public_hex() {
                    return;
                }
                let replace = match self.connections.get(&peer_key) {
                    None => true,
                    Some(existing) => preferred || !existing.preferred,
                };
                if !replace {
                    return;
                }
                let previous = self.connections.insert(
                    peer_key.clone(),
                    Connection {
                        id: conn_id,
                        preferred,
                        addr,
                        outbound,
                        connected_at: Instant::now(),
                        tx,
                    },
                );
                if let Some(previous) = previous {
                    self.sent.retain(|_, (id, _)| *id != previous.id);
                    drop(previous.tx);
                }
                self.last_connect.remove(&peer_key);
                self.last_presence.remove(&peer_key);
                self.send_presence_to(&peer_key);
                self.flush_outbox();
                self.publish_network(true, None);
            }
            HubEvent::Message {
                peer_key,
                conn_id,
                message,
                addr,
            } => {
                if self
                    .connections
                    .get(&peer_key)
                    .map(|connection| connection.id)
                    != Some(conn_id)
                {
                    return;
                }
                self.handle_message(peer_key, message, addr);
            }
            HubEvent::Closed { peer_key, conn_id } => {
                let closed = self
                    .connections
                    .get(&peer_key)
                    .map(|connection| connection.id == conn_id)
                    .unwrap_or(false);
                if !closed {
                    return;
                }
                self.connections.remove(&peer_key);
                self.sent.retain(|_, (id, _)| *id != conn_id);
                self.last_presence.remove(&peer_key);
                self.publish_network(true, None);
            }
        }
    }

    fn handle_message(&mut self, peer_key: String, message: WireMessage, addr: SocketAddr) {
        if verify_wire(&message).is_err() || message.from != peer_key {
            return;
        }
        match message.kind.as_str() {
            "ping" | "pong" => {}
            "ack" => self.handle_ack(&message),
            "relay" => self.handle_relay(&peer_key, &message),
            "relay_ack" => self.handle_relay_ack(&peer_key, &message),
            kind if relationship_kind(kind) => {
                let wants_ack = kind != "presence";
                let event = match parse_remote_event(kind, &message.body) {
                    Ok(event) => event,
                    Err(reason) => {
                        if wants_ack {
                            self.send_ack(&peer_key, &message.nonce, false, Some(&reason));
                        }
                        return;
                    }
                };
                match friends::apply_remote_event_for(&self.account_id, &peer_key, event, None) {
                    Ok(outcome) => {
                        self.after_outcome(&peer_key, &message.nonce, outcome, addr, wants_ack);
                    }
                    Err(reason) => {
                        if wants_ack {
                            self.send_ack(&peer_key, &message.nonce, false, Some(&reason));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn after_outcome(
        &mut self,
        peer_key: &str,
        op: &str,
        outcome: RemoteOutcome,
        _addr: SocketAddr,
        wants_ack: bool,
    ) {
        let changed = outcome.snapshot.clone();
        if let Some(snapshot) = changed {
            if friends::is_friend_key(&snapshot, peer_key) {
                self.send_presence_to(peer_key);
            }
            emit_snapshot(&self.account_id, snapshot);
        }
        if wants_ack {
            self.send_ack(peer_key, op, outcome.accepted, outcome.message.as_deref());
        }
        if outcome.close_peer {
            // Keep the socket long enough for the acknowledgement above to
            // flush; maintenance prunes connections to non-friends.
            self.last_presence.remove(peer_key);
        }
    }

    fn send_ack(&self, peer_key: &str, op: &str, accepted: bool, reason: Option<&str>) {
        let body = json!({
            "op": op,
            "status": if accepted { "ok" } else { "rejected" },
            "reason": reason.unwrap_or(""),
        });
        self.send_to(peer_key, build_wire(&self.identity, "ack", body));
    }

    fn handle_ack(&mut self, message: &WireMessage) {
        let Some(op) = message.body.get("op").and_then(Value::as_str) else {
            return;
        };
        // Only the addressed peer may acknowledge an outbox item.
        if let Ok(Some(peer)) = friends::outbox_peer_for(&self.account_id, op) {
            if peer != message.from {
                return;
            }
        }
        let status = message
            .body
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("ok");
        let accepted = status == "ok";
        if status == "relayed" {
            return;
        }
        if let Ok(Some(snapshot)) = friends::ack_outbox_for(&self.account_id, op, accepted) {
            emit_snapshot(&self.account_id, snapshot);
        }
        self.sent.remove(op);
    }

    fn handle_relay(&mut self, relay_key: &str, message: &WireMessage) {
        let Some(target) = message.body.get("to").and_then(Value::as_str) else {
            return;
        };
        let Some(inner_value) = message.body.get("inner").cloned() else {
            return;
        };
        let hops = message
            .body
            .get("hops")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let Ok(inner) = serde_json::from_value::<WireMessage>(inner_value) else {
            return;
        };
        if verify_wire(&inner).is_err() || inner.from == self.identity.public_hex() {
            return;
        }
        let snapshot = match friends::load_for(&self.account_id) {
            Ok(snapshot) => snapshot,
            Err(_) => return,
        };
        if !friends::is_friend_key(&snapshot, relay_key) {
            return;
        }
        if target == self.identity.public_hex() {
            if inner.kind == "ack" {
                self.handle_ack(&inner);
                return;
            }
            let event = match parse_remote_event(&inner.kind, &inner.body) {
                Ok(event) => event,
                Err(reason) => {
                    if inner.kind != "presence" {
                        self.send_relay_ack(relay_key, &inner, false, Some(&reason));
                    }
                    return;
                }
            };
            match friends::apply_remote_event_for(&self.account_id, &inner.from, event, None) {
                Ok(outcome) => {
                    let accepted = outcome.accepted;
                    let reason = outcome.message.clone();
                    if let Some(changed) = outcome.snapshot.clone() {
                        if friends::is_friend_key(&changed, &inner.from) {
                            // Reply over an existing direct link; a relayed
                            // presence is answered by the next periodic send.
                            self.send_presence_to(&inner.from);
                        }
                        emit_snapshot(&self.account_id, changed);
                    }
                    if outcome.close_peer {
                        self.connections.remove(&inner.from);
                    }
                    // Presence is periodic and needs no acknowledgement.
                    if inner.kind != "presence" {
                        self.send_relay_ack(relay_key, &inner, accepted, reason.as_deref());
                    }
                }
                Err(reason) => {
                    if inner.kind != "presence" {
                        self.send_relay_ack(relay_key, &inner, false, Some(&reason));
                    }
                }
            }
        } else if hops == 0
            && friends::is_friend_key(&snapshot, target)
            && (inner.kind != "friend_request"
                || inner.body.get("to").and_then(Value::as_str) == Some(target))
        {
            if let Some(connection) = self.connections.get(target) {
                let body = json!({
                    "to": target,
                    "inner": inner,
                    "hops": 1,
                });
                let _ = connection
                    .tx
                    .try_send(build_wire(&self.identity, "relay", body));
            }
        }
    }

    fn handle_relay_ack(&mut self, from_key: &str, message: &WireMessage) {
        let Some(target) = message.body.get("to").and_then(Value::as_str) else {
            return;
        };
        let Some(ack_value) = message.body.get("ack").cloned() else {
            return;
        };
        let Ok(ack) = serde_json::from_value::<WireMessage>(ack_value) else {
            return;
        };
        if verify_wire(&ack).is_err() || ack.from != from_key {
            return;
        }
        if target == self.identity.public_hex() {
            self.handle_ack(&ack);
        } else if let Some(connection) = self.connections.get(target) {
            let body = json!({
                "to": target,
                "inner": ack,
                "hops": 1,
            });
            let _ = connection
                .tx
                .try_send(build_wire(&self.identity, "relay", body));
        }
    }

    fn send_relay_ack(
        &self,
        relay_key: &str,
        inner: &WireMessage,
        accepted: bool,
        reason: Option<&str>,
    ) {
        let ack = build_wire(
            &self.identity,
            "ack",
            json!({
                "op": inner.nonce,
                "status": if accepted { "ok" } else { "rejected" },
                "reason": reason.unwrap_or(""),
            }),
        );
        let body = json!({
            "to": inner.from,
            "ack": ack,
        });
        self.send_to(relay_key, build_wire(&self.identity, "relay_ack", body));
    }

    fn maintenance(&mut self) -> bool {
        let snapshot = match friends::load_for(&self.account_id) {
            Ok(snapshot) => snapshot,
            Err(_) => return true,
        };
        if !snapshot.enabled {
            self.publish_network(false, None);
            return false;
        }
        let now = friends::now_millis();
        if let Ok(Some(snapshot)) =
            friends::expire_presence_for(&self.account_id, now.saturating_sub(OFFLINE_AFTER_MS))
        {
            emit_snapshot(&self.account_id, snapshot);
        }
        let targets = friends::connection_targets(&snapshot);
        let mut attempts = 0;
        for (key, endpoint) in targets {
            if attempts >= 16 {
                break;
            }
            if self.connections.contains_key(&key) {
                continue;
            }
            let due = self
                .last_connect
                .get(&key)
                .map(|last| last.elapsed() >= RECONNECT_INTERVAL)
                .unwrap_or(true);
            if due {
                attempts += 1;
                self.last_connect.insert(key.clone(), Instant::now());
                spawn_connect(key, endpoint, self.identity.clone(), self.events.clone());
            }
        }
        self.flush_outbox();
        self.push_periodic_presence(&snapshot);
        self.push_relayed_presence(&snapshot);
        self.prune_connections(&snapshot);
        if self.last_snapshot.elapsed() >= Duration::from_secs(30) {
            self.last_snapshot = Instant::now();
            emit_snapshot(&self.account_id, snapshot);
        }
        true
    }

    fn prune_connections(&mut self, snapshot: &FriendSnapshot) {
        let keep: HashSet<String> = snapshot
            .friends
            .iter()
            .chain(snapshot.incoming_requests.iter())
            .chain(snapshot.outgoing_requests.iter())
            .filter_map(|entry| entry.key.clone())
            .collect();
        let drop: Vec<String> = self
            .connections
            .iter()
            .filter(|(key, connection)| {
                !keep.contains(*key) && connection.connected_at.elapsed() >= Duration::from_secs(15)
            })
            .map(|(key, _)| key.clone())
            .collect();
        if drop.is_empty() {
            return;
        }
        for key in drop {
            self.connections.remove(&key);
            self.last_presence.remove(&key);
        }
        self.publish_network(true, None);
    }

    fn flush_outbox(&mut self) {
        let Ok(snapshot) = friends::load_for(&self.account_id) else {
            return;
        };
        let now = friends::now_millis();
        for item in snapshot.outbox.iter() {
            if now < item.next_attempt {
                continue;
            }
            if let Some(connection) = self.connections.get(&item.peer_key) {
                let should_send = self
                    .sent
                    .get(&item.id)
                    .map(|(conn, at)| *conn != connection.id || at.elapsed() >= PRESENCE_INTERVAL)
                    .unwrap_or(true);
                if !should_send {
                    continue;
                }
                let wire = wire_from_outbox(&snapshot.identity, item);
                if serde_json::to_vec(&wire)
                    .map(|bytes| bytes.len() <= OUTBOX_WIRE_BUDGET)
                    .unwrap_or(false)
                    && connection.tx.try_send(wire).is_ok()
                {
                    self.sent
                        .insert(item.id.clone(), (connection.id, Instant::now()));
                }
                continue;
            }
            if item.attempts == 0 {
                let _ = friends::mark_outbox_attempt(
                    &self.account_id,
                    &item.id,
                    1,
                    now.saturating_add(5_000),
                    None,
                );
                continue;
            }
            let relays: Vec<String> = self
                .connections
                .keys()
                .filter(|key| *key != &item.peer_key && friends::is_friend_key(&snapshot, key))
                .cloned()
                .collect();
            if relays.is_empty() {
                continue;
            }
            let inner = wire_from_outbox(&snapshot.identity, item);
            let inner_json = match serde_json::to_value(&inner) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let mut relayed = false;
            for relay in &relays {
                let key = (item.id.clone(), relay.clone());
                if self
                    .last_relay
                    .get(&key)
                    .map(|last| last.elapsed() < Duration::from_secs(30))
                    .unwrap_or(false)
                {
                    continue;
                }
                if let Some(connection) = self.connections.get(relay) {
                    let body = json!({
                        "to": item.peer_key,
                        "inner": inner_json,
                        "hops": 0,
                    });
                    if connection
                        .tx
                        .try_send(build_wire(&self.identity, "relay", body))
                        .is_ok()
                    {
                        self.last_relay.insert(key, Instant::now());
                        relayed = true;
                    }
                }
            }
            if relayed {
                let backoff =
                    (5_000u64.saturating_mul(u64::from(item.attempts.min(12)))).min(120_000);
                let _ = friends::mark_outbox_attempt(
                    &self.account_id,
                    &item.id,
                    item.attempts.saturating_add(1),
                    now.saturating_add(backoff),
                    relays.first().map(String::as_str),
                );
            }
            if self.last_relay.len() > 512 {
                self.last_relay
                    .retain(|_, at| at.elapsed() < Duration::from_secs(300));
            }
        }
    }

    fn push_presence(&mut self) {
        let Ok(snapshot) = friends::load_for(&self.account_id) else {
            return;
        };
        let keys: Vec<String> = self.connections.keys().cloned().collect();
        for key in keys {
            if friends::is_friend_key(&snapshot, &key) {
                self.send_presence_with(&snapshot, &key);
            }
        }
    }

    fn push_periodic_presence(&mut self, snapshot: &FriendSnapshot) {
        let keys: Vec<String> = self.connections.keys().cloned().collect();
        for key in keys {
            if !friends::is_friend_key(snapshot, &key) {
                continue;
            }
            let due = self
                .last_presence
                .get(&key)
                .map(|last| last.elapsed() >= PRESENCE_INTERVAL)
                .unwrap_or(true);
            if due {
                self.send_presence_with(snapshot, &key);
            }
        }
    }

    /// NAT-only friends still need presence. Reuse the single-hop relay path
    /// with a lower rate, and only through friends already connected to us.
    fn push_relayed_presence(&mut self, snapshot: &FriendSnapshot) {
        if !snapshot.enabled {
            return;
        }
        let relays: Vec<String> = self
            .connections
            .keys()
            .filter(|key| friends::is_friend_key(snapshot, key))
            .cloned()
            .collect();
        if relays.is_empty() {
            return;
        }
        let inner = build_wire(
            &self.identity,
            "presence",
            presence_body(&self.account_id, snapshot),
        );
        let Ok(inner_json) = serde_json::to_value(&inner) else {
            return;
        };
        let mut budget = 16;
        for friend in snapshot.friends.iter() {
            if budget == 0 {
                break;
            }
            let Some(key) = friend.key.as_deref() else {
                continue;
            };
            if self.connections.contains_key(key) {
                continue;
            }
            let due = self
                .last_presence
                .get(key)
                .map(|last| last.elapsed() >= PRESENCE_RELAY_INTERVAL)
                .unwrap_or(true);
            if !due {
                continue;
            }
            let mut sent = false;
            for relay in relays.iter().filter(|relay| relay.as_str() != key).take(3) {
                let body = json!({
                    "to": key,
                    "inner": inner_json.clone(),
                    "hops": 0,
                });
                let wire = build_wire(&self.identity, "relay", body);
                if self.send_to(relay, wire) {
                    sent = true;
                }
            }
            if sent {
                budget -= 1;
                self.last_presence.insert(key.to_string(), Instant::now());
            }
        }
    }

    fn send_presence_to(&mut self, key: &str) {
        let Ok(snapshot) = friends::load_for(&self.account_id) else {
            return;
        };
        if friends::is_friend_key(&snapshot, key) {
            self.send_presence_with(&snapshot, key);
        }
    }

    fn send_presence_with(&mut self, snapshot: &FriendSnapshot, key: &str) {
        if !snapshot.enabled {
            return;
        }
        let body = presence_body(&self.account_id, snapshot);
        let wire = build_wire(&self.identity, "presence", body);
        if self.send_to(key, wire) {
            self.last_presence.insert(key.to_string(), Instant::now());
        }
    }

    fn send_to(&self, key: &str, message: WireMessage) -> bool {
        self.connections
            .get(key)
            .map(|connection| connection.tx.try_send(message).is_ok())
            .unwrap_or(false)
    }

    fn publish_network(&self, running: bool, error: Option<&str>) {
        let online = if running {
            self.connections.len() as u32
        } else {
            0
        };
        match friends::set_network_status_for(
            &self.account_id,
            running,
            self.port,
            Some(&self.endpoint),
            error,
            online,
        ) {
            Ok(snapshot) => emit_snapshot(&self.account_id, snapshot),
            Err(error) => eprintln!("好友网络状态写入失败: {error}"),
        }
    }
}

fn is_preferred(self_key: &str, peer_key: &str, outbound: bool) -> bool {
    let initiator = if outbound { self_key } else { peer_key };
    let smaller = if self_key < peer_key {
        self_key
    } else {
        peer_key
    };
    initiator == smaller
}

fn spawn_accept_loop(
    listener: TcpListener,
    identity: Arc<FriendIdentity>,
    events: mpsc::Sender<HubEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let permits = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        loop {
            let Ok(permit) = permits.clone().acquire_owned().await else {
                break;
            };
            let Ok((stream, addr)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            let identity = identity.clone();
            let events = events.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let result = tokio::time::timeout(
                    HANDSHAKE_TIMEOUT,
                    secure::server_handshake(stream, identity.clone()),
                )
                .await;
                let Ok(Ok((mut read, mut write, peer_key))) = result else {
                    return;
                };
                let conn_id = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
                let preferred = is_preferred(identity.public_hex(), &peer_key, false);
                let (tx, rx) = mpsc::channel(64);
                if events
                    .send(HubEvent::Connected {
                        peer_key: peer_key.clone(),
                        conn_id,
                        addr,
                        outbound: false,
                        preferred,
                        tx,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                connection_loop(
                    &mut read, &mut write, identity, peer_key, conn_id, rx, events, addr,
                )
                .await;
            });
        }
    })
}

fn spawn_connect(
    peer_key: String,
    endpoint: String,
    identity: Arc<FriendIdentity>,
    events: mpsc::Sender<HubEvent>,
) {
    tokio::spawn(async move {
        if peer_key == identity.public_hex() {
            return;
        }
        let Ok(addrs) = tokio::net::lookup_host(endpoint.as_str()).await else {
            return;
        };
        for addr in addrs.take(2) {
            let Ok(Ok(stream)) =
                tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await
            else {
                continue;
            };
            let result = tokio::time::timeout(
                HANDSHAKE_TIMEOUT,
                secure::client_handshake(stream, identity.clone(), Some(&peer_key)),
            )
            .await;
            let Ok(Ok((mut read, mut write, actual_key))) = result else {
                continue;
            };
            if actual_key != peer_key {
                return;
            }
            let conn_id = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
            let preferred = is_preferred(identity.public_hex(), &peer_key, true);
            let (tx, rx) = mpsc::channel(64);
            if events
                .send(HubEvent::Connected {
                    peer_key: peer_key.clone(),
                    conn_id,
                    addr,
                    outbound: true,
                    preferred,
                    tx,
                })
                .await
                .is_err()
            {
                return;
            }
            connection_loop(
                &mut read, &mut write, identity, peer_key, conn_id, rx, events, addr,
            )
            .await;
            return;
        }
    });
}

async fn connection_loop(
    read: &mut secure::SecureRead,
    write: &mut secure::SecureWrite,
    identity: Arc<FriendIdentity>,
    peer_key: String,
    conn_id: u64,
    mut outgoing: mpsc::Receiver<WireMessage>,
    events: mpsc::Sender<HubEvent>,
    addr: SocketAddr,
) {
    let mut last_inbound = Instant::now();
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + HEARTBEAT_INTERVAL,
        HEARTBEAT_INTERVAL,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        enum Select {
            Inbound(Result<WireMessage, String>),
            Outbound(Option<WireMessage>),
            Tick,
        }
        let selected = tokio::select! {
            message = read.receive() => Select::Inbound(message),
            message = outgoing.recv() => Select::Outbound(message),
            _ = heartbeat.tick() => Select::Tick,
        };
        match selected {
            Select::Inbound(Ok(message)) => {
                last_inbound = Instant::now();
                if verify_wire(&message).is_err() || message.from != peer_key {
                    break;
                }
                match message.kind.as_str() {
                    "ping" => {
                        let pong = build_wire(&identity, "pong", json!({}));
                        if write.send(&pong).await.is_err() {
                            break;
                        }
                    }
                    "pong" => {}
                    _ => {
                        if events
                            .send(HubEvent::Message {
                                peer_key: peer_key.clone(),
                                conn_id,
                                message,
                                addr,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            Select::Inbound(Err(_)) => break,
            Select::Outbound(Some(message)) => {
                if write.send(&message).await.is_err() {
                    break;
                }
            }
            Select::Outbound(None) => break,
            Select::Tick => {
                if last_inbound.elapsed() >= HEARTBEAT_TIMEOUT {
                    break;
                }
                let ping = build_wire(&identity, "ping", json!({}));
                if write.send(&ping).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = events.send(HubEvent::Closed { peer_key, conn_id }).await;
}

// ── Secure transport ─────────────────────────────────────────────────────────

mod secure {
    use super::*;
    use base64::Engine;
    use ring::aead;
    use ring::agreement;
    use ring::hkdf;
    use ring::rand::{SecureRandom, SystemRandom};
    use ring::signature::{UnparsedPublicKey, ED25519};

    #[derive(Debug, Serialize, Deserialize)]
    struct Hello {
        v: u8,
        kind: String,
        key: String,
        eph: String,
        rand: String,
        sig: String,
    }

    pub struct SecureRead {
        reader: tokio::net::tcp::OwnedReadHalf,
        key: aead::LessSafeKey,
        counter: u64,
    }

    pub struct SecureWrite {
        writer: tokio::net::tcp::OwnedWriteHalf,
        key: aead::LessSafeKey,
        counter: u64,
    }

    impl SecureRead {
        pub async fn receive(&mut self) -> Result<WireMessage, String> {
            let mut length = [0u8; 4];
            self.reader
                .read_exact(&mut length)
                .await
                .map_err(|_| "好友连接已断开".to_string())?;
            let length = u32::from_be_bytes(length) as usize;
            if length == 0 || length > FRAME_MAX {
                return Err("好友消息帧过大".into());
            }
            let mut ciphertext = vec![0u8; length];
            self.reader
                .read_exact(&mut ciphertext)
                .await
                .map_err(|_| "好友连接已断开".to_string())?;
            let nonce = nonce_for(self.counter);
            self.counter = self.counter.wrapping_add(1);
            let plain = self
                .key
                .open_in_place(nonce, aead::Aad::empty(), &mut ciphertext)
                .map_err(|_| "好友消息解密失败".to_string())?;
            serde_json::from_slice(plain).map_err(|_| "好友消息格式无效".into())
        }
    }

    impl SecureWrite {
        pub async fn send(&mut self, message: &WireMessage) -> Result<(), String> {
            let mut bytes = serde_json::to_vec(message).map_err(|e| e.to_string())?;
            if bytes.len() > FRAME_MAX - 16 {
                return Err("好友消息过大".into());
            }
            let nonce = nonce_for(self.counter);
            self.counter = self.counter.wrapping_add(1);
            self.key
                .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut bytes)
                .map_err(|_| "好友消息加密失败".to_string())?;
            let length = u32::try_from(bytes.len()).map_err(|_| "好友消息过大".to_string())?;
            self.writer
                .write_all(&length.to_be_bytes())
                .await
                .map_err(|_| "好友连接已断开".to_string())?;
            self.writer
                .write_all(&bytes)
                .await
                .map_err(|_| "好友连接已断开".to_string())
        }
    }

    fn nonce_for(counter: u64) -> aead::Nonce {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&counter.to_be_bytes());
        aead::Nonce::assume_unique_for_key(nonce)
    }

    pub async fn client_handshake(
        mut stream: TcpStream,
        identity: Arc<FriendIdentity>,
        expected_peer: Option<&str>,
    ) -> Result<(SecureRead, SecureWrite, String), String> {
        stream
            .set_nodelay(true)
            .map_err(|e| format!("设置好友连接参数失败: {e}"))?;
        let rng = SystemRandom::new();
        let ephemeral = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
            .map_err(|_| "无法生成好友会话密钥".to_string())?;
        let ephemeral_public = ephemeral
            .compute_public_key()
            .map_err(|_| "无法生成好友公钥".to_string())?;
        let local_rand = random_32(&rng)?;
        let first = Hello {
            v: WIRE_VERSION,
            kind: "hello".into(),
            key: b64(identity.public_key()),
            eph: b64(ephemeral_public.as_ref()),
            rand: b64(&local_rand),
            sig: String::new(),
        };
        let first_sig = handshake_signature(
            &identity,
            b"DSF1-H1",
            &[
                identity.public_key(),
                ephemeral_public.as_ref(),
                &local_rand,
            ],
        );
        let first = Hello {
            sig: first_sig,
            ..first
        };
        write_plain(&mut stream, &first).await?;
        let second: Hello = read_plain(&mut stream).await?;
        validate_hello(&second)?;
        if second.kind != "hello" {
            return Err("好友握手阶段无效".into());
        }
        let peer_key = decode_key(&second.key)?;
        let peer_key_hex = hex::encode(peer_key);
        if let Some(expected) = expected_peer {
            if friends::normalize_public_key(expected).as_deref() != Some(peer_key_hex.as_str()) {
                return Err("好友身份与邀请码不匹配".into());
            }
        }
        if peer_key_hex == identity.public_hex() {
            return Err("不能连接自己".into());
        }
        let peer_eph = decode_key(&second.eph)?;
        let peer_rand = decode_key(&second.rand)?;
        // The responder signs with its own key; verify manually.
        let signed = handshake_bytes(
            b"DSF1-H2",
            &[
                identity.public_key(),
                ephemeral_public.as_ref(),
                &local_rand,
                &peer_key,
                &peer_eph,
                &peer_rand,
            ],
        );
        verify_raw(&peer_key, &signed, &second.sig)?;
        let finish = Hello {
            v: WIRE_VERSION,
            kind: "finish".into(),
            key: first.key.clone(),
            eph: first.eph.clone(),
            rand: first.rand.clone(),
            sig: handshake_signature(
                &identity,
                b"DSF1-H3",
                &[
                    identity.public_key(),
                    ephemeral_public.as_ref(),
                    &local_rand,
                    &peer_key,
                    &peer_eph,
                    &peer_rand,
                ],
            ),
        };
        write_plain(&mut stream, &finish).await?;
        let shared = agreement::agree_ephemeral(
            ephemeral,
            &agreement::UnparsedPublicKey::new(&agreement::X25519, &peer_eph),
            |secret| secret.to_vec(),
        )
        .map_err(|_| "好友会话密钥协商失败".to_string())?;
        let (read, write) = split_stream(
            stream,
            &shared,
            true,
            identity.public_hex(),
            &peer_key_hex,
            &local_rand,
            &peer_rand,
        )?;
        Ok((read, write, peer_key_hex))
    }

    pub async fn server_handshake(
        mut stream: TcpStream,
        identity: Arc<FriendIdentity>,
    ) -> Result<(SecureRead, SecureWrite, String), String> {
        stream
            .set_nodelay(true)
            .map_err(|e| format!("设置好友连接参数失败: {e}"))?;
        let first: Hello = read_plain(&mut stream).await?;
        validate_hello(&first)?;
        if first.kind != "hello" {
            return Err("好友握手阶段无效".into());
        }
        let initiator_key = decode_key(&first.key)?;
        let initiator_key_hex = hex::encode(initiator_key);
        if initiator_key_hex == identity.public_hex() {
            return Err("不能连接自己".into());
        }
        let initiator_eph = decode_key(&first.eph)?;
        let initiator_rand = decode_key(&first.rand)?;
        let signed = handshake_bytes(
            b"DSF1-H1",
            &[&initiator_key, &initiator_eph, &initiator_rand],
        );
        verify_raw(&initiator_key, &signed, &first.sig)?;

        let rng = SystemRandom::new();
        let ephemeral = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
            .map_err(|_| "无法生成好友会话密钥".to_string())?;
        let ephemeral_public = ephemeral
            .compute_public_key()
            .map_err(|_| "无法生成好友公钥".to_string())?;
        let local_rand = random_32(&rng)?;
        let second = Hello {
            v: WIRE_VERSION,
            kind: "hello".into(),
            key: b64(identity.public_key()),
            eph: b64(ephemeral_public.as_ref()),
            rand: b64(&local_rand),
            sig: handshake_signature(
                &identity,
                b"DSF1-H2",
                &[
                    &initiator_key,
                    &initiator_eph,
                    &initiator_rand,
                    identity.public_key(),
                    ephemeral_public.as_ref(),
                    &local_rand,
                ],
            ),
        };
        write_plain(&mut stream, &second).await?;
        let finish: Hello = read_plain(&mut stream).await?;
        validate_hello(&finish)?;
        if finish.kind != "finish"
            || finish.key != first.key
            || finish.eph != first.eph
            || finish.rand != first.rand
        {
            return Err("好友握手确认无效".into());
        }
        let signed = handshake_bytes(
            b"DSF1-H3",
            &[
                &initiator_key,
                &initiator_eph,
                &initiator_rand,
                identity.public_key(),
                ephemeral_public.as_ref(),
                &local_rand,
            ],
        );
        verify_raw(&initiator_key, &signed, &finish.sig)?;

        let shared = agreement::agree_ephemeral(
            ephemeral,
            &agreement::UnparsedPublicKey::new(&agreement::X25519, &initiator_eph),
            |secret| secret.to_vec(),
        )
        .map_err(|_| "好友会话密钥协商失败".to_string())?;
        let (read, write) = split_stream(
            stream,
            &shared,
            false,
            &initiator_key_hex,
            identity.public_hex(),
            &initiator_rand,
            &local_rand,
        )?;
        Ok((read, write, initiator_key_hex))
    }

    fn split_stream(
        stream: TcpStream,
        shared: &[u8],
        initiator: bool,
        initiator_key: &str,
        responder_key: &str,
        initiator_rand: &[u8; 32],
        responder_rand: &[u8; 32],
    ) -> Result<(SecureRead, SecureWrite), String> {
        let salt = [&initiator_rand[..], &responder_rand[..]].concat();
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(shared);
        let info: [&[u8]; 5] = [
            b"deepslate-friends-v1",
            initiator_key.as_bytes(),
            responder_key.as_bytes(),
            initiator_rand,
            responder_rand,
        ];
        let mut keys = [0u8; 64];
        prk.expand(&info, AeadKeyLen(64))
            .map_err(|_| "好友会话密钥派生失败".to_string())?
            .fill(&mut keys)
            .map_err(|_| "好友会话密钥派生失败".to_string())?;
        let initiator_to_responder = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &keys[..32])
                .map_err(|_| "好友会话密钥无效".to_string())?,
        );
        let responder_to_initiator = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &keys[32..])
                .map_err(|_| "好友会话密钥无效".to_string())?,
        );
        let (read_half, write_half) = stream.into_split();
        if initiator {
            Ok((
                SecureRead {
                    reader: read_half,
                    key: responder_to_initiator,
                    counter: 0,
                },
                SecureWrite {
                    writer: write_half,
                    key: initiator_to_responder,
                    counter: 0,
                },
            ))
        } else {
            Ok((
                SecureRead {
                    reader: read_half,
                    key: initiator_to_responder,
                    counter: 0,
                },
                SecureWrite {
                    writer: write_half,
                    key: responder_to_initiator,
                    counter: 0,
                },
            ))
        }
    }

    struct AeadKeyLen(usize);

    impl hkdf::KeyType for AeadKeyLen {
        fn len(&self) -> usize {
            self.0
        }
    }

    fn validate_hello(hello: &Hello) -> Result<(), String> {
        if hello.v != WIRE_VERSION
            || hello.key.len() > 64
            || hello.eph.len() > 64
            || hello.rand.len() > 64
            || hello.sig.len() > 128
            || hello.kind.len() > 16
        {
            return Err("好友握手消息无效".into());
        }
        Ok(())
    }

    async fn write_plain(stream: &mut TcpStream, hello: &Hello) -> Result<(), String> {
        let bytes = serde_json::to_vec(hello).map_err(|e| e.to_string())?;
        if bytes.len() > HANDSHAKE_MAX {
            return Err("好友握手消息过大".into());
        }
        let length = u16::try_from(bytes.len()).map_err(|_| "好友握手消息过大".to_string())?;
        stream
            .write_all(&length.to_be_bytes())
            .await
            .map_err(|_| "好友握手连接已断开".to_string())?;
        stream
            .write_all(&bytes)
            .await
            .map_err(|_| "好友握手连接已断开".to_string())
    }

    async fn read_plain(stream: &mut TcpStream) -> Result<Hello, String> {
        let mut length = [0u8; 2];
        stream
            .read_exact(&mut length)
            .await
            .map_err(|_| "好友握手连接已断开".to_string())?;
        let length = u16::from_be_bytes(length) as usize;
        if length == 0 || length > HANDSHAKE_MAX {
            return Err("好友握手消息过大".into());
        }
        let mut bytes = vec![0u8; length];
        stream
            .read_exact(&mut bytes)
            .await
            .map_err(|_| "好友握手连接已断开".to_string())?;
        serde_json::from_slice(&bytes).map_err(|_| "好友握手消息格式无效".into())
    }

    fn random_32(rng: &SystemRandom) -> Result<[u8; 32], String> {
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes)
            .map_err(|_| "无法生成好友握手随机数".to_string())?;
        Ok(bytes)
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
    }

    fn decode_key(input: &str) -> Result<[u8; 32], String> {
        let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(input.as_bytes())
            .map_err(|_| "好友握手密钥编码无效".to_string())?;
        if bytes.len() != 32 {
            return Err("好友握手密钥长度无效".into());
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(key)
    }

    fn handshake_signature(identity: &FriendIdentity, domain: &[u8], fields: &[&[u8]]) -> String {
        identity.sign_b64(&handshake_bytes(domain, fields))
    }

    fn handshake_bytes(domain: &[u8], fields: &[&[u8]]) -> Vec<u8> {
        let mut out = domain.to_vec();
        for field in fields {
            out.extend_from_slice(&(field.len() as u32).to_be_bytes());
            out.extend_from_slice(field);
        }
        out
    }

    fn verify_raw(public_key: &[u8], message: &[u8], signature_b64: &str) -> Result<(), String> {
        let signature = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(signature_b64.as_bytes())
            .map_err(|_| "好友握手签名编码无效".to_string())?;
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(message, &signature)
            .map_err(|_| "好友握手签名无效".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn read_kind(read: &mut secure::SecureRead, kind: &str) -> WireMessage {
        for _ in 0..16 {
            let message = tokio::time::timeout(Duration::from_secs(3), read.receive())
                .await
                .unwrap()
                .unwrap();
            if message.kind == kind {
                return message;
            }
        }
        panic!("did not receive {kind}");
    }

    async fn read_ack_for(read: &mut secure::SecureRead, op: &str) -> WireMessage {
        for _ in 0..24 {
            let message = tokio::time::timeout(Duration::from_secs(3), read.receive())
                .await
                .unwrap()
                .unwrap();
            if message.kind == "ack" && message.body.get("op").and_then(Value::as_str) == Some(op) {
                return message;
            }
        }
        panic!("missing ack for {op}");
    }

    struct TestRoot(std::path::PathBuf);

    impl Drop for TestRoot {
        fn drop(&mut self) {
            friends::set_test_config_dir(None);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn preferred_connection_direction_is_symmetric() {
        let small = "aa".repeat(32);
        let large = "bb".repeat(32);
        // The key that sorts first initiates the preferred connection from
        // both perspectives.
        assert!(is_preferred(&small, &large, true));
        assert!(!is_preferred(&large, &small, true));
        assert!(!is_preferred(&small, &large, false));
        assert!(is_preferred(&large, &small, false));
    }

    #[test]
    fn presence_and_relationship_bodies_parse_independently() {
        let presence = json!({
            "profile_id": "00000000-0000-4000-8000-000000000001",
            "name": "Bob",
            "state": "online",
            "endpoint": "127.0.0.1:27731",
            "updated_at": 1,
        });
        assert!(matches!(
            parse_remote_event("presence", &presence).unwrap(),
            RemoteEvent::Presence { .. }
        ));
        let remove = json!({ "revision": 42 });
        assert!(matches!(
            parse_remote_event("friend_remove", &remove).unwrap(),
            RemoteEvent::Remove { revision: 42 }
        ));
        let request = json!({
            "request_id": "00000000-0000-4000-8000-000000000002",
            "to": "ab".repeat(32),
            "profile_id": "00000000-0000-4000-8000-000000000001",
            "name": "Bob",
            "revision": 42,
        });
        assert!(matches!(
            parse_remote_event("friend_request", &request).unwrap(),
            RemoteEvent::Request { .. }
        ));
        assert!(parse_remote_event("friend_request", &json!({})).is_err());
    }

    #[test]
    fn wire_signatures_cover_identity_and_body() {
        let identity = FriendIdentity::generate_for_test();
        let wire = build_wire(&identity, "presence", json!({"state": "online"}));
        assert!(verify_wire(&wire).is_ok());
        let mut tampered = wire.clone();
        tampered.body = json!({"state": "offline"});
        assert!(verify_wire(&tampered).is_err());
        let mut spoofed = wire;
        spoofed.from = "00".repeat(32);
        assert!(verify_wire(&spoofed).is_err());
    }

    #[tokio::test]
    async fn encrypted_handshake_round_trips_frames() {
        let server_identity = Arc::new(FriendIdentity::generate_for_test());
        let client_identity = Arc::new(FriendIdentity::generate_for_test());
        let server_identity_for_task = server_identity.clone();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            secure::server_handshake(stream, server_identity_for_task)
                .await
                .unwrap()
        });
        let stream = TcpStream::connect(addr).await.unwrap();
        let expected = server_identity.public_hex().to_string();
        let (mut client_read, mut client_write, peer) =
            secure::client_handshake(stream, client_identity.clone(), Some(&expected))
                .await
                .unwrap();
        assert_eq!(peer, expected);
        let (mut server_read, mut server_write, peer) = server.await.unwrap();
        assert_eq!(peer, client_identity.public_hex());

        let wire = build_wire(&client_identity, "presence", json!({"state": "online"}));
        client_write.send(&wire).await.unwrap();
        let received = server_read.receive().await.unwrap();
        assert_eq!(received.body["state"], "online");
        assert!(verify_wire(&received).is_ok());

        let reply = build_wire(&server_identity, "pong", json!({}));
        server_write.send(&reply).await.unwrap();
        assert_eq!(client_read.receive().await.unwrap().kind, "pong");
    }

    #[test]
    fn service_accepts_authenticated_friend_request() {
        let _env = friends::TEST_CONFIG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = std::env::temp_dir().join(format!("deepslate-friendnet-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        friends::set_test_config_dir(Some(root.clone()));
        let _cleanup = TestRoot(root.clone());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(async {
            let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            let mut config = AppConfig::default();
            config.friend_port = port;
            config.friend_advertise = format!("127.0.0.1:{port}");
            let alice = Uuid::new_v4().to_string();
            friends::apply_action_for(
                &alice,
                crate::friends::FriendAction::Settings {
                    enabled: true,
                    allow_requests: true,
                },
            )
            .unwrap();
            sync_service(&alice, &config).await.unwrap();
            let snapshot = friends::load_for(&alice).unwrap();
            assert!(snapshot.network.running);
            assert_eq!(snapshot.network.port, port);

            let alice_identity = friends::load_or_create_identity(&alice).unwrap();
            let bob_identity = Arc::new(FriendIdentity::generate_for_test());
            let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let (mut read, mut write, peer) = secure::client_handshake(
                stream,
                bob_identity.clone(),
                Some(alice_identity.public_hex()),
            )
            .await
            .unwrap();
            assert_eq!(peer, alice_identity.public_hex());
            let bob_profile = Uuid::new_v4().to_string();
            let wire = build_wire(
                &bob_identity,
                "friend_request",
                json!({
                    "request_id": Uuid::new_v4().to_string(),
                    "to": alice_identity.public_hex(),
                    "profile_id": bob_profile,
                    "name": "Bob",
                    "revision": 1,
                    "endpoint": "127.0.0.1:1",
                    "created_at": friends::now_millis(),
                }),
            );
            write.send(&wire).await.unwrap();
            let ack = read_ack_for(&mut read, &wire.nonce).await;
            assert_eq!(ack.body["status"], "ok");

            let snapshot = friends::load_for(&alice).unwrap();
            let incoming = snapshot
                .incoming_requests
                .iter()
                .find(|entry| entry.key.as_deref() == Some(bob_identity.public_hex()))
                .expect("incoming request");
            let incoming_id = incoming.profile_id.clone();
            friends::apply_action_for(
                &alice,
                crate::friends::FriendAction::Accept {
                    profile_id: incoming_id,
                },
            )
            .unwrap();
            let accept = read_kind(&mut read, "friend_accept").await;
            assert_eq!(
                accept.body.get("request_id").and_then(Value::as_str),
                incoming.request_id.as_deref()
            );

            let presence = build_wire(
                &bob_identity,
                "presence",
                json!({
                    "profile_id": bob_profile,
                    "name": "Bob",
                    "state": "playing_server",
                    "endpoint": "127.0.0.1:1",
                    "updated_at": friends::now_millis(),
                }),
            );
            write.send(&presence).await.unwrap();
            // Presence is intentionally not acknowledged; wait for the store.
            let mut applied = false;
            for _ in 0..40 {
                if friends::load_for(&alice)
                    .map(|snapshot| {
                        snapshot.friends.iter().any(|entry| {
                            entry.key.as_deref() == Some(bob_identity.public_hex())
                                && entry.presence.as_deref() == Some("playing_server")
                        })
                    })
                    .unwrap_or(false)
                {
                    applied = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(applied, "presence was not applied");
            let snapshot = friends::load_for(&alice).unwrap();
            let friend = snapshot
                .friends
                .iter()
                .find(|entry| entry.key.as_deref() == Some(bob_identity.public_hex()))
                .expect("accepted friend");
            assert_eq!(friend.presence.as_deref(), Some("playing_server"));

            let remove = build_wire(
                &bob_identity,
                "friend_remove",
                json!({ "revision": friend.revision.saturating_add(1) }),
            );
            write.send(&remove).await.unwrap();
            let remove_ack = read_ack_for(&mut read, &remove.nonce).await;
            assert_eq!(remove_ack.body["status"], "ok");
            assert!(friends::load_for(&alice).unwrap().friends.is_empty());
            stop_service().await;
            Ok::<(), String>(())
        });
        result.unwrap();
    }

    #[tokio::test]
    async fn handshake_rejects_wrong_expected_identity() {
        let server_identity = Arc::new(FriendIdentity::generate_for_test());
        let client_identity = Arc::new(FriendIdentity::generate_for_test());
        let server_identity_for_task = server_identity.clone();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = secure::server_handshake(stream, server_identity_for_task).await;
        });
        let stream = TcpStream::connect(addr).await.unwrap();
        let result =
            secure::client_handshake(stream, client_identity, Some(&"00".repeat(32))).await;
        assert!(result.is_err());
        let _ = server.await;
    }
}
