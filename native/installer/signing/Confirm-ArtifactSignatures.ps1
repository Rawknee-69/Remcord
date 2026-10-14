# Verifies Authenticode signatures on the native Remcord artifacts (F31).
#
# READ-ONLY: never modifies artifacts, the installer definition, or policy.
# Pairs with Invoke-CodeSigning.ps1 (same directory): sign first, confirm
# here. CI runs this in audit mode on every native-sign.yml run; release
# promotion runs it with -RequireSigned.
#
# Exit 0 with warnings when artifacts are unsigned and signing is not
# required (current default). Throws (non-zero exit) when -RequireSigned is
# set and any required artifact is missing, unsigned, or untrusted - unless
# the local-only escape hatch applies (policy allowUnsignedEnv set to "1"
# outside CI). CI (GITHUB_ACTIONS set) never honors the escape hatch.
param(
    [string] $RepoRoot = "",
    [string] $PolicyPath = "",
    [switch] $RequireSigned
)
$ErrorActionPreference = "Stop"

function Resolve-RepoRoot([string] $hint) {
    if (-not [string]::IsNullOrWhiteSpace($hint)) { return $hint }
    $signing = $PSScriptRoot
    $installerDir = Split-Path -Parent $signing
    $native = Split-Path -Parent $installerDir
    return (Split-Path -Parent $native)
}

$RepoRoot = Resolve-RepoRoot $RepoRoot
if ([string]::IsNullOrWhiteSpace($PolicyPath)) {
    $PolicyPath = Join-Path $PSScriptRoot "signing.json"
}
$policy = Get-Content -Raw -LiteralPath $PolicyPath | ConvertFrom-Json
$package = Get-Content -Raw -LiteralPath (Join-Path $RepoRoot "package.json") | ConvertFrom-Json
$version = $package.version
if ([string]::IsNullOrWhiteSpace($version)) { throw "package.json must define a version" }

$require = [bool] $RequireSigned -or [bool] $policy.releaseRequiresSignature
$escapeEnv = $policy.allowUnsignedEnv
$escape = (([Environment]::GetEnvironmentVariable($escapeEnv) -eq "1") -and
    [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable("GITHUB_ACTIONS")))
if ($escape -and $require) {
    Write-Warning "escape hatch ${escapeEnv}=1 honored (local run only); treating unsigned artifacts as allowed"
    $require = $false
}

$failures = @()
$checked = 0
foreach ($artifact in $policy.artifacts) {
    $relative = $artifact.path.Replace("{version}", $version)
    $full = $relative
    if (-not [IO.Path]::IsPathRooted($full)) { $full = Join-Path $RepoRoot $relative }
    $full = [IO.Path]::GetFullPath($full)
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
        $message = "artifact not present: $relative (run build-installer.ps1 first)"
        if ($require -and $artifact.required) { $failures += $message }
        Write-Warning $message
        continue
    }
    $checked += 1
    $signature = Get-AuthenticodeSignature -LiteralPath $full
    $subject = $null
    if ($null -ne $signature.SignerCertificate) { $subject = $signature.SignerCertificate.Subject }
    $timestamped = $false
    if ($null -ne $signature.TimeStamperCertificate) { $timestamped = $true }
    Write-Host "artifact: $relative"
    Write-Host "  status: $($signature.Status)"
    Write-Host "  subject: $subject"
    Write-Host "  timestamped: $timestamped"
    if ($signature.Status -ne "Valid") {
        $message = "$relative signature status is $($signature.Status) (subject: $subject)"
        if ($require) { $failures += $message }
        else { Write-Warning "$message - allowed while signing is opt-in" }
        continue
    }
    $allow = $policy.providers.pfx.subjectAllowlist
    if ($null -ne $allow -and $allow.Count -gt 0 -and ($allow -notcontains $subject)) {
        $message = "$relative signed by unexpected subject: $subject"
        if ($require) { $failures += $message }
        else { Write-Warning $message }
    }
    if (-not $timestamped) {
        $message = "$relative carries no RFC3161 timestamp"
        if ($require) { $failures += $message }
        else { Write-Warning $message }
    }
}

if ($checked -eq 0) {
    Write-Warning "no artifacts present to confirm (build the installer first)"
}
if ($failures.Count -gt 0) { throw "signature gate failed: $($failures -join '; ')" }
Write-Host "signature confirmation passed (checked: $checked, required: $require)"
