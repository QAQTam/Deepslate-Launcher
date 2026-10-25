# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**深板岩启动器（Deepslate Launcher）** is a minimal Minecraft Java Edition launcher written entirely in Rust: an iced GUI on top of a framework-free core. Kept deliberately basic (download / launch / profiles / settings — no news, no mod loader). Visual identity: kraft-paper yellow × slate gray, light/dark themes (no purple, no green).

## Build Commands

```bash
cargo run -p gui-iced    # Run the launcher (binary: deepslate-iced)
cargo build              # Build the whole workspace
```

## Architecture

### Cargo Workspace

- **`core/`** (`deepslate-core`): framework-free business logic. No GUI dependencies; long-running tasks report progress via `tokio::sync::mpsc` channels carrying `CoreEvent` (defined in `core/src/events.rs`).
  - **`config.rs`**: AppConfig load/save, game/saves/versions directory resolution. `config.json` lives beside the executable.
  - **`profiles.rs`**: User profiles (offline + Microsoft), deterministic offline UUIDs.
  - **`auth.rs`**: Microsoft OAuth 2.0 flow (login.live.com → Xbox Live → XSTS → Minecraft Services). Authorization happens in the system browser; the user pastes the callback URL back, then `login_with_code` runs the token chain.
  - **`download.rs`**: Downloads versions/libraries/assets from Mojang's CDN with SHA1 verification and bounded concurrency.
  - **`launch.rs`**: Game process spawning (new `arguments` + legacy `minecraftArguments`, `inheritsFrom` merging, natives extraction), Java runtime scanning. stdout/stderr are piped and forwarded line-by-line as `CoreEvent::GameLog`. `game_dir` holds shared files (versions/libraries/assets); `instance_dir` is passed as `--gameDir` so saves/options/logs land in the version's instance dir when `version_isolation` is on.
  - **`backup.rs`**: Saves management. `version_saves_dir()` is the single source of truth for where a version's saves live (`root/saves/{v}` with isolation on, else the instance dir's `saves/`). Creating a symlink never deletes an existing real saves dir (contents are migrated); restore stages into a temp dir and swaps atomically, rolling back on failure. Backups via rsync (Linux) / xcopy (Windows), keeping the last 5.

- **`gui-iced/`** (bin `deepslate-iced`): iced 0.14 frontend, consumes `core` directly. Progress events flow through a static broadcast bus (`gui-iced/src/main.rs`: `EVENT_BUS` + `core_event_sender()`) into an iced `Subscription`. Styling lives in `gui-iced/src/style.rs` (dual light/dark `Scheme`s selected by `dark_mode`, persisted in `config.ui_mode`). Page/card entrance animations use iced `Animation` + `Float` transforms (300ms page switch, 400ms boot), ticked by a 33ms `iced::time::every` subscription only while animating. The version pick_list reads a cached `filtered` list (recomputed by `refilter()` only when versions load or the tab changes) instead of cloning per frame — see the `switch_perf` tests in `main.rs` for the perf profile. Pages: Home, Launch (version tabs/pickers, offline profile creation, Microsoft login, Java scan, download progress, launch/kill), Settings (appearance incl. log-window toggle, root dir, isolation toggles, download concurrency, saves backup management, save/reset). Optional game-log window: when `config.log_window` is on, launching the game spawns a separate `deepslate-iced --log-window` child process; the main process pipes `GameLog` lines into its stdin (`O\t`/`E\t` prefixed). The child renders them (stderr in red, newest on top, 2000-line auto-compacting buffer) and auto-closes a few seconds after stdin closes.

Note: data paths still default to the historical `~/Games/ObsidianLauncher` and legacy `~/.config/obsidian-launcher` fallback so existing game files keep resolving.

### Directory Layout

Shared files live under `root_dir/game/` (`version_isolation=false`) or `root_dir/` (`version_isolation=true`): `versions/`, `libraries/`, `assets/`.

The **instance dir** (`--gameDir`) is `root_dir/game` when `version_isolation=false`, or `root_dir/versions/{id}` when true — saves, `options.txt` and logs land there. With `saves_isolation` on, the instance dir's `saves` is a symlink to `root_dir/saves/{version}/` (backups of that dir go to `root_dir/saves/{version}_backups/`).
