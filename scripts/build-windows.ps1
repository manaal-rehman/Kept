# Builds the Kept Windows installer. Run from the project folder in a new PowerShell window.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
foreach ($tool in "cargo", "npm") {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "$tool not found. Run scripts\setup-windows.ps1 first, then open a new PowerShell." }
}
Write-Host "1/3 Installing JavaScript packages..." -ForegroundColor Cyan
npm ci
Write-Host "2/3 Running the automated tests..." -ForegroundColor Cyan
cargo test -p kept-core
if ($LASTEXITCODE -ne 0) { throw "Tests failed - not building." }
Write-Host "3/3 Building the installer (first build takes 5-15 minutes)..." -ForegroundColor Cyan
npm run tauri build -- --bundles nsis,msi
if ($LASTEXITCODE -ne 0) { throw "Build failed." }
$exe = Get-ChildItem target\release\bundle\nsis\*.exe | Select-Object -First 1
Write-Host ""
Write-Host "Installer ready:" -ForegroundColor Green
Write-Host "  $($exe.FullName)"
Write-Host "  (MSI: target\release\bundle\msi\)"
explorer.exe /select,"$($exe.FullName)"
