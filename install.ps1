# Installs the latest verifique for Windows into %LOCALAPPDATA%\Programs\verifique, and puts it on
# your PATH. macOS and Linux have install.sh.
#   irm https://raw.githubusercontent.com/smnkgv/verifique/main/install.ps1 | iex
$ErrorActionPreference = "Stop"

$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
$target = if ($env:VERIFIQUE_INSTALL_DIR) { $env:VERIFIQUE_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\verifique" }
New-Item -ItemType Directory -Force $target | Out-Null

$zip = Join-Path ([IO.Path]::GetTempPath()) "verifique-windows-$arch.zip"
Invoke-WebRequest -UseBasicParsing "https://github.com/smnkgv/verifique/releases/latest/download/verifique-windows-$arch.zip" -OutFile $zip

# A running queue holds its program files, so it stops while they are replaced and starts again after.
$running = @(Get-ScheduledTask -TaskName "verifique*" -ErrorAction SilentlyContinue | Where-Object State -eq "Running")
$running | Stop-ScheduledTask
if ($running.Count -gt 0) { Start-Sleep -Seconds 2 }
Expand-Archive -Force $zip $target
Remove-Item $zip
$running | Start-ScheduledTask
Write-Output "installed $(& "$target\verifique.exe" --version) in $target"

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not (($userPath -split ";") -contains $target)) {
    [Environment]::SetEnvironmentVariable("Path", "$target;$userPath", "User")
    $env:Path = "$target;$env:Path"
    Write-Output "Added $target to your PATH. Terminals opened from now on will find verifique."
}

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Output "verifique needs Git for Windows, for git itself and the sh that runs project commands: winget install Git.Git"
}
Write-Output 'Next: start the queue with `verifique service install`, add the databases your projects use, such as `verifique plugin add postgres`, then give your coding agent the output of `verifique init`.'
