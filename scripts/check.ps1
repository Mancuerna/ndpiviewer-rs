$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
    cargo clippy --locked --all-targets --all-features -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy check failed.' }
    cargo test --release --locked --all-targets -j 1
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
} finally {
    Pop-Location
}
