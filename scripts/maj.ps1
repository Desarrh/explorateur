# Met à jour l'Explorateur sans déranger l'utilisateur :
# - compile en priorité basse ;
# - si l'exe est en cours d'utilisation, il est renommé en .old (Windows l'autorise) et le nouveau
#   prend sa place : la fenêtre ouverte continue de tourner, la nouvelle version servira au prochain
#   lancement (le .old est supprimé à ce moment-là) ;
# - rien n'est fermé ni relancé, sauf avec -Relancer (autorisé pendant la session de dev, DIRECTION.md §0).
# Usage : powershell -ExecutionPolicy Bypass -File scripts\maj.ps1 [-Relancer]
param([switch]$Relancer)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Compilation (release, priorité basse)..." -ForegroundColor Cyan
$p = Start-Process cargo -ArgumentList 'build', '--release' -NoNewWindow -PassThru
$null = $p.Handle # sans ça, PowerShell perd le code de sortie
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
# Anciens .old qui ne tournent plus : on les supprime (ceux encore utilisés sont verrouillés, on les laisse).
Get-ChildItem (Split-Path $dist) -Filter 'Explorateur*.old' -ErrorAction SilentlyContinue | ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
$copied = $false
try { Copy-Item $new $dist -Force; $copied = $true } catch {}
if (-not $copied) {
    # Exe en cours d'utilisation : on le renomme (nom unique si un .old tourne encore) puis on copie.
    $old = [IO.Path]::ChangeExtension($dist, 'old')
    if (Test-Path $old) { $old = Join-Path (Split-Path $dist) ("Explorateur." + (Get-Date -Format 'yyyyMMdd-HHmmss') + ".old") }
    try { Move-Item $dist $old -Force } catch { Write-Host "Impossible de renommer l'exe en cours : $_" -ForegroundColor Red; exit 1 }
    Copy-Item $new $dist -Force
}
if ($running.Count -gt 0 -and -not $Relancer) {
    Write-Host "Explorateur mis à jour : $dist (la fenêtre ouverte garde l'ancienne version jusqu'à sa fermeture)." -ForegroundColor Green
} else {
    Write-Host "Explorateur mis à jour : $dist" -ForegroundColor Green
}
# Relance via explorer.exe : lancé depuis l'app Claude (empaquetée MSIX), un Start-Process direct
# ferait tourner l'explorateur dans son conteneur (réglages et registre virtualisés, invisibles).
if ($Relancer -and $running.Count -gt 0) { Start-Process explorer.exe -ArgumentList ('"' + $dist + '"') }
