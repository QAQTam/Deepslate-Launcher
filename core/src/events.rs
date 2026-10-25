use serde::Serialize;

use crate::auth::LoginProgress;
use crate::download::DownloadProgress;

/// 核心层向外上报的事件。
///
/// Tauri 壳将其桥接为同名前端事件（`download-progress` / `auth-progress` / `game-state`）；
/// iced/Slint 壳用 Subscription / 定时轮询消费同一 channel。
#[derive(Debug, Clone)]
pub enum CoreEvent {
    DownloadProgress(DownloadProgress),
    AuthProgress(LoginProgress),
    GameState(GameState),
}

/// 游戏进程状态，序列化形状与旧版 `game-state` 事件载荷完全一致。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum GameState {
    Launching { version: String },
    Running { pid: u32 },
    Stopped { exit_code: i32 },
}
