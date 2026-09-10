#!/usr/bin/env bash
# Compares a fresh `run.sh` output against a checked-in per-architecture
# baseline and fails if thaw's own startup time or peak RSS regressed
# beyond a tolerance. Run right after `run.sh` in CI, on the JSON it
# just produced:
#
#   benchmarks/runtime/run.sh target/release/thaw "runtime-${arch}.json"
#   benchmarks/runtime/check_regression.sh "runtime-${arch}.json" \
#     "benchmarks/runtime/baseline-${arch}.json"
#
# Compares thaw's own numbers against thaw's own past numbers (not
# against Node/Bun, which `run.sh` also measures on the same run purely
# for reference) -- a real regression check, not a "is thaw faster than
# Node" check.
#
# Tolerance is the larger of TOLERANCE_PCT% of the baseline value, or a
# fixed floor -- a tiny baseline (thaw's startup time is a couple of
# milliseconds) makes a pure percentage too tight to survive ordinary
# measurement noise, so the floor keeps the gate meaningful without
# being flaky.
#
# If no baseline file exists yet for this architecture (e.g. a new CI
# runner architecture with nothing committed), this passes without
# comparing anything -- run once for real (locally or in CI) and commit
# the resulting JSON as the new baseline instead of leaving one missing.
set -euo pipefail

current=${1:?usage: check_regression.sh <current.json> <baseline.json>}
baseline=${2:?usage: check_regression.sh <current.json> <baseline.json>}

TOLERANCE_PCT=${TOLERANCE_PCT:-25}

if [ ! -f "$baseline" ]; then
  echo "no baseline at $baseline yet -- skipping regression check (seed it by committing this run's JSON)"
  exit 0
fi

check_metric() {
  metric=$1
  floor=$2
  current_value=$(jq -r ".thaw.${metric}" "$current")
  baseline_value=$(jq -r ".thaw.${metric}" "$baseline")
  allowed=$(awk -v base="$baseline_value" -v pct="$TOLERANCE_PCT" -v floor="$floor" \
    'BEGIN { margin = base * pct / 100; if (margin < floor) margin = floor; printf "%.4f", base + margin }')
  over=$(awk -v cur="$current_value" -v max="$allowed" 'BEGIN { print (cur > max) ? "1" : "0" }')
  if [ "$over" = "1" ]; then
    echo "REGRESSION: thaw.${metric} is ${current_value} (baseline ${baseline_value}, allowed up to ${allowed}, +${TOLERANCE_PCT}% or floor ${floor})"
    return 1
  fi
  echo "ok: thaw.${metric} is ${current_value} (baseline ${baseline_value}, allowed up to ${allowed})"
  return 0
}

failed=0
check_metric "startup_ms" 5 || failed=1
check_metric "max_rss_kb" 512 || failed=1

if [ "$failed" -ne 0 ]; then
  echo "runtime regression check failed against $baseline"
  exit 1
fi
echo "runtime regression check passed against $baseline"
