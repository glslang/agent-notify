# agent-notify

Server and Windows bridge for showing coding-agent status on a UHK80 OLED display.

## Shape

- `agent-notify-server`: Linux-friendly HTTP/WebSocket event collector.
- `agent-notify-bridge`: Windows system tray bridge that connects outbound to the server and writes to the local UHK80 over HID.
- `agent-notify-cli`: small hook sender for Codex or other coding agents.

The Linux server never talks to the keyboard. The Windows machine that currently owns the UHK80 through the monitor USB/KVM updates the display locally.

## Run the server

```sh
AGENT_NOTIFY_TOKEN=change-me cargo run -p agent-notify-server -- --bind 0.0.0.0:8787
```

## Send a test event

```sh
AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
cargo run -p agent-notify-cli -- \
  --state waiting-input \
  --agent codex \
  --repo agent-notify \
  --summary "waiting for input"
```

## Run the bridge

On Windows, create `%APPDATA%\agent-notify\bridge.toml`:

```toml
server_url = "http://linux-server:8787"
token = "change-me"
mock_display = false
```

Then run:

```powershell
agent-notify-bridge.exe
```

For development without a UHK80:

```sh
AGENT_NOTIFY_TOKEN=change-me cargo run -p agent-notify-bridge -- --mock-display
```
