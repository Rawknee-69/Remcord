# Static packaging checks for the native Windows installer (§6).
# Runs WITHOUT the Inno compiler: validates the .iss definition, the
# staged file set, and separation from production Electron. Fails on the
# first violation. Content checks ignore `;` comment lines (comments may
# document the very paths the code must avoid); code and comments are both
# inspected where each matters. Rust tests alone are never treated as
# installer proof; these checks are the script-level half (lifecycle
# behavior still needs an isolated environment).
param(
    [string] $RepoRoot = "E:/Legcord"
)
$ErrorActionPreference = "Stop"
$failures = @()
function Check([bool] $ok, [string] $name) {
    if ($ok) { Write-Host "ok: $name" } else { $script:failures += $name; Write-Host "FAIL: $name" }
}
$native = Join-Path $RepoRoot "native"
$issPath = Join-Path $native "installer/legcord.iss"
$iss = Get-Content -Raw -LiteralPath $issPath
$code = (($iss -split "`r?`n") | Where-Object { -not $_.TrimStart().StartsWith(";") }) -join "`n"
$package = Get-Content -Raw (Join-Path $RepoRoot "package.json") | ConvertFrom-Json

# 1. Unique installer ID, GUID-shaped, not Electron's.
$guid = [regex]::Match($code, '#define AppIdValue "([0-9A-Fa-f-]{36})"').Groups[1].Value
Check ($guid -ne "") "AppId define present and GUID-shaped"
Check ($code -match "AppId=\{\{#AppIdValue\}\}") "Setup consumes the defined AppId"
Check ($code -notmatch "app\.legcord\.Legcord") "no Electron appId reference in code"
Check ($iss -match "app\.legcord\.Legcord") "difference from Electron appId documented in comments"

# 2. Install-path separation.
$leaf = [regex]::Match($code, '#define InstallLeaf "([A-Za-z0-9]+)"').Groups[1].Value
Check ($leaf -eq "LegcordNative") "install leaf is LegcordNative"
Check ($code -match "DefaultDirName=\{localappdata\}\\Programs\\\{#InstallLeaf\}") "dedicated per-user install dir"
Check ($code -match "PrivilegesRequired=lowest") "no administrator requirement"
Check ($code -match "UsePreviousAppDir=yes") "in-place upgrade scoped to previous dir"
Check ($code -match "CloseApplications=yes") "running instance handled gracefully"
Check (-not ($code -match "taskkill|TerminateProcess|/IM ")) "no image-name process killing"
Check ($code -match "RestartApplications=no") "no forced application restart"

# 3. Electron/DEV paths appear ONLY inside the [Code] refusal guard.
$head = ($code -split "\[Code\]")[0]
$tail = ($code -split "\[Code\]")[-1]
foreach ($needle in @("Programs\Legcord", "webview-dev-user-data", "appdata}\legcord", "localappdata}\LegcordNative")) {
    Check ($head.ToLowerInvariant().IndexOf($needle.ToLowerInvariant()) -lt 0) "installer head free of '$needle'"
}
Check ($tail -match "LegcordNative" -and $tail -match "Programs\\Legcord") "refusal guard names the protected locations"
Check ($code -match "function NextButtonClick") "directory refusal guard present"
Check ($code -match "IsForbiddenDir") "forbidden-dir predicate present"

# 4. Uninstall scope: entry present, profile never listed for deletion.
Check ($code -match "UninstallDisplayName=Legcord Native") "distinct uninstall entry"
Check ($code -match "UninstallDisplayIcon=\{app\}\\legcord\.exe") "uninstall icon from installed exe"
Check (-not ($code -match "\[UninstallDelete\]")) "no UninstallDelete section (profile cannot be listed)"
Check (-not ($head -match "localappdata\}\\LegcordNative")) "profile dir absent outside refusal guard"

# 5. Version metadata: single source (package.json via /DAppVersion).
Check ($code -match "#error AppVersion must be passed") "unversioned compile refused"
$setupName = [regex]::Match($code, "OutputBaseFilename=(\S+)").Groups[1].Value
Check ($setupName -like "LegcordNative-Setup-*") "predictable versioned setup filename"

# 6. File set: exactly the staged exe + license; icon from repo asset.
Check ($code -match 'Source: "stage\\legcord\.exe"') "packages staged exe only"
Check ($code -match 'Source: "stage\\license\.txt"') "packages license only"
Check (([regex]::Matches($code, 'Source: "')).Count -eq 2) "no additional packaged files"
Check ($code -match "SetupIconFile=" -and $code -match "build\\icon\.ico") "setup icon from repo asset"
Check ($code -match 'Name: "\{autoprograms\}\\Legcord Native"') "distinct Start Menu label"

# 7. Out of scope stays out (documents blockers, not silent gaps).
Check (-not ($code -match "signtool")) "no signing claims in installer"
Check (-not ($code -match "autoupdate|update channel")) "no updater claims in installer"

# 8. Cross-check live product metadata when a release exe exists.
$exe = Join-Path $native "target/release/legcord.exe"
if (Test-Path -LiteralPath $exe -PathType Leaf) {
    $info = (Get-Item -LiteralPath $exe).VersionInfo
    Check ($info.ProductVersion -eq $package.version) "exe ProductVersion matches package.json ($($package.version))"
    Check ($info.OriginalFilename -eq "legcord.exe") "exe OriginalFilename is legcord.exe"
} else {
    Write-Host "skip: no release exe built yet (run build-installer.ps1)"
}

if ($failures.Count -gt 0) { throw "packaging checks failed: $($failures -join '; ')" }
Write-Host "all packaging checks passed"
