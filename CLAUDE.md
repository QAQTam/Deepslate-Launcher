# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Obsidian Launcher** is a Minecraft Java Edition launcher built with Tauri v2 (Rust backend + React/TypeScript frontend).

## Build Commands

```bash
pnpm dev        # Run Tauri dev mode (frontend + backend)
pnpm build      # Build production Tauri app
pnpm --filter frontend run dev    # Frontend only (Vite dev server)
pnpm --filter frontend run build  # Frontend only build
cargo run -p gui-iced             # Run the iced frontend
cargo build                       # Build the whole Rust workspace
```

## Architecture

### Cargo Workspace

- **`core/`** (`obsidian-launcher-core`): framework-free business logic. No Tauri/GUI dependencies; long-running tasks report progress via `tokio::sync::mpsc` channels carrying `CoreEvent` (defined in `core/src/events.rs`). A future iced/Slint GUI consumes the same channels.
  - **`config.rs`**: AppConfig load/save, game/saves/versions directory resolution.
  - **`profiles.rs`**: User profiles (offline + Microsoft), deterministic offline UUIDs.
  - **`auth.rs`**: Microsoft OAuth 2.0 flow (login.live.com → Xbox Live → XSTS → Minecraft Services). Authorization happens in the system browser; the user pastes the callback URL back (`login_with_code`).
  - **`download.rs`**: Downloads versions/libraries/assets from Mojang's CDN with SHA1 verification and bounded concurrency.
  - **`launch.rs`**: Game process spawning (new `arguments` + legacy `minecraftArguments`, `inheritsFrom` merging, natives extraction), Java runtime scanning, GhastLoader javaagent injection.
  - **`backup.rs`**: Saves symlink isolation, backup/restore via rsync/xcopy (Linux/Windows).

- **`src-tauri/`**: thin Tauri shell. `src-tauri/src/lib.rs` only bridges Tauri commands to `core` and forwards `CoreEvent` channels to frontend events (`download-progress`, `game-state`, `auth-progress`). All business logic lives in `core/`.

- **`gui-iced/`** (bin `obsidian-iced`): iced 0.14 frontend, consumes `core` directly. Progress events flow through a static broadcast bus (`gui-iced/src/main.rs`: `EVENT_BUS` + `core_event_sender()`) into an iced `Subscription`. Pages: Home (status overview + quick actions), Launch (version tabs/pickers, offline profile creation, Microsoft login via browser + pasted callback URL, Java scan, download progress, launch/kill), Settings (root dir, isolation toggles, download concurrency, save/reset). Run with `cargo run -p gui-iced`.

### Linux build prerequisites

Building the Tauri shell requires system packages: `libglib2.0-dev libgtk-3-dev libwebkit2gtk-4.1-dev librsvg2-dev libxdo-dev libssl-dev libayatana-appindicator3-dev`.

### State Management

Config and profiles use JSON files:
- `config.json` stored beside the executable (v2+ also mirrors to `root_dir/config/`)
- `profiles.json` stored in `root_dir/config/` (v2) or `~/.config/obsidian-launcher/` (legacy)

### Frontend (`frontend/src/`)

- **`hooks/useAppState.tsx`**: React Context providing centralized app state and actions.
- **`hooks/useTauriInvoke.ts`**: Wrapper for `tauri.invoke` with type safety.
- **`hooks/useDownloadProgress.ts`**: Subscribes to `download-progress` Tauri events.
- **`hooks/useGameState.ts`**: Subscribes to `game-state` Tauri events.

### Directory Layout

Game directory structure depends on `version_isolation` config:
- **isolated=false**: `root_dir/game/versions/`, `root_dir/game/libraries/`, `root_dir/game/assets/`
- **isolated=true**: `root_dir/versions/`, `root_dir/libraries/`, `root_dir/assets/`

Saves are always at `root_dir/saves/` (symlinked per-version when `saves_isolation=true`).

### Tauri Commands

Key commands exposed to frontend (thin wrappers in `src-tauri/src/lib.rs`, implemented in `core/`):
- `download`, `launch`, `kill_game` - Game lifecycle
- `scan_java`, `scan_java_dir` - Java runtime detection
- `get_versions`, `get_profiles`, `get_config` - Data loading
- `create_profile`, `delete_profile`, `login_microsoft_with_code` - Profile management (`login_microsoft` always errors: webview cannot capture the OAuth callback; use the browser + manual code paste flow)
- `create_saves_backup`, `restore_saves_backup`, `list_saves_backups` - Save management
- `set_config`, `reset_all_config` - Configuration

### Event Emitters

Backend emits events frontend listens for:
- `download-progress` - Download progress updates
- `game-state` - Game launching/running/stopped states
- `auth-progress` - Microsoft login step updates
