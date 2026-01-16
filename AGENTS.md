# REDSEAT-RUST KNOWLEDGE BASE

**Generated:** 2026-01-07  
**Commit:** 76e5e3c  
**Branch:** master

## OVERVIEW

Media server with face recognition, WASM plugins, and encrypted library support. Rust/Axum/SQLite/ONNX stack. Daemon-supervised architecture with auto-restart on crash.

## STRUCTURE

```
redseat-rust/
├── src/
│   ├── main.rs           # Axum server entry, router composition
│   ├── daemon/main.rs    # Watchdog process (restarts on exit 101/201)
│   ├── server.rs         # Config (JSON/env/CLI hierarchy)
│   ├── model/            # Business logic, ModelController hub
│   ├── routes/           # Axum handlers, REST + Socket.IO
│   ├── domain/           # DTOs, camelCase serialization
│   ├── plugins/          # WASM (Extism) + internal providers
│   └── tools/            # Recognition, video, encryption, scheduler
├── test_data/            # Sample images for tests
├── .github/workflows/    # CI: multi-platform builds + Docker
└── Cargo.toml            # vcpkg integration for native libs
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Add API endpoint | `src/routes/` | Nest under existing entity router |
| Modify media processing | `src/model/medias.rs` | 1609 lines, spawns background tasks |
| Face recognition logic | `src/model/people.rs` + `src/tools/recognition.rs` | ONNX inference, clustering |
| Database schema | `src/model/store/sql/library/*.sql` | Numbered migrations, triggers |
| Plugin development | `src/plugins/` | WASM via Extism, `Source` trait |
| External tool wrappers | `src/tools/video_tools.rs` | FFmpeg builder pattern |
| Config/env variables | `src/server.rs` | `REDSEAT_*` env vars |
| Encryption | `src/tools/encryption.rs` | AES-256-CBC streaming |
| Scheduled tasks | `src/tools/scheduler/` | 15s tick loop |
| SSE and events | `docs/SSE.md` | don't forget to update the SSE.md file if you do modification or add events |

## ARCHITECTURE

### Binary Targets
- `redseat-rust`: Main server (Axum, port 8080)
- `redseat-daemon`: Watchdog that supervises main server
  - Exit 101 (panic) → restart with retry limit
  - Exit 201 → controlled restart (for updates)

### Core Components
- **ModelController** (`src/model/mod.rs`): Central state, injected via Axum State
- **SqliteStore**: Multi-DB (main `database.db` + per-library `db-{id}.db`)
- **PluginManager**: WASM plugins via Extism + internal metadata providers
- **RsScheduler**: Background tasks (backup, face scan, metadata refresh)

### Data Flow
```
Request → mw_auth (token resolution) → Handler → ModelController → SqliteStore
                                                      ↓
                                              PluginManager/Source
```

## CONVENTIONS

### Rust Patterns
- **Error handling**: Central `Error` enum in `src/error.rs`, implements `IntoResponse`
- **Async traits**: `#[async_trait]` for `Source` and other interfaces
- **Module organization**: No `lib.rs`, all modules under `main.rs`

### Serialization
- All DTOs use `#[serde(rename_all = "camelCase")]`
- `#[serde(skip_serializing_if = "Option::is_none")]` for sparse JSON
- Field `kind` renamed to `type` in JSON output

### Database
- Migrations: `include_bytes!` embedded, tracked via `user_version` pragma
- Triggers manage `modified`/`added` timestamps automatically
- Lists stored as pipe-separated strings (e.g., `alt` names)

### API
- RESTful with nested routes: `/libraries/:libraryid/medias/:id`
- Auth via `Authorization: Bearer`, `SHARETOKEN` header, or `token` query param
- Range requests supported for streaming

## ANTI-PATTERNS (DO NOT)

| Pattern | Why |
|---------|-----|
| `as any`, `@ts-ignore` | N/A (Rust) but avoid `.unwrap()` in spawned tasks |
| Empty catch blocks | Use `?` operator, log errors via `log_error()` |
| Suppress type errors | Never use turbofish to force wrong types |
| Commit secrets | Trakt client ID hardcoded in `src/model/mod.rs:153` - externalize |
| `std::sync::Mutex` across `.await` | Use `tokio::sync::Mutex` for async code |
| Panic in background tasks | Always handle errors, don't crash silently |

## GOTCHAS

1. **177 `.unwrap()` calls** - Many in background tasks, can cause silent failures
2. **libheif unsafe code** - `src/tools/convert/heic.rs` has manual memory management
3. **Runtime binary downloads** - FFmpeg/ONNX models downloaded on first run
4. **Global locks** - `FFMPEG_LOCK` can stall parallel transcoding
5. **No `lib.rs`** - Code sharing between binaries requires re-declaration
6. **vcpkg required** - Run `cargo vcpkg build` before `cargo build` on dev machines

## EXTERNAL DEPENDENCIES

| Tool | Purpose | Auto-download |
|------|---------|---------------|
| FFmpeg/FFprobe | Video processing | Yes (platform-specific) |
| YT-DLP | Remote video streaming | Yes |
| ONNX models | Face recognition (Buffalo_L) | Yes (Hugging Face) |
| ImageMagick | Image conversion (optional) | No, manual install |

## COMMANDS

```bash
# Development
cargo vcpkg build                    # One-time: fetch native deps
cargo watch -c -w src -x "run --bin redseat-rust"

# Build
cargo build --release

# Test
cargo test                           # Some tests need ffmpeg/yt-dlp

# Docker
docker pull neckaros/redseat-rust
docker run -v redseat_config:/root/.config/redseat -p 8080:8080 neckaros/redseat-rust
```

## ENV VARIABLES

| Variable | Purpose |
|----------|---------|
| `REDSEAT_SERVERID` | Force server ID |
| `REDSEAT_HOME` | Override cloud server URL |
| `REDSEAT_PORT` | Server port (default: 8080) |
| `REDSEAT_DIR` | Config directory |
| `REDSEAT_DOMAIN` | Custom domain (disables IP-based) |
| `REDSEAT_NOCERT` | Skip TLS cert generation |

## SUBDIRECTORY AGENTS

- `src/model/AGENTS.md` - Face recognition, media lifecycle, store patterns
- `src/tools/AGENTS.md` - Recognition engine, video tools, encryption
