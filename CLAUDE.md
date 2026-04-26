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
```

## Architecture

### Tauri Backend (`src-tauri/src/`)

- **`lib.rs`**: Main entry point. Registers all Tauri commands and manages app lifecycle.
- **`auth.rs`**: Microsoft OAuth 2.0 authentication flow (login.live.com → Xbox Live → XSTS → Minecraft Services).
- **`download.rs`**: Downloads Minecraft versions, libraries, and assets from Mojang's CDN with SHA1 verification.
- **`launch.rs`**: Game process spawning, Java runtime detection, native library extraction from JARs.

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

Key commands exposed to frontend (defined in `lib.rs`):
- `download`, `launch`, `kill_game` - Game lifecycle
- `scan_java`, `scan_java_dir` - Java runtime detection
- `get_versions`, `get_profiles`, `get_config` - Data loading
- `create_profile`, `delete_profile`, `login_microsoft` - Profile management
- `create_saves_backup`, `restore_saves_backup`, `list_saves_backups` - Save management
- `set_config`, `reset_all_config` - Configuration

### Event Emitters

Backend emits events frontend listens for:
- `download-progress` - Download progress updates
- `game-state` - Game launching/running/stopped states
- `auth-progress` - Microsoft login step updates
