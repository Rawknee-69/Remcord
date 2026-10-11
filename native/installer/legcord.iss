; Native Legcord Windows installer (release blocker 3).
;
; Per-user Inno Setup installer for the native WebView2 client. This is a
; SEPARATE distribution from Electron Legcord and must never detect,
; upgrade, or uninstall it:
;
; - Unique AppId below (a fresh GUID, unrelated to Electron's
;   `app.legcord.Legcord`). Upgrade detection is scoped to this ID only.
; - Install target `{localappdata}\Programs\LegcordNative` — never the
;   Electron directory (`{localappdata}\Programs\Legcord`); a [Code]
;   guard refuses it and every other known product path.
; - The native profile (`{localappdata}\LegcordNative`) lives OUTSIDE the
;   install directory and is never listed for deletion: install, upgrade,
;   repair, uninstall, and manual rollback all preserve user data.
; - `PrivilegesRequired=lowest`: per-user, no administrator rights.
; - A running instance is handled by the Restart Manager
;   (`CloseApplications=yes`), which only ever sees locks on files inside
;   the dedicated install directory — never image-name matching (the two
;   products share the `legcord.exe` image name, so image-name operations
;   are forbidden). No process is forcibly terminated.
; - Manual rollback = run an older setup: same AppId replaces the files
;   in place; the profile is untouched. (Runtime-verified lifecycle is
;   still outstanding for lack of an isolated test environment; see
;   docs/WEBVIEW2_RELEASE_READINESS.md.)
;
; No auto-updater, no signing (separate blockers). WebView2 Runtime is an
; external prerequisite (separate blocker).
;
; Built only via native/installer/build-installer.ps1, which passes
; /DAppVersion= from package.json and stages exactly the files below.
; Do not compile by hand with different sources.

#ifndef AppVersion
  #error AppVersion must be passed as /DAppVersion=<version> by build-installer.ps1
#endif

#define AppIdValue "5F76ADED-7123-4C1A-931E-625935D39BA6"
#define InstallLeaf "LegcordNative"

[Setup]
AppId={{#AppIdValue}}
AppName=Legcord Native
AppVersion={#AppVersion}
AppVerName=Legcord Native {#AppVersion}
AppPublisher=smartfrigde
AppPublisherURL=https://github.com/Legcord/Legcord
DefaultDirName={localappdata}\Programs\{#InstallLeaf}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=dist
OutputBaseFilename=LegcordNative-Setup-{#AppVersion}
SetupIconFile=..\..\build\icon.ico
UninstallDisplayName=Legcord Native
UninstallDisplayIcon={app}\legcord.exe
UsePreviousAppDir=yes
DirExistsWarning=yes
CloseApplications=yes
RestartApplications=no
ArchitecturesAllowed=x64compatible
SolidCompression=yes
WizardStyle=modern

[Files]
; Exactly the staged release files — never profile data, logs, caches, or
; anything outside the staging directory (enforced by check-installer.ps1).
Source: "stage\legcord.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "stage\license.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; "Legcord Native" keeps the shortcut visually distinct from production
; Electron "Legcord" in Start Menu and search.
Name: "{autoprograms}\Legcord Native"; Filename: "{app}\legcord.exe"

[Code]
{ WebView2 Runtime prerequisite (release blocker 4): warn-and-continue.
  The Evergreen Runtime registration is read from the documented EdgeUpdate
  ClientState locations (per-machine 64/32-bit views plus per-user) with no
  plugins and no elevation. If nothing is registered, setup shows where to
  get the official Runtime and CONTINUES: registry heuristics can miss valid
  systems, and the application itself gates authoritatively at launch with
  the loader check (webview_runtime.rs) plus an actionable dialog. No
  download is invoked, no payload is bundled, no privilege is requested. }

const
  WebView2ClientState = 'SOFTWARE\Microsoft\EdgeUpdate\ClientState\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  WebView2InfoPage = 'https://developer.microsoft.com/microsoft-edge/webview2/';

function WebView2RuntimePresent(): Boolean;
var
  Version: String;
begin
  Result :=
    (RegQueryStringValue(HKLM64, WebView2ClientState, 'pv', Version) and (Version <> '')) or
    (RegQueryStringValue(HKLM32, WebView2ClientState, 'pv', Version) and (Version <> '')) or
    (RegQueryStringValue(HKCU, WebView2ClientState, 'pv', Version) and (Version <> ''));
end;

function InitializeSetup(): Boolean;
begin
  Result := True;
  if not WebView2RuntimePresent() then
  begin
    MsgBox('Legcord Native needs the Microsoft Edge WebView2 Evergreen Runtime, '
      + 'and none was detected on this PC.' + #13#10 + #13#10
      + 'Get it free from Microsoft at:' + #13#10
      + WebView2InfoPage + #13#10 + #13#10
      + 'Setup will continue, but Legcord Native cannot start until the Runtime is installed. '
      + 'Your data is safe: installing never touches your profiles.',
      mbInformation, MB_OK);
  end;
end;

{ Directory guards: the install target must stay dedicated to native
  Legcord. Refuse the Electron installation, the Electron profile, the
  native profile, and the DEV profile, even if typed by hand. }

function IsForbiddenDir(const Dir: String): Boolean;
var
  Warm: String;
begin
  Warm := Lowercase(Dir);
  Result :=
    (Warm = Lowercase(ExpandConstant('{localappdata}\Programs\Legcord'))) or
    (Pos(Lowercase(ExpandConstant('{localappdata}\Programs\Legcord\')), Warm) = 1) or
    (Warm = Lowercase(ExpandConstant('{userappdata}\legcord'))) or
    (Warm = Lowercase(ExpandConstant('{localappdata}\LegcordNative'))) or
    (Warm = 'e:\legcord.webview-dev-user-data');
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = wpSelectDir then
  begin
    if IsForbiddenDir(WizardDirValue) then
    begin
      MsgBox('This folder belongs to another Legcord installation or to user data. '
        + 'Please keep the default native installation folder.',
        mbError, MB_OK);
      Result := False;
    end;
  end;
end;
