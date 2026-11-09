#!/bin/bash
# The day-1 / day-7 check after a release: run `grh audit --json` under the
# previous release and the new one in every repository under a root, and diff
# the per-part verdicts after the part-name map. Target: 0 differences.
#
#   scripts/outcome-diff.sh <old grh> <new grh> [root]
#
# The old binary is kept at ~/.cache/grh/0.2.0/grh.
set -eu
old=$(realpath "$1") new=$(realpath "$2") root=${3:-$HOME/Developer/Perso}
rows() { # binary dir
  (cd "$2" && "$1" audit --json 2>/dev/null) |
    python3 -c '
import json,sys
m={"gate":"greengate","guardrails":"greenpiste"}
d=json.load(sys.stdin)
for r in d["parts"]: print(m.get(r["part"],r["part"]), r["verdict"])' | sort
}
diffs=0
for g in "$root"/*/.git; do
  d=$(dirname "$g")
  if ! diff=$(diff <(rows "$old" "$d") <(rows "$new" "$d") 2>&1); then
    diffs=$((diffs + 1)); echo "== $d"; echo "$diff"
  fi
done
echo "repositories with a verdict difference: $diffs"
unknown=$("$new" status 2>&1 | grep -c unknown || true)
echo "unknown in grh status: $unknown"
[ "$diffs" -eq 0 ] && [ "$unknown" -eq 0 ]
