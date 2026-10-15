; Native Remcord Windows installer (release blocker 3).
;
; Per-user Inno Setup installer for the native WebView2 client. This is a
; SEPARATE distribution from the production Electron client and must never
; detect, upgrade, or uninstall it:
;
; - Stable AppId below (the GUID the native product shipped with).
;   Upgrade detection is scoped to this ID only, so a Remcord setup upgrades
;   the already-installed native product in place.
; - Install target `{localappdata}\Programs\Remcord`, always chosen on the
;   destination page (any folder or drive). A [Code] guard refuses
;   data directories and existing application directories, identified by
;   content markers rather than product names, so the guard keeps working
;   across product generations without naming any of them. The one
;   exception is the currently-installed location itself (read from the
;   uninstall registry entry): reinstalling or upgrading in place must
;   never be refused by the guard that protects everywhere else.
; - User data: `%LOCALAPPDATA%\Remcord` by default — outside the install
;   directory and never listed for deletion, so install, upgrade, repair,
;   uninstall, and manual rollback all preserve it. Optionally, the
;   `portabledata` task creates a `remcord-data` folder next to the
;   executable: the app then keeps everything inside the install folder,
;   wherever the user pointed the installer (a fully self-contained,
;   Electron-like install). Uninstall never deletes user data in either
;   mode: unknown files and non-empty data directories are left behind.
; - The Remcord profile (`{localappdata}\Remcord`) lives OUTSIDE the
;   install directory and is never listed for deletion: install, upgrade,
;   repair, uninstall, and manual rollback all preserve user data. The
;   portable `remcord-data` folder is likewise never listed: uninstall
;   removes only the files it installed, so user data stays on the PC. A
;   previous-generation native profile, if present, is likewise never
;   touched by the installer; the application offers a user-mediated import
;   on first launch instead.
; - `PrivilegesRequired=lowest`: per-user, no administrator rights.
; - Electron-parity install experience: the wizard always shows the
;   destination page (`DisableDirPage=no`, so any folder or drive can be
;   picked via Browse), a license page, an optional desktop shortcut
;   (checked by default), Start Menu entry (always created, so Search finds
;   Remcord), and a "Launch Remcord" checkbox on the finish page.
;   The app itself is a Windows GUI binary (see the `windows_subsystem`
;   attribute in native/src/main.rs), so shortcut/Search launches never
;   open a terminal window.
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
AppPublisher=rawknee-69
VersionInfoCompany=rawknee-69
VersionInfoDescription=Remcord Setup
VersionInfoProductName=Remcord
DefaultDirName={localappdata}\Programs\{#InstallLeaf}
DisableDirPage=no
AlwaysShowDirOnReadyPage=yes
DisableProgramGroupPage=yes
LicenseFile=..\..\license.txt
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

[Dirs]
; Portable user-data marker, created only when the `portabledata` task is
; selected. The empty folder makes the installed app keep all user data
; inside the install location; uninstall leaves it (and any data in it)
; behind, so user data stays on the PC.
Name: "{app}\remcord-data"; Tasks: portabledata

[Icons]
; "Remcord" is the product Start Menu entry, distinct from the production
; Electron client in Start Menu and search. It is always created so
; Windows Search finds Remcord after install. The desktop shortcut is
; optional but checked by default (see [Tasks]).
Name: "{autoprograms}\Remcord"; Filename: "{app}\remcord.exe"
Name: "{autodesktop}\Remcord"; Filename: "{app}\remcord.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: checkedonce
Name: "portabledata"; Description: "Keep my Remcord &data next to the app (portable install)"; GroupDescription: "User data:"; Flags: unchecked

[Run]
; Electron-parity finish page: offer to start the app right after install.
Filename: "{app}\remcord.exe"; Description: "Launch Remcord"; Flags: nowait postinstall skipifsilent

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

{ Directory guards: user-data directories are ALWAYS refused (installing
  into one would endanger settings, themes, and browser caches), while a
  folder that merely contains another installed application asks for
  confirmation instead of refusing: Inno only ever removes the files it
  installed, so side-by-side installs are safe.
  Locations are identified by content markers, never by product names:
  - a settings database (`storage\settings.json`) marks a product data
    directory of any generation;
  - `EBWebView` marks a WebView2 profile root of any generation;
  - `webview-dev-user-data` marks an isolated development profile;
  - `remcord-data` marks a portable Remcord data directory;
  - an existing `license.txt` next to any `.exe` marks an installed
    application directory (either product) — confirmation, not refusal.
  Even if typed by hand, setup refuses all data locations plus the Remcord
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
        if (Rec.Attributes and FILE_ATTRIBUTE_DIRECTORY) = 0 then
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
    DirExists(Dir + '\remcord-data') or
    (Warm = Lowercase(ExpandConstant('{localappdata}\Remcord')));
end;

{ In-place upgrade: the destination page is always shown
  (`DisableDirPage=no`), so on upgrade it opens on the previous location —
  which now contains the installed app files the guard below refuses
  everywhere else. Reinstalling into the currently-installed location
  (read from the per-user uninstall entry this setup maintains) is always
  allowed; the guard still protects every other application directory and
  every user-data directory. }

function GetInstalledAppDir(): String;
var
  Dir: String;
begin
  Result := '';
  if RegQueryStringValue(HKCU,
    'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{{{#AppIdValue}}}_is1',
    'Inno Setup: App Path', Dir) then
    Result := Dir;
end;

function IsSamePath(const A, B: String): Boolean;
var
  NormA, NormB: String;
begin
  NormA := Lowercase(A);
  NormB := Lowercase(B);
  while (Length(NormA) > 0) and (NormA[Length(NormA)] = '\') do
    Delete(NormA, Length(NormA), 1);
  while (Length(NormB) > 0) and (NormB[Length(NormB)] = '\') do
    Delete(NormB, Length(NormB), 1);
  Result := (NormA <> '') and (NormA = NormB);
end;

{ Last folder component of a path (`E:\LegcordNative` -> `LegcordNative`),
  ignoring trailing backslashes. Used to recognise a previous-generation
  install folder. }
function PathLeaf(const Dir: String): String;
var
  P: String;
  I: Integer;
begin
  P := Dir;
  while (Length(P) > 0) and (P[Length(P)] = '\') do
    Delete(P, Length(P), 1);
  I := Length(P);
  while (I > 0) and (P[I] <> '\') and (P[I] <> ':') do
    Dec(I);
  Result := Copy(P, I + 1, Length(P));
end;

{ Legacy-folder redirect: machines that still carry a previous-generation
  install location (e.g. `E:\LegcordNative`) would otherwise open every
  wizard on that folder via `UsePreviousAppDir`, so the old path — and
  the old name — never goes away. When the remembered location is not a
  Remcord folder, and it holds no portable user data (which must stay
  where it is for data continuity), the wizard opens on the fresh
  per-user Remcord folder instead. A proper Remcord home is never
  redirected: upgrades stay in place. }
procedure InitializeWizard();
var
  Prev, Fresh: String;
begin
  Prev := GetInstalledAppDir();
  if Prev = '' then
    Exit;
  if Lowercase(PathLeaf(Prev)) = Lowercase('{#InstallLeaf}') then
    Exit;
  if DirExists(Prev + '\remcord-data') then
    Exit;
  Fresh := ExpandConstant('{localappdata}\Programs\{#InstallLeaf}');
  WizardForm.DirEdit.Text := Fresh;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = wpSelectDir then
  begin
    if IsSamePath(WizardDirValue, GetInstalledAppDir()) then
      Exit;
    if IsForbiddenDir(WizardDirValue) then
    begin
      MsgBox('This folder holds user data (settings, themes, or browser data). '
        + 'Installing here would endanger it. '
        + 'Please choose a different folder.',
        mbError, MB_OK);
      Result := False;
      Exit;
    end;
    if DirContainsInstalledApp(WizardDirValue) then
    begin
      if MsgBox('This folder already contains an installed application. '
        + 'Remcord will install alongside it — uninstalling Remcord later '
        + 'removes only Remcord''s own files.' + #13#10 + #13#10
        + 'Install here anyway?',
        mbConfirmation, MB_YESNO) = IDYES then
        Exit;
      Result := False;
    end;
  end;
end;
