# Builds the native Remcord Windows installer (release blocker 3).
#
# Reproducible from a clean checkout plus one documented prerequisite:
#   winget install --id JRSoftware.InnoSetup --exact   (Inno Setup 6, for ISCC.exe)
# Fails fast with that exact instruction when ISCC.exe is not on PATH.
# Never downloads tools silently, never requires administrator rights.
# Paths derive from this script's location; no checkout path is hardcoded.
#
# Steps: read product version from package.json -> cargo build --release ->
# verify exe identity/resources -> stage exactly {remcord.exe, license.txt}
# -> ISCC native/installer/remcord.iss -> SHA-256 the setup artifact.
# Ships unsigned (signing removed from scope).
#
# Identity: the installer AppId is stable (see remcord.iss) so a Remcord
# setup upgrades the already-installed native product in place. The install
# leaf is Remcord; the release profile is %LOCALAPPDATA%\Remcord.
param(
    [string] $RepoRoot = ""
)
$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $native = Split-Path -Parent $PSScriptRoot
    $RepoRoot = Split-Path -Parent $native
}
$native = Join-Path $RepoRoot "native"
$installerDir = Join-Path $native "installer"
$stage = Join-Path $installerDir "stage"
$dist = Join-Path $installerDir "dist"

$iscc = Get-Command iscc.exe -ErrorAction SilentlyContinue
if ($null -eq $iscc) {
    throw "Inno Setup compiler (ISCC.exe) not found on PATH. Install the documented prerequisite: winget install --id JRSoftware.InnoSetup --exact. Refusing to claim an unbuilt installer works."
}
$package = Get-Content -Raw (Join-Path $RepoRoot "package.json") | ConvertFrom-Json
$version = $package.version
if ([string]::IsNullOrWhiteSpace($version)) { throw "package.json must define a version" }

Write-Host "building release executable (cargo build --release)..."
& cargo build --release --manifest-path (Join-Path $native "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "release build failed" }
$exe = Join-Path $native "target/release/remcord.exe"
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "release exe missing: $exe" }
$info = (Get-Item -LiteralPath $exe).VersionInfo
if ($info.ProductVersion -ne $version) { throw "exe ProductVersion $($info.ProductVersion) != package.json $version" }
if ($info.ProductName -ne "Remcord") { throw "exe ProductName must be Remcord" }
if ($info.OriginalFilename -ne "remcord.exe") { throw "exe OriginalFilename must be remcord.exe" }
# No-console GUI: release exe must be IMAGE_SUBSYSTEM_WINDOWS_GUI (2), never
# CONSOLE (3). The windows_subsystem attribute in native/src/main.rs controls
# this at link time; refuse to ship an installer that pops a terminal window.
$peBytes = [IO.File]::ReadAllBytes($exe)
$peOffset = [BitConverter]::ToInt32($peBytes, 0x3C)
$subsystem = [BitConverter]::ToUInt16($peBytes, $peOffset + 0x5C)
if ($subsystem -ne 2) { throw "release exe PE subsystem is $subsystem, expected 2 (WINDOWS_GUI): shortcut launches would open a terminal window" }
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon($exe)
if ($null -eq $icon -or $icon.Width -le 0) { throw "release exe carries no icon resource" }
Write-Host "exe identity OK: remcord.exe $version (icon $($icon.Width)x$($icon.Height))"

# Staging/output safety: both trees must stay inside this repository.
# (Separation from production install/profile locations is enforced by the
# installer's content-marker refusal guard in remcord.iss.)
foreach ($dir in @($stage, $dist)) {
    $full = [IO.Path]::GetFullPath($dir)
    $root = [IO.Path]::GetFullPath($RepoRoot)
    if (-not $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        throw "installer tree $dir escapes repository root $root"
    }
}
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage, $dist | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $stage "remcord.exe")
Copy-Item -LiteralPath (Join-Path $RepoRoot "license.txt") -Destination (Join-Path $stage "license.txt")
$staged = @(Get-ChildItem -LiteralPath $stage -File | Select-Object -ExpandProperty Name | Sort-Object)
if (($staged -join ",") -ne "license.txt,remcord.exe") {
    throw "staging must contain exactly remcord.exe + license.txt, found: $($staged -join ',')"
}
Write-Host "staging OK: $($staged -join ', ')"

$iss = Join-Path $installerDir "remcord.iss"
& $iscc.Source "/DAppVersion=$version" "/O$dist" $iss
if ($LASTEXITCODE -ne 0) { throw "ISCC compilation failed" }
$setup = Get-ChildItem -LiteralPath $dist -Filter "Remcord-Setup-$version.exe" | Select-Object -First 1
if ($null -eq $setup) { throw "expected setup artifact not produced in $dist" }
$hash = (Get-FileHash -LiteralPath $setup.FullName -Algorithm SHA256).Hash
Write-Host "artifact: $($setup.FullName)"
Write-Host "sha256: $hash"
[ordered]@{ Artifact = $setup.FullName; SHA256 = $hash; Version = $version }
