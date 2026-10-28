use serde::Serialize;

use crate::auth::LoginProgress;
use crate::download::DownloadProgress;

/// 核心层向外上报的事件。
///
/// GUI 层（iced）通过事件总线以 Subscription 消费：下载进度、认证步骤、
/// 游戏/服务端进程状态与日志行。
#[derive(Debug, Clone)]
pub enum CoreEvent {
    DownloadProgress(DownloadProgress),
    AuthProgress(LoginProgress),
    GameState(GameState),
    /// 与进程状态分开的受管世界会话状态。
    BridgeState {
        state: String,
        message: Option<String>,
    },
    /// Snapshot read from Minecraft 26.3's PlayerSocialManager/FriendsService.
    FriendsSnapshot {
        profile_id: String,
        snapshot: crate::friends::FriendSnapshot,
    },
    /// 游戏进程的一行输出（stdout/stderr 已捕获为管道）
    GameLog {
        is_stderr: bool,
        line: String,
    },
    /// 本地服务器进程的一行输出
    ServerLog {
        is_stderr: bool,
        line: String,
    },
    ServerState(ServerState),
}

/// 游戏进程状态，序列化形状与旧版 `game-state` 事件载荷完全一致。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum GameState {
    Launching { version: String },
    Running { pid: u32 },
    Stopped { exit_code: i32 },
}

/// 本地服务端状态。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ServerState {
    Starting { version: String },
    Running { pid: u32 },
    Ready { port: u16 },
    Stopped { exit_code: i32 },
}
