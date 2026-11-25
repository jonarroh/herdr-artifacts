# Publishes one artifact of every type, then waits for your answer (Windows).
# Open the Artifacts pane in Herdr first, then run:  pwsh examples/demo.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$ha = Join-Path $root 'target\release\herdr-artifacts.exe'
$fx = Join-Path $root 'tests\fixtures'
if (-not (Test-Path $ha)) { throw 'Build first: cargo build --release' }
$env:HERDR_ARTIFACTS_AGENT = 'demo-agent'

& $ha publish qr --id demo-qr --title 'Expo QR' --data 'exp://192.168.1.100:8081' --description 'Escanea para abrir la aplicación'
& $ha publish image (Join-Path $root 'examples\sample.png') --id demo-image --title 'Generated image'
& $ha publish markdown (Join-Path $fx 'report.md') --id demo-report --title 'Architecture Report'
& $ha publish table (Join-Path $fx 'build.csv') --id demo-table --title 'Build steps'
& $ha publish chart (Join-Path $fx 'timings.json') --id demo-chart --title 'Step timings'
& $ha publish code (Join-Path $fx 'snippet.rs') --id demo-code

# An artifact that changes over time.
& $ha publish message --id demo-build --title 'Build' --body 'Compilando…' --status running
Start-Sleep -Seconds 2
& $ha update demo-build --body "Build completed`n12 files generated" --level success --status done

# Ask the user something and wait for the answer.
& $ha publish json (Join-Path $fx 'approval.json')
Write-Host 'Waiting for your answer in the Artifacts pane (60 s)…'
& $ha wait deploy-approval --timeout 60
if ($LASTEXITCODE -eq 2) { Write-Host 'No answer.' }
