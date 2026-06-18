#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT

cd "$ROOT"

cargo test -p xlstatus-tsdb
cargo run -p xtask -- mock-agents \
  --count 100 \
  --interval 3s \
  --duration 24h \
  --dry-run \
  --output "$OUT" >/dev/null

python3 - "$OUT" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    report = json.load(f)

assert report["count"] == 100, report
assert report["interval_ms"] == 3000, report
assert report["duration_ms"] == 86400000, report
assert report["dry_run"] is True, report
assert report["total_samples"] == 2880000, report
assert len(report["agents"]) == 100, report
assert all(agent["written"] == 28800 for agent in report["agents"]), report
PY

cargo run -p xtask -- query-bench \
  --period 1d,7d,30d \
  --agents 4 \
  --samples 10 \
  --p95-target-ms 500 >/dev/null

cargo run -p xtask -- tsdb-health --json >/dev/null

echo "M8 TSDB facade and mock-agent load tooling verified"
