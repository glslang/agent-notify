# winget manifests (template)

These YAML files are **templates** for submitting to [winget-pkgs](https://github.com/microsoft/winget-pkgs). They are not consumed automatically by CI yet.

## After the first GitHub release

1. Download `agent-notify-bridge-setup.exe` from the release (or read `agent-notify-bridge-setup.exe.sha256`).
2. Copy `packaging/winget/glslang.agent-notify-bridge/<version>/` with the new version folder.
3. Set `InstallerSha256` and `InstallerUrl` to match the release asset.
4. Submit with [Komac](https://github.com/russellbanks/Komac) or `wingetcreate submit`.

```powershell
komac new glslang.agent-notify-bridge --version 0.1.0 `
  --urls https://github.com/glslang/agent-notify/releases/download/v0.1.0/agent-notify-bridge-setup.exe
```

Or open a PR manually using the template under `glslang.agent-notify-bridge/`.

## Install (once published to winget-pkgs)

```powershell
winget install glslang.agent-notify-bridge
```

Silent install with server and token:

```powershell
winget install glslang.agent-notify-bridge --override '/VERYSILENT /SP- /SERVERURL=http://host:8787 /TOKEN=your-token'
```
