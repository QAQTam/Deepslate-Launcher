//! 深板岩启动器（Deepslate Launcher）业务核心（框架无关）。
//!
//! 不依赖任何 GUI 框架：长任务通过 `tokio::sync::mpsc` channel 上报
//! [`CoreEvent`]，由 GUI 层（iced）自行消费展示。

pub mod auth;
pub mod backup;
pub mod config;
pub mod download;
pub mod events;
pub mod launch;
pub mod profiles;

pub use auth::AuthMethod;
pub use config::AppConfig;
pub use events::{CoreEvent, GameState};
pub use profiles::{ProfileStore, UserProfile};
