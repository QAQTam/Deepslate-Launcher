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
  - **`launch.rs`**: Game process spawning (new `arguments` + legacy `minecraftArguments`, `inheritsFrom` merging, natives extraction), Java runtime scanning. `game_dir` holds shared files (versions/libraries/assets); `instance_dir` is passed as `--gameDir` so saves/options/logs land in the version's instance dir when `version_isolation` is on.
  - **`backup.rs`**: Saves management. `version_saves_dir()` is the single source of truth for where a version's saves live (`root/saves/{v}` with isolation on, else the instance dir's `saves/`). Creating a symlink never deletes an existing real saves dir (contents are migrated); restore stages into a temp dir and swaps atomically, rolling back on failure. Backups via rsync (Linux) / xcopy (Windows), keeping the last 5.

- **`gui-iced/`** (bin `obsidian-iced`): iced 0.14 frontend, consumes `core` directly. Progress events flow through a static broadcast bus (`gui-iced/src/main.rs`: `EVENT_BUS` + `core_event_sender()`) into an iced `Subscription`. Pages: Home (status + quick actions), Launch (version tabs/pickers, offline profile creation, Microsoft login, Java scan, download progress, launch/kill), Settings (root dir, isolation toggles, download concurrency, saves backup management, save/reset).

### Directory Layout

Shared files live under `root_dir/game/` (`version_isolation=false`) or `root_dir/` (`version_isolation=true`): `versions/`, `libraries/`, `assets/`.

The **instance dir** (`--gameDir`) is `root_dir/game` when `version_isolation=false`, or `root_dir/versions/{id}` when true — saves, `options.txt` and logs land there. With `saves_isolation` on, the instance dir's `saves` is a symlink to `root_dir/saves/{version}/` (backups of that dir go to `root_dir/saves/{version}_backups/`).
