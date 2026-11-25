#!/bin/bash
# usage: fetch.sh DIR  -- depth-1 fetch of the test262 commit pinned in TEST262_COMMIT into DIR.
set -euo pipefail
dir=${1:?usage: fetch.sh DIR}
sha=$(tr -d '[:space:]' < "$(dirname "$0")/TEST262_COMMIT")
if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD)" = "$sha" ]; then exit 0; fi
mkdir -p "$dir" && cd "$dir"
git init -q . && git remote add origin https://github.com/tc39/test262 2>/dev/null || true
git fetch -q --depth 1 origin "$sha" && git checkout -q FETCH_HEAD
