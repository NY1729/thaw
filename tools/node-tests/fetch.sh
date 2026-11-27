#!/bin/bash
# usage: fetch.sh DIR  -- depth-1 sparse fetch of the nodejs/node commit pinned in TEST_COMMIT (tag v22.23.1)
# into DIR: test/parallel, test/common, test/fixtures only. DIR should be outside the repo (CI-cacheable).
set -euo pipefail
dir=${1:?usage: fetch.sh DIR}
sha=$(tr -d '[:space:]' < "$(dirname "$0")/TEST_COMMIT")
if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$sha" ]; then exit 0; fi
mkdir -p "$dir" && cd "$dir"
git init -q . && git remote add origin https://github.com/nodejs/node 2>/dev/null || true
git sparse-checkout set --no-cone /test/parallel /test/common /test/fixtures
git fetch -q --depth 1 --filter=blob:none origin "$sha" && git checkout -q FETCH_HEAD
