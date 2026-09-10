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

`request.bodyHex()` (commit `34e38005`) is the stopgap for a byte-exact
HTTP request body. `new Uint8Array([...])` still lowers through the
dynamic host (`constructDynamicValue`, a QuickJS `JsValue`).

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

### Phase 2 -- string <-> bytes

- Two thaw-hir intrinsics + thaw-std impls, operating on the native
  layout (Phase 1) so no QuickJS:
  - `__thaw_bytes_from_string(str, encoding) -> Bytes`
  - `__thaw_bytes_to_string(bytes, encoding) -> Str`
  - encodings: `utf8` (default, lossy on decode), `hex`, `base64`,
    `latin1`.
- Surface them as `Buffer.from(x, enc?)` / `buf.toString(enc?)`:
  - `Buffer.from(str, enc?)` -> `__thaw_bytes_from_string`
  - `Buffer.from(numberArray)` -> clamp each element to `u8` (a small
    codegen loop or a `__thaw_bytes_from_number_array` helper)
  - `buf.toString(enc?)` -> `__thaw_bytes_to_string`
  - `Buffer.alloc(n)` -> zero-filled `Bytes` of length `n`
  - `Buffer.concat(list)` -> flatten (reuse array concat)
- `.d.ts` for `node:buffer` / the ambient `Buffer` class: real signatures
  returning `Bytes`, not `any`.
- Test: `Buffer.from("héllo").toString()` round-trips; `hex` and
  `base64` round-trip; `Buffer.from([256, -1, 65])` clamps to
  `[0, 255, 65]`.

### Phase 3 -- thaw-std produces/consumes Bytes

- An ABI for a thaw-std function to return a native array. Options, in
  order of preference:
  1. reuse the FFI array-return path (`crates/thaw-llvm/src/hir_codegen/
     ffi_calls.rs`, `ffi_array_alloc`) -- if a std `.d.ts` signature
     returning `Bytes` / `number[]` already routes through it, this is
     free;
  2. a `#[no_mangle] extern "C"` pair `__thaw_native_bytes_alloc(len) ->
     *mut u8` / `__thaw_native_bytes_finish(ptr, len) -> <native array>`
     that builds the codegen layout from Rust.
- `node:http`: `request.bodyBytes(): Buffer` returns the raw body as
  `Bytes` (the http `RequestContext` already keeps `_raw_body:
  Vec<u8>`). Optionally make the `request.on("data", cb)` chunk `Bytes`
  instead of a lossy string (a breaking `.d.ts` change to that callback
  signature -- do it with the `callback_param_compatible` width-subtyping
  already in place).
- `response.end` / `write` gain a `Bytes` overload (send the bytes
  verbatim, no `endEncoded("hex")` dance).
- Test: a `00 ff 41 80` body reaches the handler as `Bytes` with those
  four values and echoes back verbatim.

### Phase 4 -- fill in the Buffer surface

`buf.slice(start, end)`, `buf.subarray(...)`, `Buffer.byteLength(str)`,
`buf.equals(other)`, `buf.indexOf(...)`, `buf.copy(...)`, and the
numeric accessors (`readUInt8` / `readUInt16LE` / `readUInt32BE` /
`readBigUInt64BE` / the `write*` mirror). Each is a small builtin over
the native layout. Add as real code needs them, not speculatively.

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
