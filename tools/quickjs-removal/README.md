# QuickJS removal tools

## Declaration blocker shapes (`blocker_shapes.py`)

Buckets the thaw-js declaration blockers recorded at build time (`thaw_js_blockers`,
`thaw-js-blocker: package=... why=...` in `thaw inspect` output, or `stays on QuickJS: ...`
under `THAW_JS_RUNTIME_DEBUG=1`) into the callback/class shapes S1-S6 and gate reasons.

    python3 tools/quickjs-removal/blocker_shapes.py [--json] PATH...

PATH can be CI artifact directories or files. A directory holding `name.txt` is a corpus case
(its name is shown, and the row is marked `*`). Run the selftest with:

    python3 -m unittest discover -s tools/quickjs-removal -p 'test_*.py'

Shapes: S1 callback param, S2 callback in an option object, S3 class-typed param/result,
S4 `Promise<...>` / async callback, S5 constructor with several arities, S6 generic with callback,
`other` (unclassified, listed). Reasons: callback, type word, class op, no declaration,
no typed target, bundle scan, other.
