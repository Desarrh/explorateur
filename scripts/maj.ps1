# Met à jour l'Explorateur sans déranger l'utilisateur :
# - compile en priorité basse ;
# - si l'exe est en cours d'utilisation, il est renommé en .old (Windows l'autorise) et le nouveau
#   prend sa place : la fenêtre ouverte continue de tourner, la nouvelle version servira au prochain
#   lancement (le .old est supprimé à ce moment-là) ;
# - rien n'est fermé ni relancé, sauf avec -Relancer (à n'utiliser qu'avec l'accord de l'utilisateur).
# Usage : powershell -ExecutionPolicy Bypass -File scripts\maj.ps1 [-Relancer]
param([switch]$Relancer)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Compilation (release, priorité basse)..." -ForegroundColor Cyan
$p = Start-Process cargo -ArgumentList 'build', '--release' -NoNewWindow -PassThru
try { $p.PriorityClass = 'BelowNormal' } catch {}
$p.WaitForExit()
if ($p.ExitCode -ne 0) { Write-Host "La compilation a échoué : rien n'a été remplacé." -ForegroundColor Red; exit 1 }

$dist = Join-Path $root 'dist\Explorateur.exe'
$new  = Join-Path $root 'target\release\explorateur.exe'
New-Item -ItemType Directory -Force (Split-Path $dist) | Out-Null

$running = @(Get-Process explorateur -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $dist })
if ($running.Count -gt 0 -and $Relancer) {
    # Fermeture propre (réglages enregistrés), seulement si on l'a explicitement demandé.
    foreach ($r in $running) { [void]$r.CloseMainWindow() }
    $running | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue
}
if (Test-Path $dist) {
    $old = [IO.Path]::ChangeExtension($dist, 'old')
    Remove-Item $old -Force -ErrorAction SilentlyContinue
    try { Move-Item $dist $old -Force } catch { Write-Host "Impossible de renommer l'exe en cours : $_" -ForegroundColor Red; exit 1 }
}
Copy-Item $new $dist -Force
if ($running.Count -gt 0 -and -not $Relancer) {
    Write-Host "Explorateur mis à jour : $dist (la fenêtre ouverte garde l'ancienne version jusqu'à sa fermeture)." -ForegroundColor Green
} else {
    Write-Host "Explorateur mis à jour : $dist" -ForegroundColor Green
}
if ($Relancer -and $running.Count -gt 0) { Start-Process $dist }
