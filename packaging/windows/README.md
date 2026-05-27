# Windows installer (Inno Setup)

Release builds are produced by [`.github/workflows/release.yml`](../../.github/workflows/release.yml) on tag push. You do not need a local Windows machine to publish.

## Optional local build

1. Install [Inno Setup](https://jrsoftware.org/isinfo.php) 6.x.
2. From the repo root:

```powershell
cargo build --release -p agent-notify-bridge --locked
iscc packaging\windows\agent-notify-bridge.iss /DAppVersion=0.1.0 /DReleaseDir=target\release
```

Output: `packaging/windows/output/agent-notify-bridge-setup.exe`

## Silent install

```powershell
.\agent-notify-bridge-setup.exe /VERYSILENT /SP- /SERVERURL=http://host:8787 /TOKEN=your-token
```

Skip login autostart: add `/NOAUTOSTART`.

## winget (after a GitHub release exists)

Submit or update manifests under [`packaging/winget/`](../winget/) using the release asset URL, then:

```powershell
winget install glslang.agent-notify-bridge
```

With custom server/token:

```powershell
winget install glslang.agent-notify-bridge --override '/VERYSILENT /SP- /SERVERURL=http://host:8787 /TOKEN=your-token'
```

Tokens on the command line may appear in shell history and logs; prefer the interactive installer or editing `%APPDATA%\agent-notify\bridge.toml` when possible.
