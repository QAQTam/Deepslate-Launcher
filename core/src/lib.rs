//! 深板岩启动器（Deepslate Launcher）业务核心（框架无关）。
//!
//! 不依赖任何 GUI 框架：长任务通过 `tokio::sync::mpsc` channel 上报
//! [`CoreEvent`]，由 GUI 层（iced）自行消费展示。

pub mod auth;
pub mod backup;
pub mod branding;
pub mod bridge;
pub mod config;
pub mod download;
pub mod enhanced;
pub mod eula;
pub mod events;
pub mod fabric;
pub mod friendnet;
pub mod friends;
pub mod game_options;
pub mod launch;
pub mod maps;
pub mod migration;
pub mod mods;
pub mod nbt;
pub mod net;
pub mod profiles;
pub mod runtime;
pub mod saves;
pub mod server;
pub mod server_ping;
pub mod versions;
pub mod worldgen;

pub use auth::AuthMethod;
pub use config::AppConfig;
pub use events::{CoreEvent, GameState, ServerState};
pub use profiles::{ProfileStore, UserProfile};
