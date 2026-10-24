#!/bin/sh
# Re-record the expected output of every package script with real Node.
# usage: record.sh <projects-dir with <proj>/node_modules> ; project dir names: yup validator jsyaml ajv dayjs
set -e
here=$(cd "$(dirname "$0")" && pwd)
for js in "$here"/*.js; do
  pkg=$(sed -n '1s#^// pkg: ##p' "$js")
  proj=$(echo "$pkg" | tr -d '-')
  (cd "$1/$proj" && TZ=America/New_York NODE_PATH="$PWD/node_modules" node "$js" > "${js%.js}.out" 2>/dev/null) || true
done
