# Repository Guidelines

## Project Structure & Module Organization

This is a Rust workspace with member crates under `crates/`.

- `crates/agent-notify-core`: shared event types and domain errors.
- `crates/agent-notify-server`: HTTP/WebSocket event collector.
- `crates/agent-notify-cli`: command-line sender for hooks and local testing.
- `crates/agent-notify-bridge`: Windows tray bridge that connects to the server and updates the UHK80 display.

Keep cross-crate protocol types in `agent-notify-core`. Put binary-specific parsing, transport, and platform code in the relevant application crate. Tests should live beside the module they exercise or in a crate-level `tests/` directory when integration scope is needed.

## Build, Test, and Development Commands

- `cargo check --workspace`: quickly validate the full workspace.
- `cargo nextest run --workspace` (or `cargo test --workspace`): run all tests locally; CI uses [nextest](https://nexte.st/).
- `cargo fmt --all`: format every crate with `rustfmt`.
- `cargo clippy --workspace --all-targets`: run lints across binaries, libraries, and tests.
- `AGENT_NOTIFY_TOKEN=change-me cargo run -p agent-notify-server -- --bind 0.0.0.0:8787`: run the server.
- `cargo run -p agent-notify-cli -- --state waiting-input --agent codex --repo agent-notify --summary "waiting for input"`: send a test event; set `AGENT_NOTIFY_SERVER` and `AGENT_NOTIFY_TOKEN` first.
- `AGENT_NOTIFY_TOKEN=change-me cargo run -p agent-notify-bridge -- --mock-display`: run the bridge without UHK80 hardware.

## Coding Style & Naming Conventions

Use Rust 2024 edition conventions and standard `rustfmt` output. Prefer clear module boundaries over large `main.rs` files as behavior grows. Use `snake_case` for modules, functions, variables, and CLI flags; use `PascalCase` for types and enum variants. Keep errors typed in shared code where callers need to match them, and use `anyhow` for application-level context.

## Testing Guidelines

There are no explicit coverage gates yet. Add focused unit tests for parsing, serialization, and state transitions in the crate being changed. Add integration tests when behavior crosses crate boundaries, such as CLI-to-server event compatibility. Run `cargo nextest run --workspace` (or `cargo test --workspace`) before opening a PR.

## Commit & Pull Request Guidelines

The current history uses short, imperative summaries such as `Implement Rust agent notify prototype`. Follow that style: one concise subject line describing the change. For pull requests, include the purpose, notable implementation details, verification commands run, and any hardware or platform assumptions. Attach screenshots only when UI or tray-display behavior changes.

## Security & Configuration Tips

Do not commit real `AGENT_NOTIFY_TOKEN` values or local bridge configuration. For Windows bridge development, keep `%APPDATA%\agent-notify\bridge.toml` local and use `mock_display = true` or `--mock-display` when UHK80 hardware is unavailable.
