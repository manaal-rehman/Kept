# Installs everything needed to build Kept on Windows 10/11 (run in PowerShell).
# Uses winget, which ships with Windows 10 (1809+) and Windows 11.
$ErrorActionPreference = "Stop"
Write-Host "Installing Microsoft C++ Build Tools (needed by Rust)..." -ForegroundColor Cyan
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-package-agreements --accept-source-agreements `
  --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
Write-Host "Installing Rust..." -ForegroundColor Cyan
winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements
Write-Host "Installing Node.js LTS..." -ForegroundColor Cyan
winget install --id OpenJS.NodeJS.LTS -e --accept-package-agreements --accept-source-agreements
Write-Host "Checking for the WebView2 runtime (preinstalled on Windows 11)..." -ForegroundColor Cyan
winget install --id Microsoft.EdgeWebView2Runtime -e --accept-package-agreements --accept-source-agreements 2>$null
Write-Host ""
Write-Host "Done. CLOSE this window and open a NEW PowerShell so the new tools are on PATH, then run:" -ForegroundColor Green
Write-Host "  .\scripts\build-windows.ps1"
