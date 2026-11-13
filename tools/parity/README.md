# tools/parity

`node_globals.json` is Node 22's own global object, sorted, as `[name, typeof]` pairs.
`crates/thaw-hir/src/lower/platform_globals.rs` is checked against it by unit tests.

Regenerate (Node 22; the output is single-line JSON):

    node -e "console.log(JSON.stringify(Object.getOwnPropertyNames(globalThis).map(n=>[n, typeof globalThis[n]]).sort()))" > tools/parity/node_globals.json

The raw output includes CommonJS-wrapper bindings and REPL/`-e` builtin-module
names (`require`, `fs`, `util`, ...). Those are not script globals; they are
listed in `NODE_ONLY_NOT_GLOBAL` in `platform_globals.rs`. A new Node release
that adds a global fails `every_node_global_is_classified` until it is added to
`PLATFORM_GLOBALS` or documented in one of the exception lists.
