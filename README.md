# agent-notify

Server and Windows bridge for showing coding-agent status on a UHK80 OLED display.

## Shape

- `agent-notify-server`: Linux-friendly HTTP/WebSocket event collector.
- `agent-notify-bridge`: Windows system tray bridge that connects outbound to the server and writes to the local UHK80 over HID.
- `agent-notify-cli`: small hook sender for Codex or other coding agents.
- `clients/typescript/agent-notify-cli`: npm hook sender with the same HTTP interface as the Rust CLI.

The Linux server never talks to the keyboard. The Windows machine that currently owns the UHK80 through the monitor USB/KVM updates the display locally.

## Run the server

```sh
AGENT_NOTIFY_TOKEN=change-me cargo run -p agent-notify-server -- --bind 0.0.0.0:8787
```

## Send a test event

With the Rust CLI:

```sh
AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
cargo run -p agent-notify-cli -- \
  --state waiting-input \
  --agent codex \
  --repo agent-notify \
  --summary "waiting for input"
```

With the npm CLI:

```sh
npm install -g agent-notify-cli

AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
agent-notify-cli \
  --state waiting-input \
  --agent codex \
  --repo agent-notify \
  --summary "waiting for input"
```

Or without a global install:

```sh
npx agent-notify-cli \
  --server http://127.0.0.1:8787 \
  --token change-me \
  --state done \
  --repo agent-notify \
  --summary "finished"
```

## Dismiss the current notification

```sh
AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
cargo run -p agent-notify-cli -- --dismiss
```

The npm CLI supports the same operation:

```sh
AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
agent-notify-cli --dismiss
```

The Windows bridge tray menu also includes "Dismiss notification".

## Agent hook examples

Codex-style shell hook:

```sh
AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
npx agent-notify-cli \
  --state waiting-input \
  --agent codex \
  --repo "$PWD" \
  --summary "waiting for input"
```

Claude Code hook commands:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "npx agent-notify-cli --state done --agent claude --repo \"$PWD\" --summary \"finished\""
          }
        ]
      }
    ],
    "Notification": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "npx agent-notify-cli --state waiting-input --agent claude --repo \"$PWD\" --summary \"waiting for input\""
          }
        ]
      }
    ]
  }
}
```

Set `AGENT_NOTIFY_SERVER` and `AGENT_NOTIFY_TOKEN` in the hook environment, or pass `--server` and `--token`.

## Run the bridge

On Windows, create `%APPDATA%\agent-notify\bridge.toml`. The bridge does not create this file automatically.

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
