# agent-notify

[![CI](https://github.com/glslang/agent-notify/actions/workflows/ci.yml/badge.svg)](https://github.com/glslang/agent-notify/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/glslang/agent-notify/graph/badge.svg)](https://codecov.io/gh/glslang/agent-notify)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://github.com/glslang/agent-notify/blob/main/LICENSE)

Rust workspace plus TypeScript CLI for pushing **coding-agent** status to a tiny HTTP/WebSocket server, then showing it on a **UHK80** OLED via a Windows system-tray bridge (HID locally—no GTK on the tray path).

## Components

- **`agent-notify-server`**: HTTP + WebSocket event collector (friendly to run on Linux or elsewhere).
- **`agent-notify-bridge`**: Windows tray app; connects to the server and updates the UHK80 over HID.
- **`agent-notify-cli`**: Rust hook sender for Codex and similar agents.
- **`clients/typescript/agent-notify-cli`**: Node 20+ CLI with the same HTTP API as the Rust binary; install and versioning are described in [`clients/typescript/agent-notify-cli/README.md`](clients/typescript/agent-notify-cli/README.md).

The Linux server never drives the keyboard. The machine that currently owns the UHK80 (often through a monitor USB/KVM) runs the bridge and updates the display.

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

## Semantics and limits

The server stores one in-memory latest notification. New events replace earlier events from the same work item, where a matching `run_id` wins first and otherwise `agent`, `host`, and `repo` are compared. Different live events are ranked by priority and receive time; default priorities are `waiting-input` 90, `failed` 80, `done` 50, and `running` 20.

Events expire after their TTL. The default TTL is 120 seconds, and the server clamps requested TTL values to 1 through 3600 seconds.

When the Windows bridge is paused, the server still accepts events and updates `GET /v1/events/latest`, but the bridge suppresses WebSocket deliveries while paused. The keyboard can therefore intentionally keep showing its previous state until the bridge is unpaused, reconnected, or explicitly dismissed.

The current server keeps state in memory and broadcasts from a single process. It is intended for a small single-server setup, not multi-instance high availability or audit logging.

## Security and deployment

Authentication is a single bearer token (`AGENT_NOTIFY_TOKEN`), sent in the `Authorization` header for HTTP and as a `?token=` query parameter for the bridge WebSocket. The server logs only the request method and path, never the query string, so the token does not land in access logs—but a TLS-terminating proxy in front of it might log full URLs, so configure that accordingly.

The server does not terminate TLS itself. For any non-loopback deployment, front it with a reverse proxy that provides TLS; otherwise the bearer token travels in cleartext. The server applies a request body limit and a global concurrency limit, but it performs no origin checking on WebSocket upgrades and no per-client rate limiting, so treat the token as the only access control and keep the listener off untrusted networks.

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

## Contributing

Conventions, test commands (`cargo nextest`, TypeScript `npm test`), and security tips are in [`AGENTS.md`](AGENTS.md). Keep real `AGENT_NOTIFY_TOKEN` values and `bridge.toml` secrets out of git.
