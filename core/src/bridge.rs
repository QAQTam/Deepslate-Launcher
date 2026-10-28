//! Authenticated local session transport for the managed Fabric client.
//! The listener is created before Java starts; credentials never appear in arguments.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

const PROTOCOL: u64 = 2;
const MAX_MESSAGE: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WorldGameRules {
    pub keep_inventory: bool,
    pub mob_griefing: bool,
    pub daylight_cycle: bool,
    pub weather_cycle: bool,
    pub hostile_mobs: bool,
    pub pvp: bool,
}

impl Default for WorldGameRules {
    fn default() -> Self {
        Self {
            keep_inventory: false,
            mob_griefing: true,
            daylight_cycle: true,
            weather_cycle: true,
            hostile_mobs: true,
            pvp: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ManagedTarget {
    Open(String),
    Create {
        id: String,
        name: String,
        seed: String,
        mode: String,
        difficulty: String,
        preset: String,
        cheats: bool,
        rules: WorldGameRules,
    },
    JoinServer(String),
}
impl ManagedTarget {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Open(id) | Self::Create { id, .. } => Some(id),
            Self::JoinServer(_) => None,
        }
    }
    fn command(&self) -> (&'static str, Value) {
        match self {
            Self::Open(id) => ("OpenWorld", json!({"map_id": id})),
            Self::Create {
                id,
                name,
                seed,
                mode,
                difficulty,
                preset,
                cheats,
                rules,
            } => (
                "CreateWorld",
                json!({
                    "map_id": id,
                    "name": name,
                    "seed": seed,
                    "mode": mode,
                    "difficulty": difficulty,
                    "preset": preset,
                    "cheats": cheats,
                    "rules": {
                        "keep_inventory": rules.keep_inventory,
                        "mob_griefing": rules.mob_griefing,
                        "daylight_cycle": rules.daylight_cycle,
                        "weather_cycle": rules.weather_cycle,
                        "hostile_mobs": rules.hostile_mobs,
                        "pvp": rules.pvp
                    }
                }),
            ),
            Self::JoinServer(address) => ("JoinServer", json!({"address": address})),
        }
    }
}

pub struct BridgeListener {
    listener: TcpListener,
    session_id: String,
    token: String,
    owner_profile_id: Option<String>,
    description: PathBuf,
}

pub struct BridgeConnection {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    session_id: String,
}

#[derive(Debug, Clone)]
pub enum BridgeCommand {
    Quit,
    SyncFriends {
        profile_id: String,
        snapshot: crate::friends::FriendSnapshot,
    },
}

impl BridgeListener {
    /// Credentials are owner-only. Platforms without a secure implementation fail closed.
    pub async fn bind(cache: &Path) -> Result<Self, String> {
        Self::bind_for_profile_inner(cache, None).await
    }

    /// Bind a session to the selected launcher profile so native friend snapshots
    /// can be cached under the same Minecraft account.
    pub async fn bind_for_profile(cache: &Path, profile_id: &str) -> Result<Self, String> {
        let profile_id = uuid::Uuid::parse_str(profile_id)
            .map_err(|_| "当前档案 UUID 无效，无法同步好友列表".to_string())?
            .hyphenated()
            .to_string();
        Self::bind_for_profile_inner(cache, Some(profile_id)).await
    }

    async fn bind_for_profile_inner(
        cache: &Path,
        owner_profile_id: Option<String>,
    ) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = cache;
            let _ = owner_profile_id;
            return Err("当前平台尚未实现受保护的 Bridge 会话文件".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .map_err(|e| e.to_string())?;
            let session_id = Uuid::new_v4().to_string();
            let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
            let dir = cache.join(format!("bridge-{session_id}"));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .map_err(|e| e.to_string())?;
            let description = dir.join("session.json");
            let result = (|| -> Result<(), String> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&description)
                    .map_err(|e| e.to_string())?;
                serde_json::to_writer(
                    &mut file,
                    &json!({"session_id": session_id, "token": token,
                    "port": listener.local_addr().map_err(|e| e.to_string())?.port(),
                    "profile_id": owner_profile_id}),
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })();
            if let Err(e) = result {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
            Ok(Self {
                listener,
                session_id,
                token,
                owner_profile_id,
                description,
            })
        }
    }

    pub fn description_path(&self) -> &Path {
        &self.description
    }

    /// Ignore unauthenticated connections until the overall handshake deadline.
    pub async fn accept(&self, timeout: Duration) -> Result<BridgeConnection, String> {
        tokio::time::timeout(timeout, async {
            loop {
                let (stream, _) = self.listener.accept().await.map_err(|e| e.to_string())?;
                if let Ok(Ok(mut connection)) =
                    tokio::time::timeout(Duration::from_secs(3), self.authenticate(stream)).await
                {
                    connection.send("Welcome", "", json!({})).await?;
                    let _ = std::fs::remove_file(&self.description);
                    return Ok(connection);
                }
            }
        })
        .await
        .map_err(|_| "等待 Bridge 握手超时".to_string())?
    }

    async fn authenticate(&self, stream: TcpStream) -> Result<BridgeConnection, String> {
        let (read, writer) = stream.into_split();
        let mut reader = BufReader::new(read);
        let hello = read_message(&mut reader).await?;
        if hello.get("type").and_then(Value::as_str) != Some("Hello")
            || hello.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL)
            || hello.get("session_id").and_then(Value::as_str) != Some(&self.session_id)
            || hello.get("token").and_then(Value::as_str) != Some(&self.token)
        {
            return Err("Bridge 握手无效".into());
        }
        Ok(BridgeConnection {
            reader,
            writer,
            session_id: self.session_id.clone(),
        })
    }
}

impl Drop for BridgeListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.description);
        if let Some(parent) = self.description.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

async fn read_message<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value, String> {
    // `take` bounds allocation even if the peer never sends a newline.
    let mut data = Vec::new();
    let n = reader
        .take((MAX_MESSAGE + 1) as u64)
        .read_until(b'\n', &mut data)
        .await
        .map_err(|e| e.to_string())?;
    if n == 0 || n > MAX_MESSAGE || data.last() != Some(&b'\n') {
        return Err("Bridge 消息过长或连接已断开".into());
    }
    serde_json::from_slice(&data).map_err(|e| format!("Bridge 消息无效: {e}"))
}

impl BridgeConnection {
    pub async fn send(&mut self, kind: &str, request_id: &str, extra: Value) -> Result<(), String> {
        let mut message = extra.as_object().cloned().ok_or("Bridge 参数必须为对象")?;
        message.insert("protocol".into(), json!(PROTOCOL));
        message.insert("session_id".into(), json!(self.session_id));
        message.insert("request_id".into(), json!(request_id));
        message.insert("type".into(), json!(kind));
        let mut bytes = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        if bytes.len() + 1 > MAX_MESSAGE {
            return Err("Bridge 消息过长".into());
        }
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn receive(&mut self) -> Result<Value, String> {
        let message = read_message(&mut self.reader).await?;
        if message.get("protocol").and_then(Value::as_u64) != Some(PROTOCOL)
            || message.get("session_id").and_then(Value::as_str) != Some(&self.session_id)
        {
            return Err("Bridge 消息会话或协议不匹配".into());
        }
        Ok(message)
    }
}

/// Drive one managed world request. The child process is waited on independently by launch.
pub async fn run_world_session(
    listener: BridgeListener,
    map_id: &str,
    events: &tokio::sync::mpsc::Sender<crate::events::CoreEvent>,
) -> Result<(), String> {
    let (_sender, receiver) = tokio::sync::mpsc::channel::<BridgeCommand>(1);
    run_world_session_with_commands(
        listener,
        &ManagedTarget::Open(map_id.into()),
        events,
        receiver,
        None,
    )
    .await
}

pub async fn run_world_session_with_commands(
    listener: BridgeListener,
    target: &ManagedTarget,
    events: &tokio::sync::mpsc::Sender<crate::events::CoreEvent>,
    mut commands: tokio::sync::mpsc::Receiver<BridgeCommand>,
    entered: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), String> {
    let emit = |state: &str| crate::events::CoreEvent::BridgeState {
        state: state.to_string(),
        message: None,
    };
    let owner_profile_id = listener.owner_profile_id.clone();
    let _ = events.send(emit("waiting_for_bridge")).await;
    let mut connection = listener.accept(Duration::from_secs(90)).await?;
    let _ = events.send(emit("connected")).await;
    let initial_snapshot = match owner_profile_id.as_deref() {
        Some(profile_id) => crate::friends::load_for(profile_id)?,
        None => crate::friends::FriendSnapshot::default(),
    };
    connection
        .send("SyncFriends", "", json!({"snapshot": initial_snapshot}))
        .await?;
    let request = Uuid::new_v4().to_string();
    let mut sent = false;
    loop {
        let event = tokio::select! {
            event = connection.receive() => event?,
            Some(command) = commands.recv() => {
                match command {
                    BridgeCommand::Quit => {
                        connection.send("QuitClient", &Uuid::new_v4().to_string(), json!({})).await?;
                    }
                    BridgeCommand::SyncFriends { profile_id, snapshot } => {
                        if owner_profile_id.as_deref() == Some(profile_id.as_str()) {
                            connection.send("SyncFriends", "", json!({"snapshot": snapshot})).await?;
                        }
                    }
                }
                continue;
            }
        };
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let event_request = event
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "FriendAction" => {
                let Some(profile_id) = owner_profile_id.as_ref() else {
                    continue;
                };
                let Some(value) = event.get("action").cloned() else {
                    continue;
                };
                let Ok(action) = serde_json::from_value::<crate::friends::FriendAction>(value)
                else {
                    continue;
                };
                match crate::friends::apply_action_for(profile_id, action) {
                    Ok(snapshot) => {
                        let _ = events
                            .send(crate::events::CoreEvent::FriendsSnapshot {
                                profile_id: profile_id.clone(),
                                snapshot: snapshot.clone(),
                            })
                            .await;
                        connection
                            .send("SyncFriends", "", json!({"snapshot": snapshot}))
                            .await?;
                    }
                    Err(message) => {
                        let _ = events
                            .send(crate::events::CoreEvent::BridgeState {
                                state: "friend_action_failed".into(),
                                message: Some(message),
                            })
                            .await;
                    }
                }
            }
            "Status" => {
                let request_matches = if sent {
                    event_request == request
                } else {
                    event_request.is_empty()
                };
                let phase = event.get("phase").and_then(Value::as_str).unwrap_or("");
                if !request_matches
                    || phase.is_empty()
                    || phase.len() > 64
                    || !phase
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'_' || b.is_ascii_digit())
                {
                    continue;
                }
                let progress = event
                    .get("progress")
                    .and_then(Value::as_u64)
                    .filter(|value| *value <= 100)
                    .map(|value| format!("{value}%"));
                let _ = events
                    .send(crate::events::CoreEvent::BridgeState {
                        state: format!("phase_{phase}"),
                        message: progress,
                    })
                    .await;
            }
            "Ready" if !sent => {
                let _ = events.send(emit("ready")).await;
                let (kind, args) = target.command();
                connection.send(kind, &request, args).await?;
                sent = true;
            }
            "WorldLoading" | "WorldEntered" | "WorldLeaving"
                if sent && event_request == request =>
            {
                let state = match kind {
                    "WorldLoading" => "loading",
                    "WorldEntered" => "entered",
                    _ => "saving",
                };
                if kind == "WorldEntered" {
                    if let Some(flag) = &entered {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                let _ = events.send(emit(state)).await;
            }
            "WorldClosed" if sent && event_request == request => {
                let _ = events.send(emit("closed")).await;
                return Ok(());
            }
            "OperationFailed" if sent && event_request == request => {
                return Err(event
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Bridge 操作失败")
                    .to_string());
            }
            _ => {}
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    #[test]
    fn server_target_does_not_resolve_to_a_local_map() {
        let target = ManagedTarget::JoinServer("127.0.0.1:25587".into());
        assert!(target.id().is_none());
        let (command, args) = target.command();
        assert_eq!(command, "JoinServer");
        assert_eq!(args["address"], "127.0.0.1:25587");
    }
    #[tokio::test]
    async fn handshake_rejects_intruder_and_exchanges_bounded_messages() {
        let cache = std::env::temp_dir().join(format!("bridge-test-{}", Uuid::new_v4()));
        let listener = BridgeListener::bind(&cache).await.unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(listener.description_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let info: Value =
            serde_json::from_slice(&std::fs::read(listener.description_path()).unwrap()).unwrap();
        let port = info["port"].as_u64().unwrap() as u16;
        let bad = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        drop(bad);
        let mut client = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        let hello = json!({"protocol":2,"type":"Hello","session_id":info["session_id"],"token":info["token"]});
        client
            .write_all(format!("{hello}\n").as_bytes())
            .await
            .unwrap();
        let mut connection = listener.accept(Duration::from_secs(2)).await.unwrap();
        assert!(!listener.description_path().exists());
        connection
            .send("OpenWorld", "req", json!({"map_id":"map-1"}))
            .await
            .unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["type"],
            "Welcome"
        );
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let command: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(command["type"], "OpenWorld");
        let response =
            json!({"protocol":2,"type":"Ready","session_id":info["session_id"],"request_id":""});
        reader
            .get_mut()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
        assert_eq!(connection.receive().await.unwrap()["type"], "Ready");
        drop(listener);
        std::fs::remove_dir_all(cache).unwrap();
    }
    #[tokio::test]
    async fn rejects_oversized_messages() {
        let bytes = vec![b'x'; MAX_MESSAGE + 1];
        assert!(read_message(&mut BufReader::new(bytes.as_slice()))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn managed_session_waits_for_ready_and_correlates_events() {
        let cache = std::env::temp_dir().join(format!("bridge-session-test-{}", Uuid::new_v4()));
        let listener = BridgeListener::bind(&cache).await.unwrap();
        let info: Value =
            serde_json::from_slice(&std::fs::read(listener.description_path()).unwrap()).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let task = tokio::spawn(async move { run_world_session(listener, "world-id", &tx).await });
        let mut client = TcpStream::connect((
            std::net::Ipv4Addr::LOCALHOST,
            info["port"].as_u64().unwrap() as u16,
        ))
        .await
        .unwrap();
        let hello = json!({"protocol":2,"type":"Hello","session_id":info["session_id"],"token":info["token"]});
        client
            .write_all(format!("{hello}\n").as_bytes())
            .await
            .unwrap();
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap(); // Welcome
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let initial_sync: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(initial_sync["type"], "SyncFriends");
        let ready =
            json!({"protocol":2,"type":"Ready","session_id":info["session_id"],"request_id":""});
        reader
            .get_mut()
            .write_all(format!("{ready}\n").as_bytes())
            .await
            .unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let command: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(command["type"], "OpenWorld");
        assert_eq!(command["map_id"], "world-id");
        for kind in [
            "WorldLoading",
            "WorldEntered",
            "WorldLeaving",
            "WorldClosed",
        ] {
            let event = json!({"protocol":2,"type":kind,"session_id":info["session_id"],"request_id":command["request_id"]});
            reader
                .get_mut()
                .write_all(format!("{event}\n").as_bytes())
                .await
                .unwrap();
        }
        task.await.unwrap().unwrap();
        let mut states = Vec::new();
        while let Some(crate::events::CoreEvent::BridgeState { state, .. }) = rx.recv().await {
            states.push(state);
        }
        assert_eq!(
            states,
            [
                "waiting_for_bridge",
                "connected",
                "ready",
                "loading",
                "entered",
                "saving",
                "closed"
            ]
        );
        std::fs::remove_dir_all(cache).unwrap();
    }
}
