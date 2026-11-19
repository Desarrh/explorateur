# Met à jour l'Explorateur : compile, ferme l'app ouverte, remplace l'exe, relance.
# Usage : powershell -ExecutionPolicy Bypass -File scripts\maj.ps1   (ou clic droit > Exécuter avec PowerShell)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Compilation (release)..." -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { Write-Host "La compilation a échoué : rien n'a été remplacé." -ForegroundColor Red; exit 1 }

$dist = Join-Path $root 'dist\Explorateur.exe'
$new  = Join-Path $root 'target\release\explorateur.exe'

# Fermer proprement (les réglages sont enregistrés à la fermeture), puis forcer si besoin.
$running = Get-Process explorateur -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $dist }
$wasOpen = [bool]$running
foreach ($p in $running) { [void]$p.CloseMainWindow() }
if ($running) {
    $running | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue
    Get-Process explorateur -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $dist } | Stop-Process -Force
}

New-Item -ItemType Directory -Force (Split-Path $dist) | Out-Null
Copy-Item $new $dist -Force
Write-Host "Explorateur mis à jour : $dist" -ForegroundColor Green

if ($wasOpen) { Start-Process $dist }
