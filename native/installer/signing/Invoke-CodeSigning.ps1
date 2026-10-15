# Signs the native Remcord Windows artifacts with an OV/EV PFX certificate.
#
# POST-BUILD step. Run AFTER native/installer/build-installer.ps1 produced
# the artifacts described in signing.json (same directory). This script never
# edits remcord.iss, build-installer.ps1, or check-installer.ps1 - the
# installer definition stays signing-free by contract (check-installer Sec.7
# asserts no signtool claims in it).
#
# Provider: PFX only (signtool.exe + RFC3161 timestamp).
#   - -PfxPath (explicit local file) or WINDOWS_PFX_BASE64 secret material
#     (written to a temp file, shredded after use). URL sources are refused.
#
# Fail-closed: -Required (or policy releaseRequiresSignature) with no usable
# provider throws. Without -Required, a missing provider is a loud warning
# and exit 0, matching the current unsigned-by-default reality.
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string] $RepoRoot = "",
    [string] $PolicyPath = "",
    [string] $PfxPath = "",
    [switch] $Required
)
$ErrorActionPreference = "Stop"

function Resolve-RepoRoot([string] $hint) {
    if (-not [string]::IsNullOrWhiteSpace($hint)) { return $hint }
    $signing = $PSScriptRoot
    $installerDir = Split-Path -Parent $signing
    $native = Split-Path -Parent $installerDir
    return (Split-Path -Parent $native)
}

function Find-Signtool([string] $minimum) {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($null -ne $onPath) { return $onPath.Source }
    $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits/10/bin"
    if (Test-Path -LiteralPath $kits -PathType Container) {
        $best = Get-ChildItem -LiteralPath $kits -Directory |
            Where-Object { $_.Name -match "^10\." } |
            Sort-Object Name -Descending |
            Select-Object -First 1
        if ($null -ne $best) {
            $candidate = Join-Path $best.FullName "x64/signtool.exe"
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
        }
    }
    return $null
}

function Expand-ArtifactPath([string] $template, [string] $version, [string] $repoRoot) {
    $expanded = $template.Replace("{version}", $version)
    $full = $expanded
    if (-not [IO.Path]::IsPathRooted($full)) { $full = Join-Path $repoRoot $expanded }
    return ([IO.Path]::GetFullPath($full))
}

$RepoRoot = Resolve-RepoRoot $RepoRoot
if ([string]::IsNullOrWhiteSpace($PolicyPath)) {
    $PolicyPath = Join-Path $PSScriptRoot "signing.json"
}
$policy = Get-Content -Raw -LiteralPath $PolicyPath | ConvertFrom-Json
$package = Get-Content -Raw -LiteralPath (Join-Path $RepoRoot "package.json") | ConvertFrom-Json
$version = $package.version
if ([string]::IsNullOrWhiteSpace($version)) { throw "package.json must define a version" }

if ($policy.digestAlgorithm -ne "SHA256") {
    throw "policy digestAlgorithm must be SHA256, found: $($policy.digestAlgorithm)"
}
$tsUri = [Uri] $policy.timestampServer
if ($tsUri.Scheme -ne "https") { throw "policy timestampServer must be https, found: $($policy.timestampServer)" }
if ($policy.allowedTimestampHosts -notcontains $tsUri.Host) {
    throw "timestamp host $($tsUri.Host) is not in policy allowedTimestampHosts"
}

$require = [bool] $Required -or [bool] $policy.releaseRequiresSignature

$targets = @()
foreach ($artifact in $policy.artifacts) {
    $full = Expand-ArtifactPath $artifact.path $version $RepoRoot
    $root = [IO.Path]::GetFullPath($RepoRoot)
    if (-not $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        throw "artifact path escapes repository root: $($artifact.path)"
    }
    if (Test-Path -LiteralPath $full -PathType Leaf) {
        $targets += $full
    } elseif ($artifact.required) {
        Write-Warning "required artifact not built yet, skipping: $full (run build-installer.ps1 first)"
    }
}
if ($targets.Count -eq 0) {
    $message = "no signing targets present (build the installer first)"
    if ($require) { throw $message }
    Write-Warning $message
    return
}

if (-not [string]::IsNullOrWhiteSpace($PfxPath)) {
    if ($PfxPath -match "^https?://") { throw "refusing URL PFX source: $PfxPath (local file or secret only)" }
    if (-not (Test-Path -LiteralPath $PfxPath -PathType Leaf)) { throw "PFX file not found: $PfxPath" }
}

$pfxBase64 = [Environment]::GetEnvironmentVariable($policy.providers.pfx.base64Env)
$pfxPassword = [Environment]::GetEnvironmentVariable($policy.providers.pfx.passwordEnv)
$pfxReady = (-not [string]::IsNullOrWhiteSpace($PfxPath)) -or (-not [string]::IsNullOrWhiteSpace($pfxBase64))

if (-not $pfxReady) {
    $message = "no signing provider configured (set $($policy.providers.pfx.base64Env) / -PfxPath)"
    if ($require) { throw $message }
    Write-Warning "$message - leaving artifacts unsigned (current default; see docs/NATIVE_CODE_SIGNING.md)"
    return
}

$signtool = Find-Signtool $policy.signtoolMinimum
if ($null -eq $signtool) {
    $message = "signtool.exe not found (install Windows SDK 10+, or use the windows-latest runner which carries it)"
    if ($require) { throw $message }
    Write-Warning "$message - leaving artifacts unsigned"
    return
}

$tempPfx = $null
try {
    if ([string]::IsNullOrWhiteSpace($PfxPath)) {
        $tempPfx = Join-Path ([IO.Path]::GetTempPath()) ("remcord-sign-" + [Guid]::NewGuid().ToString("N") + ".pfx")
        [IO.File]::WriteAllBytes($tempPfx, [Convert]::FromBase64String($pfxBase64.Trim()))
        $PfxPath = $tempPfx
    }
    foreach ($target in $targets) {
        $allow = @()
        foreach ($subject in $policy.providers.pfx.subjectAllowlist) { $allow += $subject }
        $signArgs = @("sign", "/fd", "sha256", "/tr", $policy.timestampServer, "/td", "sha256", "/f", $PfxPath)
        if (-not [string]::IsNullOrWhiteSpace($pfxPassword)) { $signArgs += @("/p", $pfxPassword) }
        if ($allow.Count -gt 0) { $signArgs += @("/n", $allow[0]) }
        $signArgs += @($target)
        if ($PSCmdlet.ShouldProcess($target, "signtool sign (SHA256 + RFC3161 timestamp)")) {
            & $signtool @signArgs
            if ($LASTEXITCODE -ne 0) { throw "signtool failed for $target" }
        }
        Write-Host "signed: $target (provider: pfx)"
    }
} finally {
    if ($null -ne $tempPfx -and (Test-Path -LiteralPath $tempPfx -PathType Leaf)) {
        try {
            $bytes = New-Object byte[] ((Get-Item -LiteralPath $tempPfx).Length)
            [IO.File]::WriteAllBytes($tempPfx, $bytes)
        } catch {
            Write-Warning "could not overwrite temp PFX before deletion: $_"
        }
        Remove-Item -LiteralPath $tempPfx -Force -ErrorAction SilentlyContinue
    }
}
