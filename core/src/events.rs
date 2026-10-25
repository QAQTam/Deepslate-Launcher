use serde::Serialize;

use crate::auth::LoginProgress;
use crate::download::DownloadProgress;

/// 核心层向外上报的事件。
///
/// GUI 层（iced）通过事件总线以 Subscription 消费：下载进度、认证步骤、游戏进程状态、游戏日志行。
#[derive(Debug, Clone)]
pub enum CoreEvent {
    DownloadProgress(DownloadProgress),
    AuthProgress(LoginProgress),
    GameState(GameState),
    /// 游戏进程的一行输出（stdout/stderr 已捕获为管道）
    GameLog { is_stderr: bool, line: String },
}

/// 游戏进程状态，序列化形状与旧版 `game-state` 事件载荷完全一致。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum GameState {
    Launching { version: String },
    Running { pid: u32 },
    Stopped { exit_code: i32 },
}
