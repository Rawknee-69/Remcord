# Builds the native Legcord Windows installer (release blocker 3).
#
# Reproducible from a clean checkout plus one documented prerequisite:
#   winget install --id JRSoftware.InnoSetup --exact   (Inno Setup 6, for ISCC.exe)
# Fails fast with that exact instruction when ISCC.exe is not on PATH.
# Never downloads tools silently, never requires administrator rights,
# never touches production Electron paths (asserted before compiling).
#
# Steps: read product version from package.json -> cargo build --release ->
# verify exe identity/resources -> stage exactly {legcord.exe, license.txt}
# -> ISCC native/installer/legcord.iss -> SHA-256 the setup artifact.
# No signing (separate blocker).
param(
    [string] $RepoRoot = "E:/Legcord"
)
$ErrorActionPreference = "Stop"
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
$exe = Join-Path $native "target/release/legcord.exe"
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "release exe missing: $exe" }
$info = (Get-Item -LiteralPath $exe).VersionInfo
if ($info.ProductVersion -ne $version) { throw "exe ProductVersion $($info.ProductVersion) != package.json $version" }
if ($info.ProductName -ne "Legcord") { throw "exe ProductName must be Legcord" }
if ($info.OriginalFilename -ne "legcord.exe") { throw "exe OriginalFilename must be legcord.exe" }
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Icon]::ExtractAssociatedIcon($exe)
if ($null -eq $icon -or $icon.Width -le 0) { throw "release exe carries no icon resource" }
Write-Host "exe identity OK: legcord.exe $version (icon $($icon.Width)x$($icon.Height))"

# Production-path safety: the staging and output trees must stay inside the
# repo and far from every Electron-managed location.
$electronDirs = @(
    (Join-Path $env:LOCALAPPDATA "Programs\Legcord"),
    (Join-Path $env:APPDATA "legcord")
)
foreach ($dir in @($stage, $dist)) {
    foreach ($prod in $electronDirs) {
        if ($dir.StartsWith($prod, [StringComparison]::OrdinalIgnoreCase)) {
            throw "installer tree $dir overlaps production path $prod"
        }
    }
}
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage, $dist | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $stage "legcord.exe")
Copy-Item -LiteralPath (Join-Path $RepoRoot "license.txt") -Destination (Join-Path $stage "license.txt")
$staged = @(Get-ChildItem -LiteralPath $stage -File | Select-Object -ExpandProperty Name | Sort-Object)
if (($staged -join ",") -ne "legcord.exe,license.txt") {
    throw "staging must contain exactly legcord.exe + license.txt, found: $($staged -join ',')"
}
Write-Host "staging OK: $($staged -join ', ')"

$iss = Join-Path $installerDir "legcord.iss"
& $iscc.Source "/DAppVersion=$version" "/O$dist" $iss
if ($LASTEXITCODE -ne 0) { throw "ISCC compilation failed" }
$setup = Get-ChildItem -LiteralPath $dist -Filter "LegcordNative-Setup-$version.exe" | Select-Object -First 1
if ($null -eq $setup) { throw "expected setup artifact not produced in $dist" }
$hash = (Get-FileHash -LiteralPath $setup.FullName -Algorithm SHA256).Hash
Write-Host "artifact: $($setup.FullName)"
Write-Host "sha256: $hash"
[ordered]@{ Artifact = $setup.FullName; SHA256 = $hash; Version = $version }
