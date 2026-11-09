# Builds the panel for Windows: target\verifique-panel.exe. Building needs Rust's MSVC build tools, and
# running it needs the WebView2 runtime, which Windows 10 and 11 already have.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$built = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "target" }
$exe = Join-Path $PSScriptRoot "target\verifique-panel.exe"
New-Item -ItemType Directory -Force "target" | Out-Null
try {
    Copy-Item (Join-Path $built "release\verifique-panel.exe") $exe -Force
} catch {
    throw "Quit the panel that's running first: Windows won't replace $exe while it runs."
}
Write-Output "built $exe"
