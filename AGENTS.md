# Redseat Rust

Media server built with Rust, Axum, SQLite, ONNX, and Extism WASM plugins.
`redseat-rust` runs the server; `redseat-daemon` supervises it. Modules are
declared by the binary entry points; there is no `lib.rs`.

## Working approach

- Carry requested work through implementation and relevant verification. Use
  context and existing patterns for routine decisions; ask only when missing
  information materially affects the outcome or authorization is needed.
- Keep changes focused on the request. Treat these files as project guidance,
  not a backlog of unrelated fixes; explicit user instructions take precedence.
- Report the result, validation, and any remaining blocker concisely.

## Where to work

| Area | Entry points |
| --- | --- |
| Server, configuration, daemon | `src/main.rs`, `src/server.rs`, `src/daemon/main.rs` |
| HTTP routes and authentication | `src/routes/`, `src/routes/mw_auth.rs` |
| Business logic and persistence | `src/model/` — see `src/model/AGENTS.md` |
| API data types | `src/domain/` |
| Plugins and storage sources | `src/plugins/`, `src/plugins/sources/` |
| Media utilities and scheduler | `src/tools/` — see `src/tools/AGENTS.md` |
| Event contract | `docs/SSE.md` |
| Native dependencies and release builds | `Cargo.toml`, `.github/workflows/builder.yml`, `Dockerfile` |

## Project constraints

- Follow the route → `ModelController` → store/plugin structure and existing
  nested library routes. Preserve authorization checks and streaming range support.
- Preserve API serialization conventions: camelCase DTOs, sparse optional fields,
  and existing `kind` → `type` renames.
- Update `docs/SSE.md` when adding, changing, or removing events or their payloads.
- Use existing error types and propagate errors with `?`; log failures in detached
  tasks with the project logging helpers. Avoid panic-based error handling there.
- Do not hold synchronous lock guards across `.await`. Keep async lock lifetimes
  limited to the resource they protect.
- Preserve the daemon exit-code contract: 101 triggers crash recovery; 201 requests
  a controlled restart.

## Validation

- For Rust changes, start with `cargo check --bin redseat-rust`, then run the
  narrowest relevant `cargo test --bin redseat-rust <filter>`. Check the daemon
  target when changing its code or shared modules.
- Match verification to the change. Documentation-only edits need content/path
  review and `git diff --check`; run broader tests when affected behavior or
  failures justify them. Add tests for meaningful behavior, not to mirror edits.
- Native setup is platform-specific: CI uses `cargo vcpkg build` on Windows and
  system packages on Linux/macOS. Consult the build files above before installing
  dependencies.
- Cold Windows builds need at least 5 minutes for an initial check and 6 minutes
  for the first filtered test. Check and test compilation have separate caches.
  After a timeout, inspect surviving compiler processes before retrying so a
  second command does not just wait on the build lock.
- Some media tests require FFmpeg/FFprobe, YT-DLP, or ONNX models; first use may
  download them. Report unavailable prerequisites and which checks could not run.
