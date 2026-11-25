#!/bin/sh
# Measures RAM and CPU of a running Herdr Artifacts viewer (Linux/macOS).
#
# Open the viewer pane first, then run:
#   sh scripts/measure.sh [idle_seconds] [burst]
#
# RSS comes from ps (KiB). CPU time is the cumulative TIME column, so compare
# it between phases: an idle viewer should not advance it measurably.

IDLE="${1:-30}"
BURST="${2:-50}"
EXE="${HERDR_ARTIFACTS_BIN:-$(dirname "$0")/../target/release/herdr-artifacts}"

# Oldest matching process is the viewer; CLI invocations exit in milliseconds.
PID="$(pgrep -o -x herdr-artifacts)"
if [ -z "$PID" ]; then
    echo "No running viewer found. Open the Artifacts pane in Herdr first." >&2
    exit 1
fi

sample() {
    printf '%-24s ' "$1"
    ps -o rss=,time=,nlwp= -p "$PID" 2>/dev/null || ps -o rss=,time= -p "$PID"
}

echo "viewer pid $PID"
printf '%-24s %s\n' "phase" "rss_kib cpu_time threads"
sample "start"
sleep "$IDLE"
sample "idle ${IDLE}s"

i=0
while [ "$i" -lt "$BURST" ]; do
    "$EXE" publish message --id "measure-$i" --title "Measure $i" --body "payload $i" >/dev/null
    i=$((i + 1))
done
sample "burst ($BURST publishes)"

i=0
while [ "$i" -lt "$BURST" ]; do
    "$EXE" remove "measure-$i" >/dev/null
    i=$((i + 1))
done
sample "cleanup"
sleep 10
sample "idle after"
