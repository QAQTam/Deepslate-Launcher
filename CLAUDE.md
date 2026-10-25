# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Obsidian Launcher** is a minimal Minecraft Java Edition launcher written entirely in Rust: an iced GUI on top of a framework-free core. Kept deliberately basic (download / launch / profiles / settings — no news, no mod loader).

## Build Commands

```bash
cargo run -p gui-iced    # Run the launcher (binary: obsidian-iced)
cargo build              # Build the whole workspace
```

## Architecture

### Cargo Workspace

- **`core/`** (`obsidian-launcher-core`): framework-free business logic. No GUI dependencies; long-running tasks report progress via `tokio::sync::mpsc` channels carrying `CoreEvent` (defined in `core/src/events.rs`).
  - **`config.rs`**: AppConfig load/save, game/saves/versions directory resolution. `config.json` lives beside the executable.
  - **`profiles.rs`**: User profiles (offline + Microsoft), deterministic offline UUIDs.
  - **`auth.rs`**: Microsoft OAuth 2.0 flow (login.live.com → Xbox Live → XSTS → Minecraft Services). Authorization happens in the system browser; the user pastes the callback URL back, then `login_with_code` runs the token chain.
  - **`download.rs`**: Downloads versions/libraries/assets from Mojang's CDN with SHA1 verification and bounded concurrency.
  - **`launch.rs`**: Game process spawning (new `arguments` + legacy `minecraftArguments`, `inheritsFrom` merging, natives extraction), Java runtime scanning.
  - **`backup.rs`**: Saves symlink isolation + backup/restore via rsync (Linux) / xcopy (Windows), keeping the last 5 backups.

- **`gui-iced/`** (bin `obsidian-iced`): iced 0.14 frontend, consumes `core` directly. Progress events flow through a static broadcast bus (`gui-iced/src/main.rs`: `EVENT_BUS` + `core_event_sender()`) into an iced `Subscription`. Pages: Home (status + quick actions), Launch (version tabs/pickers, offline profile creation, Microsoft login, Java scan, download progress, launch/kill), Settings (root dir, isolation toggles, download concurrency, save/reset).

### Directory Layout

Game directory structure depends on `version_isolation` in the config:
- **false**: `root_dir/game/` holds `versions/`, `libraries/`, `assets/`
- **true**: `root_dir/versions/{id}/` is each version's full root

Saves resolve under `root_dir/saves/`; with `saves_isolation` enabled, each version's `saves` is a symlink to `root_dir/saves/{version}/`.
