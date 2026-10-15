# Static packaging checks for the native Remcord Windows installer (§6).
# Runs WITHOUT the Inno compiler: validates the .iss definition, the
# staged file set, and separation from production Electron. Fails on the
# first violation. Content checks ignore `;` comment lines. Rust tests alone
# are never treated as installer proof; these checks are the script-level
# half (lifecycle behavior still needs an isolated environment).
#
# The refusal guard is verified by behavior markers, not product names, so
# this script itself names no product generation: it asserts the guard
# exists, covers data/profile/install markers, and keeps the install head
# free of data locations.
param(
    [string] $RepoRoot = ""
)
$ErrorActionPreference = "Stop"
$failures = @()
function Check([bool] $ok, [string] $name) {
    if ($ok) { Write-Host "ok: $name" } else { $script:failures += $name; Write-Host "FAIL: $name" }
}
if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $nativeProbe = Split-Path -Parent $PSScriptRoot
    $RepoRoot = Split-Path -Parent $nativeProbe
}
$native = Join-Path $RepoRoot "native"
$issPath = Join-Path $native "installer/remcord.iss"
$iss = Get-Content -Raw -LiteralPath $issPath
$code = (($iss -split "`r?`n") | Where-Object { -not $_.TrimStart().StartsWith(";") }) -join "`n"
$package = Get-Content -Raw (Join-Path $RepoRoot "package.json") | ConvertFrom-Json

# 1. Stable installer ID: the GUID the native product shipped with, so a
# Remcord setup upgrades the already-installed native product in place.
$guid = [regex]::Match($code, '#define AppIdValue "([0-9A-Fa-f-]{36})"').Groups[1].Value
Check ($guid -eq "5F76ADED-7123-4C1A-931E-625935D39BA6") "AppId stable across the rebrand"
Check ($code -match "AppId=\{\{\{#AppIdValue\}\}\}") "Setup consumes the defined AppId"
Check ($code -notmatch "app\.[a-z]+\.[A-Za-z]+") "no reverse-DNS appId reference in code"

# 2. Install-path separation.
$leaf = [regex]::Match($code, '#define InstallLeaf "([A-Za-z0-9]+)"').Groups[1].Value
Check ($leaf -eq "Remcord") "install leaf is Remcord"
Check ($code -match "DefaultDirName=\{localappdata\}\\Programs\\\{#InstallLeaf\}") "dedicated per-user install dir"
Check ($code -match "PrivilegesRequired=lowest") "no administrator requirement"
Check ($code -match "UsePreviousAppDir=yes") "in-place upgrade scoped to previous dir"
Check ($code -match "CloseApplications=yes") "running instance handled gracefully"
Check (-not ($code -match "taskkill|TerminateProcess|/IM ")) "no image-name process killing"
Check ($code -match "RestartApplications=no") "no forced application restart"
Check ($code -match "(?m)^DisableDirPage=no") "destination page always shown (any folder or drive)"
Check ($code -match "AlwaysShowDirOnReadyPage=yes") "chosen folder confirmed on ready page"
Check ($code -match "(?m)^LicenseFile=") "license page shown during install"

# 3. Refusal guard: content markers, never product names. Data/profile
# locations must appear ONLY inside the [Code] guard; the [Setup] head must
# stay free of them.
$head = ($code -split "\[Code\]")[0]
$tail = ($code -split "\[Code\]")[-1]
foreach ($needle in @("webview-dev-user-data", "storage\settings.json", "EBWebView", "localappdata}\Remcord")) {
    Check ($head.ToLowerInvariant().IndexOf($needle.ToLowerInvariant()) -lt 0) "installer head free of '$needle'"
}
Check ($tail -match "webview-dev-user-data" -and $tail -match "EBWebView") "refusal guard covers profile markers"
Check ($tail -match "storage" -and $tail -match "settings\.json") "refusal guard covers data markers"
Check ($tail -match "DirContainsInstalledApp") "installed-app heuristic present"
Check ($code -match "function NextButtonClick") "directory refusal guard present"
Check ($code -match "IsForbiddenDir") "forbidden-dir predicate present"

# 4. Uninstall scope: entry present, profile never listed for deletion.
Check ($code -match "UninstallDisplayName=Remcord") "distinct uninstall entry"
Check ($code -match "UninstallDisplayIcon=\{app\}\\remcord\.exe") "uninstall icon from installed exe"
Check (-not ($code -match "\[UninstallDelete\]")) "no UninstallDelete section (profile cannot be listed)"
Check (-not ($head -match "localappdata\}\\Remcord")) "profile dir absent outside refusal guard"

# 5. Version metadata: single source (package.json via /DAppVersion).
Check ($code -match "#error AppVersion must be passed") "unversioned compile refused"
$setupName = [regex]::Match($code, "OutputBaseFilename=(\S+)").Groups[1].Value
Check ($setupName -like "Remcord-Setup-*") "predictable versioned setup filename"

# 6. File set: exactly the staged exe + license; icon from repo asset.
Check ($code -match 'Source: "stage\\remcord\.exe"') "packages staged exe only"
Check ($code -match 'Source: "stage\\license\.txt"') "packages license only"
Check (([regex]::Matches($code, 'Source: "')).Count -eq 2) "no additional packaged files"
Check ($code -match "SetupIconFile=" -and $code -match "build\\icon\.ico") "setup icon from repo asset"
Check ($code -match 'Name: "\{autoprograms\}\\Remcord"') "distinct Start Menu label"
Check ($code -match 'Name: "\{autodesktop\}\\Remcord"') "desktop shortcut created"
Check ($code -match 'Tasks: desktopicon') "desktop shortcut wired to opt-out task"
Check ($code -match '\[Tasks\]') "additional-shortcuts task section present"
Check ($code -match "checkedonce") "desktop shortcut checked by default"
Check ($code -match "(?m)^Filename: ""\{app\}\\remcord\.exe""; Description: ""Launch Remcord""") "finish page offers to launch"
Check ($code -match "portabledata") "portable user-data task present"
Check ($code -match '(?m)^Name: "\{app\}\\remcord-data"; Tasks: portabledata') "portable marker created only when selected"
Check ($code -match "GetInstalledAppDir") "in-place upgrade reads previous location"
Check ($code -match "IsSamePath\(WizardDirValue, GetInstalledAppDir\(\)\)") "upgrade into current location always allowed"
Check ($code -match "procedure InitializeWizard") "wizard redirect initializes destination page"
Check ($code -match "WizardForm\.DirEdit\.Text") "legacy folder redirected to the Remcord folder"

# 7. Out of scope stays out (documents blockers, not silent gaps).
Check (-not ($code -match "signtool")) "no signing claims in installer"
Check (-not ($code -match "autoupdate|update channel")) "no updater claims in installer"

# 8. WebView2 Runtime prerequisite: detect, warn, never block or download.
Check ($code -match "function WebView2RuntimePresent\(\)") "runtime presence function present"
Check ($code -match "F3017226-FE2A-4295-8BDF-00C3A9A7E4C5") "EdgeUpdate ClientState ID present"
Check (($code -match "HKLM64") -and ($code -match "HKLM32") -and ($code -match "HKCU")) "per-machine and per-user views covered"
Check ($code -match "function InitializeSetup\(\).*Result := True" -or ($code -match "function InitializeSetup" -and $code -match "Result := True")) "setup warns and continues (no hard block)"
Check ($code -match "https://developer\.microsoft\.com/microsoft-edge/webview2/") "verified official page referenced"
Check (-not ($code -match "DownloadPage|IDPDownload|InternetReadFile|URLDownloadToFile|bitsadmin|Invoke-WebRequest|curl\.exe|Start-Process.*http")) "no silent download machinery"

# 8. Cross-check live product metadata when a release exe exists.
$exe = Join-Path $native "target/release/remcord.exe"
if (Test-Path -LiteralPath $exe -PathType Leaf) {
    $info = (Get-Item -LiteralPath $exe).VersionInfo
    Check ($info.ProductVersion -eq $package.version) "exe ProductVersion matches package.json ($($package.version))"
    Check ($info.ProductName -eq "Remcord") "exe ProductName is Remcord"
    Check ($info.OriginalFilename -eq "remcord.exe") "exe OriginalFilename is remcord.exe"
    # No-console GUI: the PE optional-header Subsystem must be 2
    # (IMAGE_SUBSYSTEM_WINDOWS_GUI), never 3 (CONSOLE), so shortcut and
    # Search launches never pop a terminal window.
    $bytes = [IO.File]::ReadAllBytes($exe)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C)
    Check ($subsystem -eq 2) "release exe is a GUI binary with no console window (subsystem=$subsystem)"
} else {
    Write-Host "skip: no release exe built yet (run build-installer.ps1)"
}

# 9. No-console source guard: release Windows builds must carry the
# windows_subsystem attribute, or the next rebuild silently regresses to
# a terminal popup.
$mainRs = Get-Content -Raw -LiteralPath (Join-Path $native "src/main.rs")
Check ($mainRs -match 'windows_subsystem\s*=\s*"windows"') "release binary hides the console window"

# 10. Self-contained data: the resolver must honor a `remcord-data`
# marker next to the executable (created by the installer task or by
# hand), so the app plus its data live wherever the user installed.
$profileRs = Get-Content -Raw -LiteralPath (Join-Path $native "src/profile.rs")
Check ($profileRs -match 'PORTABLE_MARKER_LEAF\s*:\s*&str\s*=\s*"remcord-data"') "portable marker is remcord-data"
Check ($profileRs -match "ProfileKind::Portable") "portable profile kind present"
Check ($profileRs -match "portable_dir_for_exe_dir") "portable candidate derived from executable dir"

if ($failures.Count -gt 0) { throw "packaging checks failed: $($failures -join '; ')" }
Write-Host "all packaging checks passed"
