# Byte buffers (`Buffer` / `Uint8Array`)

## Status

**Phase 1 done (`e6ed1dd9`).** `HirType::Bytes` is a real distinct type
during lowering -- `Buffer` / `Uint8Array` annotations resolve to it,
`expect_type` / `coerce_to_declared` interchange it with `Array(F64)`
both directions, `infer_expr_type` surfaces it as `Array(F64)` for
indexing / `.length` / iteration / spread / array methods -- and
`lower/bytes_erasure.rs` walks the whole lowered `HirProgram` rewriting
every `Bytes` to `Array(F64)` before it leaves `thaw_hir::lower`.
Codegen never sees `Bytes`. `buf.toString("utf8")` still comma-joins
(Phase 2).

**Phase 2 done (`67ed65d3`).** `Buffer.from(str, enc?)` / `Buffer.alloc(n)`
/ `buf.toString(enc?)` -- `utf8` (default), `hex`, `base64`/`base64url`,
`latin1` -- all on the native array layout via
`thaw_bytes_*` (`thaw-runtime/.../native_values/bytes.rs`). So
`Buffer.from(request.bodyHex(), "hex")` round-trips a byte-exact HTTP
body today. Follow-up: `Buffer.from(number[])` now copies through
`thaw_bytes_from_array`, clamping each element to a byte (`300` -> `44`,
`-1` -> `255`) and detaching the copy from the source array; `Buffer.concat(list)`
(`thaw_bytes_concat`, 1-arg form) flattens an array of byte buffers.

**Phase 3 done.** `node:http` produces and consumes the native byte
array directly: `request.bodyBytes()` hands the handler the raw request
body as a first-class array it indexes / iterates / `.length`s with no
hex detour, and `response.writeBytes(...)` / `response.endBytes(...)`
send an arbitrary byte sequence back verbatim (same streaming /
buffering path as the string `write`/`end`, only the payload is decoded
from an `Array(F64)` handle instead of a NUL-terminated C string). All
three go through thaw-std's existing native-closure property mechanism
(`crates/thaw-std/src/http.rs`, `native_bytes_to_vec` /
`native_bytes_from_slice` build and read the `[len: i64][f64 * len]`
layout `compile_array_wrap` assumes) -- no new codegen ABI. A
`00 ff 41 80` body round-trips exactly, handler-computed length and
checksum proving each byte value arrived intact
(`node_http_round_trips_a_binary_body_as_first_class_bytes`).

Not done in Phase 3: the `node:http` `.d.ts` types these as `number[]`,
not `Uint8Array` -- the bridge's `.d.ts` parser doesn't resolve
`Uint8Array` in a native-builtin interface member position (it drops the
whole interface, taking `bodyHex`/`on` with it), so the return of
`request.bodyBytes()` is a plain `number[]` and `.toString("hex")` /
`.slice(...)` don't dispatch on it as `Bytes` yet. Wrap it with
`Buffer.from(request.bodyBytes())` for that. `request.on("data")` chunks
are still lossy strings.

`new Uint8Array([...])` still lowers through the dynamic host
(`constructDynamicValue`, a QuickJS `JsValue`).

## Goal

A native byte container that:

- compiled TS can hold, index (`buf[i]` -> `number` 0..255), take
  `.length` of, iterate, slice, and pass around without touching QuickJS;
- thaw-std can build (an HTTP request body) and read (a response body);
- interoperates with `string` through explicit encode/decode
  (`utf8` / `hex` / `base64` / `latin1`), never implicitly.

Non-goals for the first pass: the full Node `Buffer` numeric-accessor
surface (`readUInt32BE` &c.), `Buffer` pooling, resizable buffers,
`SharedArrayBuffer`.

## Representation

`Buffer` gets the **same physical layout as `Array(F64)`** -- codegen's
`[len: i64][payload: f64 * len]` heap block -- but a **distinct
`HirType::Bytes` identity** so method/builtin dispatch can tell a byte
buffer from a plain `number[]`. Every codegen site that handles
`Array(F64)` (alloc, index load/store, `.length`, iteration, spread,
equality, printing, `JSON.stringify`) treats `Bytes` identically -- in
most match arms this is `HirType::Array(inner) if **inner == F64** |
HirType::Bytes => ...` or a shared helper.

Tradeoff: 8 bytes of storage per logical byte. Acceptable for the target
use (bodies up to a few hundred KB, tokens, headers, framing). A large
body is still better served by `bodyHex()` (2 chars/byte) until a packed
`u8` payload variant is worth the second layout.

## Phases

### Phase 1 -- the type ✅ (`e6ed1dd9`)

- `HirType::Bytes` variant.
- `lower_ts_type`: bare `Buffer` / `Uint8Array` -> `HirType::Bytes`
  (after the `__thaw_`-prefix -> `JsValue` check, so a rewritten
  external `Buffer` stays `JsValue`).
- `expect_type` / `coerce_to_declared`: `Bytes` <-> `Array(F64)` both
  directions. `infer_expr_type` normalizes `Bytes` -> `Array(F64)` for
  every consumer (a wrapper over `infer_expr_type_inner`).
- `lower/bytes_erasure.rs`: full `HirProgram` walk, `Bytes` -> `Array(F64)`.
- `native_typeof_name(Bytes)` -> `"object"`.
- Not done: `Array.isArray(buf)` still `false` (matches Node);
  `new Uint8Array([...])` still dynamic-host.

### Phase 2 -- string <-> bytes ✅ (`67ed65d3`)

- `thaw-runtime/.../native_values/bytes.rs`:
  `thaw_bytes_from_string(text, enc) -> [len][f64...]`,
  `thaw_bytes_to_string(buf, enc) -> cstr`, `thaw_bytes_alloc(size)`.
  Encodings: `utf8` (default, lossy decode), `hex`,
  `base64` / `base64url` (hand-rolled, no crate), `latin1` / `binary` /
  `ascii`.
- thaw-hir: `Buffer.from(str, enc?)` / `Buffer.alloc(n)` static
  builtins (infer `Bytes`); `Buffer.from(number[])` passes through;
  `buf.toString(enc?)` on a `Bytes` receiver (via
  `infer_expr_type_inner`) decodes.
- thaw-llvm: decls + `compile_array_call` dispatch (wrap/unwrap the
  array handle).
- Follow-up landed: `Buffer.from(number[])` byte-clamping copy
  (`thaw_bytes_from_array`); `Buffer.concat(list, totalLength?)`
  (`thaw_bytes_concat`, `totalLength` truncates / zero-pads, `-1` for
  "sum of parts").
- Not done: real `node:buffer` `.d.ts` signatures.

### Phase 3 -- thaw-std produces/consumes Bytes ✅

- No new codegen ABI was needed. thaw-std already builds the native
  `[len: i64][f64 * len]` payload + one-word handle from Rust in
  `json.rs` (`wrap_array_handle`); `http.rs` gained the same two
  helpers, `native_bytes_from_slice` / `native_bytes_to_vec`, arena-
  allocated so the value lives as long as any other heap value the
  handler sees. A `bodyBytes` / `writeBytes` / `endBytes` native-closure
  property on `IncomingMessage` / `ServerResponse` (exactly like the
  existing `bodyHex` / `write` closures) returns / accepts that handle;
  the generic function-typed-property call path in thaw-hir/thaw-llvm
  passes it through unmarshalled since an `Array(F64)` value already
  *is* a pointer.
- `request.bodyBytes()` -- the raw request body as a native byte array,
  a fresh handle per call (bodies are bounded by `MAX_REQUEST_BODY`).
- `response.writeBytes(bytes)` / `response.endBytes(bytes)` -- same
  streaming / buffering path as the string `write`/`end`, payload
  decoded from the handle (each element truncated toward zero, mod 256,
  matching `Buffer`'s `ToUint8`), so an arbitrary byte sequence goes out
  verbatim -- no `endEncoded("hex")` dance.
- Test: `node_http_round_trips_a_binary_body_as_first_class_bytes` --
  a `00 ff 41 80` body reaches the handler, which indexes / iterates /
  `.length`s it (length 4, checksum 448 in response headers) and echoes
  it back byte-for-byte through `endBytes`.
- Not done: the `.d.ts` types these `number[]`, not `Uint8Array` (the
  bridge's native-builtin interface parser drops any interface with an
  unresolved member type ref, and `Uint8Array` is one there), so the
  result isn't a dispatch-distinct `Bytes` -- `.toString`/`.slice` on it
  need a `Buffer.from(...)` wrap. `request.on("data")` chunks are still
  lossy strings.

### Phase 4 -- fill in the Buffer surface

Landed so far:

- `buf.slice(start?, end?)` / `buf.subarray(start?, end?)` -- reuse
  `__thaw_array_slice`'s codegen under a `Bytes`-result alias
  (`__thaw_bytes_slice`), so a sliced `Buffer` stays a `Buffer` and a
  chained `.toString("hex")` decodes. `subarray` returns a **copy**, not
  a view (thaw arrays aren't views); it errors on a non-array receiver.
- `Buffer.byteLength(str, enc?)` -- `thaw_bytes_byte_length`, the
  encoded byte count (`utf8` default) for a `Content-Length`.
- `a.equals(b)` -- `thaw_bytes_equals`, byte-for-byte equality, `Bytes`
  receiver only (a plain `number[]` errors -- use `===` / a loop).
- `Buffer.concat`'s optional `totalLength` (truncate / zero-pad).
- The binding-wrapper (`wrap_call_argument_bindings`) now types its
  lambda return with `infer_expr_type_inner`, so a `Bytes`-typed call
  whose args needed hoisting (`buf.slice(i, j)`, `Buffer.from(someVar)`)
  keeps its byte-buffer identity for a chained `.toString`.

Still speculative -- add when real code needs them, not before:
`buf.indexOf(sub)` (the number form already works via `Array.indexOf`;
this is string / sub-buffer search), `buf.copy(...)`, and the numeric
accessors (`readUInt8` / `readUInt16LE` / `readUInt32BE` /
`readBigUInt64BE` / `readInt*` / the `write*` mirror). Each is a small
builtin over the native layout.

## Where to hook (file map)

| concern | file |
| --- | --- |
| `HirType::Bytes` | `crates/thaw-hir/src/hir/types.rs` |
| annotation -> `Bytes` | `crates/thaw-hir/src/lower/type_resolution.rs` |
| assignability | `crates/thaw-hir/src/lower/inference/types.rs` |
| `Buffer.*` / `buf.*` dispatch | `crates/thaw-hir/src/lower/invocations/` |
| array codegen parity | `crates/thaw-llvm/src/hir_codegen/collections.rs`, `operators.rs` |
| encode/decode impls | `crates/thaw-std/src/` (new `bytes.rs`) |
| std array-return ABI | `crates/thaw-llvm/src/hir_codegen/ffi_calls.rs` |
| `node:http` / `node:buffer` `.d.ts` | `crates/thaw-registry/src/registry/resolution.rs` |
| http body bytes | `crates/thaw-std/src/http.rs` |
