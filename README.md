# Thaw

Thaw is an experimental ahead-of-time compiler for a deliberately small,
typed subset of TypeScript. It lowers TypeScript through SWC and Thaw's HIR to
LLVM, then links a native executable. Its primary target is small AWS Lambda
handlers with fast startup, while npm packages are exposed through either a C
ABI fast path or an embedded QuickJS fallback.

Thaw is not yet a drop-in replacement for Node.js or `tsc`. The compatibility
table below is part of the project contract: additions should update both the
table and the relevant design document.

## Quick start

Requirements:

- Rust and Cargo
- LLVM 22 and a C linker available as `cc`
- npm, only when importing packages with `thaw registry add`

Build the compiler and compile a program:

```sh
cargo build --release -p thaw-cli
target/release/thaw build app.ts -o app
./app
```

On Linux, `--static` requests a completely static ELF and verifies that the
result has no dynamic interpreter:

```sh
target/release/thaw build app.ts --static -o app
```

The selected C toolchain must provide `libc.a`, `libm.a`, and `libdl.a`
(for example Fedora's `glibc-static`, or an equivalent musl toolchain).
`--static` rejects `.so`/`.dylib` arguments passed through `--link` and N-API
`.node` addons, which require a dynamic loader; JavaScript fallbacks and
static `native.a` backends remain compatible.

Inspect a produced artifact without running it:

```sh
target/release/thaw inspect app
```

This reports its ELF architecture, static/dynamic system linkage, embedded npm
packages, and whether QuickJS or N-API support is present.

Compile the Lambda example:

```sh
target/release/thaw build examples/hello-lambda/handler.ts -o bootstrap
```

Import and use an npm package through the local registry:

```sh
target/release/thaw registry add left-pad@1.3.0
target/release/thaw build app.ts --use left-pad -o app
```

The default registry is `./thaw_modules`. `--registry <directory>` selects a
different one. `--bridge <file.d.ts>` and `--link <library>` provide the manual
FFI route. Native functions returning an error-aware result struct can opt in
with `--ffi-metadata <file.json>`:

```json
{
  "version": 2,
  "functions": {
    "read_config": {
      "errorAbi": "thaw-result",
      "returnOwnership": "owned",
      "destroy": "free_config"
    }
  }
}
```

For a declared return type `T`, `thaw-result` expects the C function to return
`struct { T value; const char *error; }`. Exactly one channel should be active;
a non-null error propagates through Thaw `try/catch/finally`. Unlisted symbols
keep the legacy direct return ABI.
Version 2 additionally describes string ownership. `borrowed` leaves a pointer
untouched; `owned` copies it into Thaw's request arena and calls the required
`destroy`/`returnDestroy` function exactly once; `arena-copy` performs the copy
and optionally calls a destructor. `errorOwnership` uses the same values and an
optional `errorDestroy`. Version 1 remains accepted and defaults both channels
to `borrowed`.

## Architecture

```text
TypeScript --SWC--> thaw-parser --> thaw-hir --> thaw-llvm --> object file
                                                               |
                  thaw-arena + thaw-std + thaw-runtime --------+--> executable
                  thaw-quickjs -------------------------------+

.d.ts --> thaw-bridge --> C ABI fast path or QuickJS fallback
npm   --> thaw-registry --> package.d.ts + bundle.js + lock.json
```

The workspace crates have narrow responsibilities:

| Crate | Responsibility |
| --- | --- |
| `thaw-cli` | `build` and `registry add`, object generation and final linking |
| `thaw-parser` | TypeScript and JavaScript parsing through SWC |
| `thaw-hir` | Typed intermediate representation and AST lowering |
| `thaw-llvm` | LLVM code generation and native entry points |
| `thaw-arena` | Request-scoped bump allocation used by generated values |
| `thaw-runtime` | AWS Lambda Runtime API loop |
| `thaw-std` | Built-ins such as HTTP fetch and JSON operations |
| `thaw-bridge` | `.d.ts` extraction, ABI classification and shim generation |
| `thaw-quickjs` | JavaScript fallback execution |
| `thaw-registry` | npm acquisition, dependency discovery and JS bundling |

## Compatibility

### Implemented

- Typed top-level functions, calls, forward references and recursion
- `number`, `string`, `boolean`, `void`, `Json`, number arrays and fixed-shape
  objects
- `interface`, interface inheritance and generic interface instantiation
- Local-variable inference from supported expressions
- `let`/`const`, assignment, arithmetic, comparison, `if`, `while` and classic
  `for`
- Object fields and array indexing/mutation
- `throw`, `try/catch` and `finally`, including propagation and rethrow across
  generated Thaw function calls and nested cleanup ordering
- `process.env`, `console.log`, JSON operations, and both legacy blocking and
  Promise-based non-blocking HTTP GET
- Synchronous lowering of `async` functions and `await`
- Ambient declarations and C ABI calls; number arrays become `(pointer,
  length)` and object fields become scalar arguments
- QuickJS fallback for signatures which cannot use the C ABI path
- CommonJS dependency bundling, a limited ESM-to-CommonJS rewrite, selected
  Node built-in polyfills, scoped packages and package version locking
- Relative user-module graphs (`./file`, `./file.ts`, and `./dir/index.ts`)
  with named/default imports, aliases, named re-exports, export-all,
  module-local symbol isolation, dependency deduplication and cycle diagnostics
- Bare imports automatically resolve packages already installed in
  `thaw_modules`, generate their existing native/QuickJS/N-API bridge, and
  support named, default and namespace call syntax without a matching `--use`
- QuickJS and N-API imports with representable `.d.ts` signatures use ordinary
  typed calls: primitive, `Json`, `number[]`, and fixed object arguments are
  marshalled automatically and results are converted back to their declared
  native type; legacy unsupported signatures retain the explicit `Json` ABI
- Registry-managed JavaScript bundles, Rust runtimes, static native archives,
  and N-API addon bytes are embedded into the produced executable. The binary
  can run after `thaw_modules` and its original `.node` files are removed
- Root package `exports` conditions select `types` and `require`/`import` entry
  points during `registry add`, with `types`/`typings` and `main` fallbacks
- Exact and single-wildcard package subpath exports such as `pkg/feature` and
  `pkg/features/*` are registered with their own conditional type/runtime
  entries and can be imported alongside the root; export arrays and repeated
  wildcard substitutions in their targets are resolved in declaration order
- Minimal importable `node:path`, `node:util`, `node:process`, and `node:buffer`
  modules backed by the same QuickJS polyfills used by npm dependencies
- `node:fs` exposes native synchronous UTF-8 `existsSync`, `readFileSync`,
  `writeFileSync`, and recursive `mkdirSync`, including fully static builds
- `node:http` exposes native `serveOnce(port, body)` and
  `serveOnceWith(port, callback)` server slices. Typed arrow functions compile
  to arena-backed closures, including captured values and nested closures, and
  Rust can invoke them through the callback FFI in fully static executables;
  captured mutable bindings are shared with their outer scope
- The Node-shaped `createServer(callback).listen(port)` slice passes typed
  request/response objects and supports `method`, `url`, `statusCode`,
  `setHeader`, `write`, and `end`; it currently handles one request per listen
- Native AWS Lambda Runtime API polling with synchronous or resumable async
  `(event: Json): Json` handlers; events are parsed before invocation and
  results are serialized for the response endpoint
- A C ABI promise handle and single-thread continuation queue for the future
  resumable `async` implementation
- Promise handles distinguish pending, fulfilled and rejected settlement;
  rejection wakes the same ordered continuation queue exactly once
- `throw` after suspension rejects the async completion Promise; awaiting
  callers propagate rejection without interpreting the error as a typed value
- A direct `throw` in an async `try` activates a guarded `catch` state, binds
  the error from the frame slot, and allows the catch body itself to suspend
- Rejection metadata on await-resume states routes a rejected child Promise to
  the enclosing async catch; awaits outside a try still propagate upward
- Rejection paths through `finally` run its injected finalizer once before
  rethrowing; `catch` plus `finally` finalizes after the catch settles normally
- A catch body may suspend and rethrow; the guarded rethrow rejects with the
  new error after any injected finalizer has run
- Arbitrarily nested async try blocks select the nearest handler by state;
  try/catch may also nest in async if/while blocks and vice versa
- Locals, return, and throw inside async if/while blocks use guarded frame
  states and preserve values across suspension
- Lexically shadowed locals and catch bindings receive unique HIR symbols and
  independent frame slots
- Local initializer, call-site parameter, and unannotated function return types
  are inferred; forward call chains are solved to a fixed point before final
  HIR lowering, while conflicting call-site constraints produce an error
- Assignments, returns, arithmetic/comparisons, array indexes/elements, and
  function argument count/types are checked against the inferred native layout
- Unresolved and conflicting parameter constraints identify the responsible
  function or call with its source byte range
- Identity-shaped generic functions are monomorphized once per concrete
  module-local type, with stable mangled symbols and call-site rewriting; the
  same specialization is deduplicated
- Generic variables nested in number arrays and fixed-layout numeric objects
  are inferred recursively and encoded into specialization symbols
- Function type variables used as arguments to named generic interfaces such
  as `Box<T>` are substituted before recursively expanding interface fields
- Named structures with multiple type parameters, such as `Pair<T, U>`, are
  expanded and specialized by their complete concrete field layout
- Generic projection functions can return a typed property path such as
  `pair.first` or `wrapper.boxed.value`; each specialization rebuilds the HIR
  access path with concrete object layouts and a concrete return ABI
- Timer-backed promises and a top-level `run_until_resolved` loop, providing a
  real non-threaded event source for coroutine integration tests
- File-descriptor readable/writable promises integrated with the same `poll`
  loop, including timer coexistence, cancellation on destroy, and invalid-fd
  rejection
- `await fetch(http://...)` and `await fetch(https://...)` use a non-blocking
  DNS/connect/TLS/write/read state machine with total timeout and Promise
  rejection on resolution, transport, certificate, TLS, and HTTP errors
- DNS lookup runs on a short-lived per-lookup worker and reports completion through
  the same fd event loop, so timers and unrelated I/O continue while resolving
- Streaming HTTP response parsing completes from `Content-Length` without
  waiting for keep-alive close and decodes chunked bodies incrementally
- HTTP redirects resolve absolute and relative locations, preserve one total
  timeout across hops, support HTTP-to-HTTPS upgrades, and reject after 10 hops
- `await sleep(milliseconds)` is type-checked and driven through the timer
  Promise/event loop in generated native programs
- Async functions, including functions with parameters, are split into a
  Promise-returning ramp plus an internal resume function for top-level
  `await sleep(...)`; parameters survive resume in typed frame slots
- Top-level local values have fixed frame slots and retain assignments across
  multiple await/resume cycles
- Explicit `return;` in frame-split `Promise<void>` functions resolves the
  completion promise and prevents later states from running
- Value-returning frame-split functions store their return value in a dedicated
  frame result slot; a top-level `const value = await compute()` reads the
  typed result into the caller's frame when its continuation resumes
- Await expressions nested inside a top-level expression are extracted
  left-to-right into typed temporary frame slots, including multiple awaits
  in one expression
- `await` in an `if` condition resumes before selecting the branch; awaits in
  then/else bodies use guarded states, so the inactive branch creates no
  Promise. More deeply nested async control flow remains an explicit error
- `while` bodies can suspend repeatedly: generated states re-check the loop
  condition and use a back edge after each iteration. The loop condition can
  itself await and is re-run on every iteration; direct `break` and `continue`
  in an async loop body update its exit/back-edge guards
- Nested `if` bodies compose parent/child frame guards recursively; inactive
  parents do not evaluate child conditions, and nested loop `break`/`continue`
  propagate to the enclosing async loop guards
- Nested `if` conditions may await as well; their Promise is guarded by the
  parent branch, and conditions plus branch bodies may both suspend
- Nested `while` loops receive their own enabled/condition/body guards and
  back edges recursively; inactive parents skip the inner condition Promise
- Non-throwing `try/finally` can suspend: normal completion and explicit
  `return` execute the finalizer in the order already established by HIR
- QuickJS throws and Promise rejections use a native `{ value, error }` result
  ABI and propagate through Thaw `try/catch/finally`
- Uncaught synchronous exceptions and async rejections from Lambda handlers
  are posted to the Runtime API invocation error endpoint with their original
  message
- Versioned FFI metadata can opt native functions into a typed
  `{ value, error }` result ABI whose errors propagate through Thaw catch paths
- Metadata v2 copies owned native string results/errors into the request arena
  and invokes their configured destructors exactly once

### Not yet compatible

- Contextual/generic TypeScript inference, overload resolution, classes, enums,
  tuples, broad union/intersection support, multi-capture export keys, anonymous
  default functions and the complete JavaScript expression/statement set
- Block-scoped locals inside nested control flow, nested/control-flow await,
  value-returning/general user-defined Promise async functions and concurrent
  promise combinators. Other user-defined async calls still use the synchronous
  V1 ABI
- Automatic exception propagation through external C calls that use the legacy
  direct ABI; error-aware calls must opt into `thaw-result` metadata
- Full Node.js module resolution, all core modules and the complete Node global
  API
- Full ESM semantics and a parser-backed production bundler
- A fully general ABI-description format. String result/error ownership is
  supported, but `(pointer, length)` strings, aggregate-result ownership and
  calling-convention details are not yet described
- Asynchronous Node N-API addons and the broader N-API surface beyond the
  current synchronous number/string/boolean/JSON/Buffer host
- Garbage collection. Values owned by generated code use request-scoped arena
  allocation by design

GC is intentionally not an automatic requirement: Lambda request values are
normally discarded together at the request boundary. Long-lived values,
escaping values and true asynchronous coroutine frames require explicit
lifetime rules before they are added.

## Testing

```sh
cargo test --workspace
```

Several integration tests bind loopback TCP sockets to exercise HTTP fetch and
the Lambda Runtime API. Environments which prohibit local sockets will report
`Operation not permitted` for those tests; run them in an environment that
allows loopback networking before treating that result as a product failure.

## Design documents

- [`docs/design/bridge.md`](docs/design/bridge.md): `.d.ts` classification,
  QuickJS fallback and C ABI integration
- [`docs/design/registry.md`](docs/design/registry.md): npm acquisition,
  dependency bundling, compatibility discoveries and version locking
- [`docs/design/async-await.md`](docs/design/async-await.md): synchronous V1 and
  the proposed LLVM-coroutine V2
- [`docs/design/native-addons.md`](docs/design/native-addons.md): proposed
  minimal N-API host for `.node` addons

Some design documents are chronological implementation journals. Later
sections supersede earlier statements marked as unimplemented. This README is
the current top-level status; code and tests remain authoritative.

## Development priorities

The dependency order for closing the major compatibility gaps is:

1. Establish explicit runtime value ownership and an error-result ABI.
2. Add cross-function error propagation and `finally` on that ABI.
3. Introduce a real promise/event-loop contract, then lower `async` functions
   to resumable state machines or LLVM coroutines.
4. Expand HIR typing and inference without weakening native layout guarantees.
5. Replace ad-hoc module rewriting with parser-backed module analysis and fill
   Node resolution/global compatibility from real package tests.
6. Add a versioned ABI metadata format for fast-path libraries.
7. Implement the minimal synchronous N-API host described in
   `native-addons.md`, then expand it from observed addon requirements.

The first synchronous N-API host is now implemented. `thaw registry add`
automatically selects a compatible addon bundled under
`prebuilds/<platform>-<arch>/`, copies it to
`thaw_modules/<package>/native.node`, and records its target and SHA-256 in
`native-addon.json`. `thaw build --use <package>` embeds it into the produced
executable, loads the bytes at module initialization, and routes fallback wrappers through its exported N-API
functions. Loading native addons executes unrestricted native code in the
generated process and is not sandboxed. If bundled prebuilds exist but none
match the current platform, architecture, or libc, `registry add` reports the
mismatch and retains the JavaScript fallback.
The Linux x64 prebuild from `utf-8-validate@6.0.6` is verified through both
the host API and the complete `thaw build --use utf-8-validate` pipeline.
Node's JSON Buffer shape (`{"type":"Buffer","data":[...]}`) is converted
to a real `napi_value` Buffer, and addons that return a function as their
module root are bound to the single declaration name from `package.d.ts`.

Each step must include an end-to-end native execution test in addition to unit
tests for its individual lowering/runtime layers.
