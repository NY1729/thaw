#!/usr/bin/env bash
# Runs one shard of `cargo test $CARGO_TEST_ARGS`: the tests are listed, every SHARDS-th one (offset SHARD) is
# selected, and only those run (--exact). Extra arguments are passed to the test binaries (e.g. --test-threads=1).
# Same scheme as the engine-tests job in .github/workflows/ci.yml.
set -euo pipefail
: "${CARGO_TEST_ARGS:?}" "${SHARD:?}" "${SHARDS:?}"
# shellcheck disable=SC2086
listing=$(cargo test $CARGO_TEST_ARGS -- --list) || exit 1
mapfile -t tests < <(sed -n 's/: test$//p' <<< "$listing")
if (( ${#tests[@]} == 0 )); then
  echo "No tests were listed for: $CARGO_TEST_ARGS" >&2
  exit 1
fi
selected=()
for index in "${!tests[@]}"; do
  if (( index % SHARDS == SHARD )); then
    selected+=("${tests[$index]}")
  fi
done
echo "Running ${#selected[@]} of ${#tests[@]} tests (shard $SHARD of $SHARDS)"
# shellcheck disable=SC2086
cargo test $CARGO_TEST_ARGS -- --exact "$@" "${selected[@]}"
