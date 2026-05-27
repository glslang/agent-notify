; Agent Notify Bridge — Inno Setup installer (built in CI via iscc).
;
; Compile-time defines (CI):
;   /DAppVersion=0.1.0
;   /DReleaseDir=..\..\target\release
;
; Silent install with config (runtime, winget --override):
;   setup.exe /VERYSILENT /SP- /SERVERURL=http://host:8787 /TOKEN=secret
;
; Optional: /NOAUTOSTART to skip login Run key even when config is present.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef ReleaseDir
  #define ReleaseDir "..\..\target\release"
#endif

#define MyAppName "Agent Notify Bridge"
#define MyAppExe "agent-notify-bridge.exe"
#define MyAppPublisher "glslang"
#define MyAppURL "https://github.com/glslang/agent-notify"

[Setup]
AppId={{A7B4E2C1-9F3D-4B8E-A1C6-5D2E8F0B4A91}
AppName={#MyAppName}
AppVersion={#AppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\Programs\agent-notify-bridge
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputBaseFilename=agent-notify-bridge-setup
OutputDir=output
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppExe}
LicenseFile=..\..\LICENSE
VersionInfoVersion={#AppVersion}.0

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start {#MyAppName} when I sign in to Windows"; GroupDescription: "Startup:"; Flags: checkedonce

[Files]
Source: "{#ReleaseDir}\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Registry]
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "agent-notify-bridge"; ValueData: """{app}\{#MyAppExe}"""; Flags: uninsdeletevalue; Tasks: autostart; Check: ShouldInstallAutostart

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"

[Code]
var
  ConfigPage: TInputQueryWizardPage;
  ConfigDir: string;
  ConfigPath: string;
  ServerUrl: string;
  Token: string;
  SkipAutostart: Boolean;

function CmdLineParam(const PName: string): string;
var
  I: Integer;
  S: string;
  Prefix: string;
begin
  Result := '';
  Prefix := '/' + PName + '=';
  for I := 1 to ParamCount do
  begin
    S := ParamStr(I);
    if CompareText(S, '/' + PName) = 0 then
    begin
      if I < ParamCount then
        Result := ParamStr(I + 1);
      Exit;
    end;
    if CompareText(Copy(S, 1, Length(Prefix)), Prefix) = 0 then
    begin
      Result := Copy(S, Length(Prefix) + 1, MaxInt);
      Exit;
    end;
  end;
end;

function HasCmdLineFlag(const Flag: string): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/' + Flag) = 0 then
    begin
      Result := True;
      Exit;
    end;
end;

function ConfigIsComplete: Boolean;
begin
  Result := (Trim(ServerUrl) <> '') and (Trim(Token) <> '');
end;

function ShouldInstallAutostart: Boolean;
begin
  Result := ConfigIsComplete and not SkipAutostart;
end;

function EscapeTomlString(const S: string): string;
begin
  Result := S;
  StringChangeEx(Result, '\', '\\', True);
  StringChangeEx(Result, '"', '\"', True);
end;

procedure LoadParameters;
begin
  ServerUrl := Trim(CmdLineParam('SERVERURL'));
  Token := Trim(CmdLineParam('TOKEN'));
  SkipAutostart := HasCmdLineFlag('NOAUTOSTART');
end;

function ShouldSkipConfigPage(Sender: TWizardPage): Boolean;
begin
  Result := WizardSilent or ConfigIsComplete;
end;

procedure InitializeWizard;
begin
  LoadParameters;
  ConfigPage := CreateInputQueryPage(wpSelectTasks,
    'Server configuration', 'Connect to your agent-notify server',
    'Required before the bridge can start. You can edit %APPDATA%\agent-notify\bridge.toml later.');
  ConfigPage.Add('Server URL (server_url):', False);
  ConfigPage.Add('Token:', True);
  if ServerUrl <> '' then
    ConfigPage.Values[0] := ServerUrl;
  if Token <> '' then
    ConfigPage.Values[1] := Token;
  ConfigPage.ShouldSkipPage := @ShouldSkipConfigPage;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = ConfigPage.ID then
  begin
    ServerUrl := Trim(ConfigPage.Values[0]);
    Token := Trim(ConfigPage.Values[1]);
  end;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = ConfigPage.ID then
  begin
    ServerUrl := Trim(ConfigPage.Values[0]);
    Token := Trim(ConfigPage.Values[1]);
    if not ConfigIsComplete then
    begin
      MsgBox('Server URL and token are required.', mbError, MB_OK);
      Result := False;
    end;
  end;
end;

function WriteBridgeConfig: Boolean;
var
  Lines: TArrayOfString;
begin
  Result := True;
  if not ConfigIsComplete then
    Exit;
  if FileExists(ConfigPath) then
    Exit;

  ConfigDir := ExpandConstant('{userappdata}\agent-notify');
  ConfigPath := ConfigDir + '\bridge.toml';
  if not DirExists(ConfigDir) then
    ForceDirectories(ConfigDir);

  SetArrayLength(Lines, 3);
  Lines[0] := 'server_url = "' + EscapeTomlString(ServerUrl) + '"';
  Lines[1] := 'token = "' + EscapeTomlString(Token) + '"';
  Lines[2] := 'mock_display = false';
  if not SaveStringsToFile(ConfigPath, Lines, False) then
  begin
    MsgBox('Failed to write ' + ConfigPath, mbError, MB_OK);
    Result := False;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    if not WizardSilent and (ConfigPage <> nil) and not ShouldSkipConfigPage(nil) then
    begin
      ServerUrl := Trim(ConfigPage.Values[0]);
      Token := Trim(ConfigPage.Values[1]);
    end;
    ConfigPath := ExpandConstant('{userappdata}\agent-notify\bridge.toml');
    if not WriteBridgeConfig then
      Abort;
  end;
end;

function InitializeSetup: Boolean;
begin
  LoadParameters;
  ConfigPath := ExpandConstant('{userappdata}\agent-notify\bridge.toml');
  Result := True;
end;

function InitializeUninstall: Boolean;
begin
  Result := True;
end;
