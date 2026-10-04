$ErrorActionPreference = "Stop"

Push-Location $PSScriptRoot
try {
    cargo run --manifest-path (Join-Path $PSScriptRoot "Cargo.toml")
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    Pop-Location
}
