#!/usr/bin/env bash
# Selftest for run.py: a fake `thaw` (builds a wrapper around node) must yield MATCH, DIFF, RUN-FAIL, BUILD-FAIL.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/app-corpus-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
mkdir "$T/programs"
for n in ok diff runfail buildfail; do echo "console.log('hi $n'); await (await import('node:fs/promises')).writeFile('o.txt','x');" > "$T/programs/$n.mjs"; done
cat > "$T/thaw" <<'FAKE'
#!/bin/sh
# usage: thaw build <prog> -o <out>
prog=$2; out=$4; name=${prog%.*}
case $name in buildfail) echo "error: unsupported on thaw-js: method <expression>.end expects at least 1 argument(s), got 0" >&2; exit 1;; esac
{ echo '#!/bin/sh'
  case $name in
    diff) echo "node $prog; echo extra";;
    runfail) echo "echo 'Uncaught opaque exception' >&2; exit 1";;
    *) echo "exec node $prog";;
  esac; } > "$out"
chmod +x "$out"
FAKE
chmod +x "$T/thaw"
python3 -I "$HERE/run.py" --programs "$T/programs" --thaw "$T/thaw" --out "$T/out" --known /nonexistent > "$T/log" 2>&1 || { cat "$T/log"; exit 1; }
for want in "^MATCH +ok.mjs" "^DIFF +diff.mjs" "^RUN-FAIL +runfail.mjs +Uncaught opaque exception" "^BUILD-FAIL +buildfail.mjs +error: unsupported"; do
  grep -qE "$want" "$T/log" || { echo "selftest FAIL: missing '$want'"; cat "$T/log"; exit 1; }
done
grep -q "BUILD-FAIL count: 1" "$T/out/summary.txt" || { echo "selftest FAIL: summary"; exit 1; }
echo "selftest ok"
