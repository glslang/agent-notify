# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Rust workspace plus a Node CLI that pushes coding-agent status (e.g. Codex/Claude Code hook events) to a small HTTP/WebSocket server, which fans them out to a Windows tray bridge that drives the UHK80 keyboard's OLED via HID.

Topology: agents → `agent-notify-cli` (Rust or npm) → `agent-notify-server` (anywhere, usually Linux) → `agent-notify-bridge` (Windows tray, holds the UHK80 over USB/KVM).

## Commands

Rust workspace (run from repo root):

- `cargo check --workspace` — quick validate
- `cargo nextest run --workspace` — run tests (CI uses nextest; `cargo test --workspace` also works)
- `cargo nextest run -p agent-notify-core <test_name>` — single test
- `cargo fmt --all` and `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — matches CI
- `AGENT_NOTIFY_TOKEN=change-me cargo run -p agent-notify-server -- --bind 0.0.0.0:8787`
- `AGENT_NOTIFY_TOKEN=change-me cargo run -p agent-notify-bridge -- --mock-display` — bridge without UHK80 hardware
- `cargo run -p agent-notify-cli -- --state waiting-input --agent codex --repo agent-notify --summary "..."` — send a test event (requires `AGENT_NOTIFY_SERVER` + `AGENT_NOTIFY_TOKEN`)

TypeScript client (`clients/typescript/agent-notify-cli/`, Node ≥22):

- `npm run build` — `tsc`
- `npm test` — builds, then runs the three `dist/test/*.test.js` files via `node --test`
- `npm run test:coverage` — same, with `c8` lcov output
- `npm run typecheck`

## Architecture

Four Rust crates under `crates/` plus one Node package under `clients/typescript/agent-notify-cli/`.

- **`agent-notify-core`** is the *only* place protocol types live. `AgentEventInput`/`AgentEvent`, `BridgeClientMessage`/`BridgeServerMessage`, the `choose_latest` selection rule, and the UHK macro/HID byte formatters (`macro_command_for_event`, `uhk_exec_macro_report`, `concise_display_text`) are all here so the CLI, server, and bridge agree on the wire format. Keep new shared protocol/types in this crate; binary-specific transport/parsing/platform code goes in the relevant application crate.
- **`agent-notify-server`** is an axum app holding `Arc<Mutex<Option<AgentEvent>>>` — one in-memory latest event — plus a `broadcast::Sender<Arc<BridgeServerMessage>>` that fans events to connected bridges (payload is `Arc`-wrapped so each subscriber clone is cheap). Auth is a single bearer token (`AGENT_NOTIFY_TOKEN`), compared constant-time against a precomputed `Bearer <token>` string; the WebSocket also accepts the token as `?token=` query param. The `TraceLayer` span logs only method + path so the WS token never reaches logs. A `RequestBodyLimitLayer` and `GlobalConcurrencyLimitLayer` bound memory and connection count. Endpoints: `GET /healthz`, `POST /v1/events`, `GET /v1/events/latest`, `DELETE /v1/events/latest`, `GET /v1/bridge/ws`. Error responses carry `{ code, error }`. There is no persistence and no multi-instance coordination — this is intentional, designed for a single small server.
- **`agent-notify-cli`** and the Node `agent-notify-cli` are deliberately parallel HTTP senders implementing the same flag surface (`--state`, `--agent`, `--host`, `--repo`, `--summary`, `--priority`, `--ttl-seconds`, `--run-id`, `--dismiss`). When changing the wire shape or flag surface, update **both** clients so hook configs work with either binary.
- **`agent-notify-bridge`** is Windows-only by design for the HID path, split into `main.rs` (CLI + entry), `tray.rs` (Windows tray, `#[cfg(windows)]`), `worker.rs` (tokio worker, session loop, reconnect/backoff), `url.rs` (WS URL build + token redaction), `settings.rs`, and `uhk.rs`. On Windows it runs a `tray-icon` system tray on a `winit` event loop; menu actions cross into the worker tokio runtime via `mpsc::UnboundedSender<BridgeCommand>`. On non-Windows it runs a console worker with Ctrl-C wired to `Quit`. The worker reconnects with exponential backoff + jitter (capped at 30s), sends a `Status` heartbeat every 5s, and on each `Event` builds a UHK macro string (`macro_command_for_event`) and writes it as a HID report through `hidapi` to vendor `0x37a8`/product `0x0009`. The tray Test action routes a synthetic `AgentEvent` through the same `macro_command_for_event` path. When `--mock-display` is set or on non-Windows, `DisplayAdapter` just logs the macro string. The bridge config (`%APPDATA%\agent-notify\bridge.toml` on Windows, plus `ProjectDirs` fallback) is `BridgeConfig { server_url, token, hostname?, mock_display }`; CLI flags override file values; the file is not auto-created.

### Event semantics (in `agent_notify_core::choose_latest`)

- Server stores **one** latest event. A new event replaces the current one when it's the same work item: `run_id` matches if present, otherwise `(agent, host, repo)` all match.
- Otherwise the live event with higher `(priority, seq)` wins, where `seq` is a monotonic counter the server assigns on accept (clock-independent tiebreak).
- Default priorities: `WaitingInput`=90, `Failed`=80, `Done`=50, `Running`=20.
- TTL defaults to 120s, clamped to `[1, 3600]`. Expired latest is dropped on read.
- When the bridge reports `paused: true` over WebSocket, the server still accepts and stores events but the bridge silently drops broadcast deliveries — so the keyboard intentionally keeps showing whatever it last had until unpause/reconnect/dismiss.

### UHK macro/HID details (in `agent-notify-core`)

UHK report payload is capped at `UHK_MAX_USB_PAYLOAD_SIZE - 2 = 61` bytes. The macro-building helpers escape `"`/`\` and truncate to fit, so if you add new event-derived display fields keep the budget in mind (`concise_display_text` and `quoted_macro_command` are where this happens). `Test`-action menu item in the bridge bypasses the helpers and writes a literal `notify "agent notify test"` command.

### Platform notes

The bridge tray uses Win32 backends via `tray-icon` + `winit` — **no GTK on this path**. `tray-icon`/`muda` upstream still ship GTK *3* helpers for Linux/BSD only, which would pull a vulnerable `glib`, so GTK features stay disabled in `Cargo.toml`. If a Unix UI ever becomes necessary, wire **gtk4-rs**, not GTK3.

## Conventions (see AGENTS.md for the full list)

- Rust 2024 edition, standard `rustfmt`, `snake_case` for modules/functions/CLI flags, `PascalCase` for types/enum variants.
- Typed errors (`thiserror`) in shared/core code; `anyhow` for application-level context in binaries.
- Tests live beside the module they exercise, or in a crate `tests/` for cross-crate integration.
- Commit subjects are short imperatives (e.g. `Implement Rust agent notify prototype`).
- Never commit real `AGENT_NOTIFY_TOKEN` values or local `bridge.toml`. For Dependabot PRs to upload Codecov, add `CODECOV_TOKEN` under Settings → Secrets and variables → **Dependabot** (Actions secrets are not exposed to `dependabot[bot]`).
