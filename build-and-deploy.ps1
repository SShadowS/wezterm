param(
    [switch]$Debug
)

$ErrorActionPreference = "Stop"

$env:PATH = "C:\Strawberry\perl\bin;C:\Strawberry\c\bin;$env:PATH"

$InstallDir = "C:\Program Files\WezTerm"
$ShimDir = "$InstallDir\tmux-compat"

if ($Debug) {
    $BuildProfile = "dev"
    $ProfileLabel = "debug"
    $TargetDir = "U:\Git\wezterm\target\debug"
    $CargoFlags = @()
} else {
    $BuildProfile = "release"
    $ProfileLabel = "release"
    $TargetDir = "U:\Git\wezterm\target\release"
    $CargoFlags = @("--release")
}

Write-Host "=== Building wezterm ($ProfileLabel) ===" -ForegroundColor Cyan
Set-Location U:\Git\wezterm
& cargo build @CargoFlags -p wezterm-gui -p wezterm -p wezterm-mux-server -p tmux-compat-shim -p env-shim
if ($LASTEXITCODE -ne 0) { throw "Build failed" }

Write-Host ""
Write-Host "=== Stopping running processes ===" -ForegroundColor Cyan
foreach ($name in @("wezterm-gui", "wezterm", "wezterm-mux-server")) {
    $procs = Get-Process -Name $name -ErrorAction SilentlyContinue
    if ($procs) {
        Write-Host "  Stopping $name..." -ForegroundColor Yellow
        $procs | Stop-Process -Force
        Start-Sleep -Milliseconds 500
    }
}

Write-Host ""
Write-Host "=== Copying to $InstallDir ===" -ForegroundColor Cyan
foreach ($exe in @("wezterm-gui.exe", "wezterm.exe", "wezterm-mux-server.exe")) {
    try {
        Copy-Item "$TargetDir\$exe" "$InstallDir\$exe" -Force
        Write-Host "  Copied $exe" -ForegroundColor Green
    } catch {
        Write-Host "  FAILED to copy $exe (is it running?)" -ForegroundColor Red
    }
}

# Deploy tmux compat shim
Write-Host ""
Write-Host "=== Deploying tmux compat shim to $ShimDir ===" -ForegroundColor Cyan
if (-not (Test-Path $ShimDir)) {
    New-Item -ItemType Directory -Path $ShimDir | Out-Null
    Write-Host "  Created $ShimDir" -ForegroundColor Green
}
try {
    Copy-Item "$TargetDir\tmux.exe" "$ShimDir\tmux.exe" -Force
    Write-Host "  Copied tmux.exe" -ForegroundColor Green
} catch {
    Write-Host "  FAILED to copy tmux.exe" -ForegroundColor Red
}
try {
    Copy-Item "$TargetDir\env.exe" "$ShimDir\env.exe" -Force
    Write-Host "  Copied env.exe" -ForegroundColor Green
} catch {
    Write-Host "  FAILED to copy env.exe" -ForegroundColor Red
}

Write-Host ""
Write-Host "=== Done ($ProfileLabel) ===" -ForegroundColor Cyan
