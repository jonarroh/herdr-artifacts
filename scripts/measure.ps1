# Measures RAM and CPU of a running Herdr Artifacts viewer (Windows).
#
# Open the viewer pane first, then run:
#   pwsh scripts/measure.ps1 [-IdleSeconds 30] [-Burst 50]
#
# Phases:
#   idle   the viewer sits untouched for -IdleSeconds
#   burst  -Burst message artifacts are published over IPC, then removed
# CPU is reported as seconds of CPU time and as a percentage of ONE core.

param(
    [int]$IdleSeconds = 30,
    [int]$Burst = 50,
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\herdr-artifacts.exe')
)

$ErrorActionPreference = 'Stop'

# The viewer is the long-lived process; CLI invocations exit in milliseconds.
$viewer = Get-Process herdr-artifacts -ErrorAction SilentlyContinue | Sort-Object StartTime | Select-Object -First 1
if (-not $viewer) { throw 'No running viewer found. Open the Artifacts pane in Herdr first.' }

function Sample($label, [scriptblock]$work) {
    $p = Get-Process -Id $viewer.Id
    $cpu0 = $p.TotalProcessorTime.TotalSeconds
    $t0 = Get-Date
    & $work
    $p = Get-Process -Id $viewer.Id
    $wall = ((Get-Date) - $t0).TotalSeconds
    $cpu = $p.TotalProcessorTime.TotalSeconds - $cpu0
    [pscustomobject]@{
        Phase        = $label
        WallSeconds  = [math]::Round($wall, 1)
        CpuSeconds   = [math]::Round($cpu, 3)
        CpuPercent   = [math]::Round(100 * $cpu / $wall, 2)
        WorkingSetMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
        PrivateMB    = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        PeakWsMB     = [math]::Round($p.PeakWorkingSet64 / 1MB, 1)
        Threads      = $p.Threads.Count
    }
}

$results = @()
$results += Sample 'idle' { Start-Sleep -Seconds $IdleSeconds }
$results += Sample "burst ($Burst publishes)" {
    for ($i = 0; $i -lt $Burst; $i++) {
        & $Exe publish message --id "measure-$i" --title "Measure $i" --body "payload $i" | Out-Null
    }
}
$results += Sample 'cleanup' {
    for ($i = 0; $i -lt $Burst; $i++) { & $Exe remove "measure-$i" | Out-Null }
}
$results += Sample 'idle after' { Start-Sleep -Seconds ([math]::Min($IdleSeconds, 10)) }

"viewer pid $($viewer.Id), binary $([math]::Round((Get-Item $Exe).Length / 1MB, 2)) MB"
$results | Format-Table -AutoSize
