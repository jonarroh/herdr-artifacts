#!/bin/sh
# Publishes one artifact of every type, then waits for your answer.
# Open the Artifacts pane in Herdr first, then run:  sh examples/demo.sh
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HA="$ROOT/target/release/herdr-artifacts"
FX="$ROOT/tests/fixtures"
[ -x "$HA" ] || { echo "Build first: cargo build --release" >&2; exit 1; }
export HERDR_ARTIFACTS_AGENT=demo-agent

"$HA" publish qr --id demo-qr --title "Expo QR" --data "exp://192.168.1.100:8081" --description "Escanea para abrir la aplicación"
"$HA" publish image "$ROOT/examples/sample.png" --id demo-image --title "Generated image"
"$HA" publish markdown "$FX/report.md" --id demo-report --title "Architecture Report"
"$HA" publish table "$FX/build.csv" --id demo-table --title "Build steps"
"$HA" publish chart "$FX/timings.json" --id demo-chart --title "Step timings"
"$HA" publish code "$FX/snippet.rs" --id demo-code

# An artifact that changes over time.
"$HA" publish message --id demo-build --title "Build" --body "Compilando…" --status running
sleep 2
"$HA" update demo-build --body "Build completed
12 files generated" --level success --status done

# Ask the user something and wait for the answer.
"$HA" publish json "$FX/approval.json"
echo "Waiting for your answer in the Artifacts pane (60 s)…"
"$HA" wait deploy-approval --timeout 60 || echo "No answer."
