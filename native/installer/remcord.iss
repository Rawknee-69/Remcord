; Native Remcord Windows installer (release blocker 3).
;
; Per-user Inno Setup installer for the native WebView2 client. This is a
; SEPARATE distribution from the production Electron client and must never
; detect, upgrade, or uninstall it:
;
; - Stable AppId below (the GUID the native product shipped with).
;   Upgrade detection is scoped to this ID only, so a Remcord setup upgrades
;   the already-installed native product in place.
; - Install target `{localappdata}\Programs\Remcord`. A [Code] guard refuses
;   data directories and existing application directories, identified by
;   content markers rather than product names, so the guard keeps working
;   across product generations without naming any of them.
; - The Remcord profile (`{localappdata}\Remcord`) lives OUTSIDE the
;   install directory and is never listed for deletion: install, upgrade,
;   repair, uninstall, and manual rollback all preserve user data. A
;   previous-generation native profile, if present, is likewise never
;   touched by the installer; the application offers a user-mediated import
;   on first launch instead.
; - `PrivilegesRequired=lowest`: per-user, no administrator rights.
; - A running instance is handled by the Restart Manager
;   (`CloseApplications=yes`), which only ever sees locks on files inside
;   the dedicated install directory — never image-name matching. Image-name
;   operations are forbidden. No process is forcibly terminated.
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
#define InstallLeaf "Remcord"

[Setup]
AppId={{{#AppIdValue}}}
AppName=Remcord
AppVersion={#AppVersion}
AppVerName=Remcord {#AppVersion}
AppPublisher=smartfrigde
DefaultDirName={localappdata}\Programs\{#InstallLeaf}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=dist
OutputBaseFilename=Remcord-Setup-{#AppVersion}
SetupIconFile=..\..\build\icon.ico
UninstallDisplayName=Remcord
UninstallDisplayIcon={app}\remcord.exe
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
Source: "stage\remcord.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "stage\license.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; "Remcord" is the product Start Menu entry, distinct from the production
; Electron client in Start Menu and search.
Name: "{autoprograms}\Remcord"; Filename: "{app}\remcord.exe"

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
    MsgBox('Remcord needs the Microsoft Edge WebView2 Evergreen Runtime, '
      + 'and none was detected on this PC.' + #13#10 + #13#10
      + 'Get it free from Microsoft at:' + #13#10
      + WebView2InfoPage + #13#10 + #13#10
      + 'Setup will continue, but Remcord cannot start until the Runtime is installed. '
      + 'Your data is safe: installing never touches your profiles.',
      mbInformation, MB_OK);
  end;
end;

{ Directory guards: the install target must stay dedicated to Remcord.
  Locations are identified by content markers, never by product names:
  - a settings database (`storage\settings.json`) marks a product data
    directory of any generation;
  - `EBWebView` marks a WebView2 profile root of any generation;
  - `webview-dev-user-data` marks an isolated development profile;
  - an existing `license.txt` next to any `.exe` marks an installed
    application directory (either product).
  Even if typed by hand, setup refuses all of them plus the Remcord
  profile itself. }

function DirContainsInstalledApp(const Dir: String): Boolean;
var
  Rec: TFindRec;
  FoundExe: Boolean;
begin
  Result := False;
  if not FileExists(Dir + '\license.txt') then
    Exit;
  FoundExe := False;
  if FindFirst(Dir + '\*.exe', Rec) then
  begin
    try
      repeat
        if (Rec.Attributes and faDirectory) = 0 then
          FoundExe := True;
      until (not FoundExe) or (not FindNext(Rec));
    finally
      FindClose(Rec);
    end;
  end;
  Result := FoundExe;
end;

function IsForbiddenDir(const Dir: String): Boolean;
var
  Warm: String;
begin
  Warm := Lowercase(Dir);
  Result :=
    FileExists(Dir + '\storage\settings.json') or
    DirExists(Dir + '\EBWebView') or
    (Pos('webview-dev-user-data', Warm) > 0) or
    (Warm = Lowercase(ExpandConstant('{localappdata}\Remcord'))) or
    DirContainsInstalledApp(Dir);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = wpSelectDir then
  begin
    if IsForbiddenDir(WizardDirValue) then
    begin
      MsgBox('This folder belongs to an installed application or to user data. '
        + 'Please keep the default Remcord installation folder.',
        mbError, MB_OK);
      Result := False;
    end;
  end;
end;
