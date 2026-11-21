#!/bin/sh
# Amostras de janela (`samples.jsonl`) que calibram tokens × % sobre os transcripts da fixture `hist`.
set -eu
dir="$HOME/.claude/hades/limits"
mkdir -p "$dir"
cat > "$dir/samples.jsonl" <<'JSON'
{"at":"2026-11-01T00:00:00.000Z","five":{"pct":5,"resetsAt":1793500000},"seven":{"pct":10,"resetsAt":1794000000}}
{"at":"2026-11-01T00:10:00.000Z","five":{"pct":12,"resetsAt":1793500000},"seven":{"pct":12,"resetsAt":1794000000}}
{"at":"2026-11-02T00:00:00.000Z","five":{"pct":3,"resetsAt":1793600000},"seven":{"pct":14,"resetsAt":1794000000}}
{"at":"2026-11-02T00:30:00.000Z","five":{"pct":15,"resetsAt":1793600000},"seven":{"pct":18,"resetsAt":1794000000}}
{"at":"2026-11-03T00:00:00.000Z","five":{"pct":2,"resetsAt":1793700000},"seven":{"pct":19,"resetsAt":1794000000}}
{"at":"2026-11-03T00:10:00.000Z","five":{"pct":9,"resetsAt":1793700000},"seven":{"pct":22,"resetsAt":1794000000}}
{"at":"2026-11-20T11:00:00.000Z","five":{"pct":40,"resetsAt":1795190000},"seven":{"pct":55,"resetsAt":1795500000}}
JSON
