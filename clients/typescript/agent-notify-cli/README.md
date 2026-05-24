# agent-notify-cli

Hook sender for agent-notify. This npm package mirrors the Rust `agent-notify-cli` command for coding-agent machines that already have Node installed.

It only sends HTTP events. It does not run the server, bridge, tray, or UHK HID integration.

## Install

```sh
npm install -g agent-notify-cli
```

You can also run it without installing globally:

```sh
npx agent-notify-cli --help
```

## Send Events

```sh
AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
agent-notify-cli \
  --state waiting-input \
  --agent codex \
  --repo agent-notify \
  --summary "waiting for input"
```

```sh
npx agent-notify-cli \
  --server http://127.0.0.1:8787 \
  --token change-me \
  --state done \
  --repo agent-notify \
  --summary "finished"
```

## Dismiss

```sh
AGENT_NOTIFY_SERVER=http://127.0.0.1:8787 \
AGENT_NOTIFY_TOKEN=change-me \
agent-notify-cli --dismiss
```

## Agent Hook Examples

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

Claude Code hook command:

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

## Interface

- `--server` / `AGENT_NOTIFY_SERVER`
- `--token` / `AGENT_NOTIFY_TOKEN`
- `--agent` / `AGENT_NOTIFY_AGENT`, default `codex`
- `--host` / `AGENT_NOTIFY_HOST`, with local hostname fallback
- `--repo` / `AGENT_NOTIFY_REPO`
- `--state running|waiting-input|done|failed`
- `--summary`
- `--priority`
- `--ttl-seconds`
- `--run-id`
- `--dismiss`
