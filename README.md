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
- `let`/`const`, assignment, arithmetic, typed unary `+`/`-`/`!`, strict
  equality/inequality and ordered comparisons, `if`, `while`, classic
  `for`, `do/while`, and typed-array `for...of`
- Object fields and array indexing/mutation
- Typed array literals support multiple spreads and ordinary elements with
  single, left-to-right evaluation, including awaited spread sources
- `throw`, `try/catch` and `finally`, including propagation and rethrow across
  generated Thaw function calls and nested cleanup ordering
- `process.env`, `console.log`, JSON operations, and both legacy blocking and
  Promise-based non-blocking HTTP GET
- Synchronous lowering of `async` functions and `await`
- Ambient declarations and C ABI calls; number-array parameters become
  `(pointer, length)`, object parameters become scalar fields, and metadata-
  selected portable array/object return structs are copied into the Thaw arena
- FFI metadata v3 selects null-terminated or `(pointer, length)` string ABIs
  per parameter and return, internal/portable aggregate returns, plus
  C/fast/cold LLVM calling conventions
- QuickJS fallback for signatures which cannot use the C ABI path
- Parser-backed CommonJS/ESM dependency discovery and bundling, including
  literal and same-package runtime dynamic imports, lexical live named/default
  bindings with shadowing, live namespace/re-export bindings, synchronous
  cycles, acyclic top-level await, JSON import attributes, package `imports`,
  conditional exact/wildcard `exports`, selected Node built-in polyfills,
  scoped packages and package version locking
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
- QuickJS bundles receive `setTimeout`/`clearTimeout`, repeating
  `setInterval`/`clearInterval`, and `queueMicrotask`. Promise waits drive the
  timer queue after pending microtasks, preserve timer registration order, and
  forward callback arguments. `TextEncoder`/`TextDecoder` provide UTF-8
  `Uint8Array` conversion, including `encodeInto`, replacement decoding, BOM
  removal, and fatal decoding errors
- `node:fs` exposes native synchronous UTF-8 `existsSync`, `readFileSync`,
  `writeFileSync`, and recursive `mkdirSync`, including fully static builds
- `node:http` exposes native `serveOnce(port, body)` and
  `serveOnceWith(port, callback)` server slices. Typed arrow functions compile
  to arena-backed closures, including captured values and nested closures, and
  Rust can invoke them through the callback FFI in fully static executables;
  captured mutable bindings are shared with their outer scope
- The Node-shaped `createServer(callback).listen(port)` slice passes typed
  request/response objects and supports `method`, `url`, `statusCode`,
  `setHeader`, `write`, and `end`; `listen` registers its socket and returns,
  then the generated process entry point accepts sequential requests through
  the same fd poller used by timers and asynchronous HTTP, while
  `listenMany(port, count)` provides deterministic bounded server execution;
  partial requests and response backpressure are advanced independently across
  connections; callbacks remain single-threaded, and idempotent `close()`
  interrupts the listener while allowing accepted connections to finish;
  `listen(port, callback)` fires after registration and `close(callback)` after
  those accepted connections drain; `server.on("listening" | "close" |
  "error", callback)` supports persistent, ordered event listeners, with
  `ERR_SOCKET_BAD_PORT` and `EADDRINUSE` details for listen failures
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
- Strict equality compares numbers and booleans by value, strings by content,
  and arrays/objects by reference identity
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
  in one expression, function arguments, array literals, and object literals
- Async functions can return numbers, strings, booleans, objects, arrays, and
  heterogeneous typed tuples from conditional branches
- `Promise<T>` values can be stored in locals and object fields, passed through
  typed function parameters, and awaited later
- `new Promise<T>((resolve, reject) => ...)` supports contextually typed arrow
  executors. `.then()` transforms resolved values, `.catch()` recovers rejected
  values, returned Promises are flattened, and callback/executor throws reject
  the derived Promise. `new Promise<void>` exposes a zero-argument `resolve()`;
  direct awaits preserve both fulfillment and rejection through async frames,
  and void `.then()`/`.catch()` callbacks settle their derived Promise
- `.finally()` runs for either settlement, preserves the original value/error,
  waits for returned Promises, and replaces the result when cleanup throws or
  rejects. Directly awaited `.finally()` chains use the same async-frame
  rejection path as constructors and continuations. Expression-bodied logging
  callbacks are correctly typed as `void`. Promise callbacks accept arrows, function variables, and named
  functions; constructor `T` is inferred from consistent `resolve(value)` calls
- Resolving with another native Promise adopts its eventual state, while a
  self-resolution cycle becomes an explicit rejection
- `await` in an `if` condition resumes before selecting the branch; awaits in
  then/else bodies use guarded states, so the inactive branch creates no
  Promise
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
- `Promise.all` joins homogeneous Promise arrays concurrently. It supports
  number, string, boolean, object, and nested-array results, accepts both
  literals and array variables, and infers heterogeneous array literals as
  typed tuples. It preserves input order, supports empty arrays, composes with async
  `if`/`while`/`try`, and routes the first observed rejection to the nearest
  catch immediately. Remaining children are retained and drained before the
  process exits or the Lambda request arena resets
- `Promise.race` concurrently observes homogeneous Promise literals or array
  variables and settles with the first fulfillment or rejection. It supports
  number, string, boolean, object, and array values, deduplicates repeated
  handles, and drains every losing child before the request arena resets
- `Promise.any` ignores early rejections and resolves with the first fulfilled
  homogeneous input. If every input rejects, it reports `All promises were
  rejected` through the normal async `try/catch` path; repeated and losing
  handles are deduplicated and drained
- `Promise.allSettled` waits for every homogeneous input and returns ordered
  `{ status, value, reason }` objects. Rejections are values rather than parent
  failures; literals, Promise array variables, empty inputs, repeated handles,
  and every native value shape are supported
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
- Native, user-created and foreign thenable values work across locals,
  parameters, fields, chains, named callbacks, `.finally`, and the four
  implemented static combinators. Promise constructor inference follows
  executor-local initializer chains. Full discriminated-union narrowing
  remains unsupported
- Automatic exception propagation through external C calls that use the legacy
  direct ABI; error-aware calls must opt into `thaw-result` metadata
- Full Node.js module resolution, all core modules and the complete Node global
  API
- Full ESM semantics: the parser-backed bundler covers live imports, JSON
  `with`/`assert` attributes, acyclic top-level await and same-package runtime
  dynamic imports. Top-level-await cycles are explicit errors; `import.meta`,
  star-export ambiguity, non-JSON attributes and runtime-computed external
  package imports remain outside the supported subset
- `JsValue` retains callable/object identity across the native boundary,
  including callable return values, handle arguments, properties, methods,
  Promise resolution, constructors, mixed JSON/handle arguments and explicit
  release. Callable interfaces and default callable exports are recognized
  from `.d.ts`; generated programs release all remaining handles at shutdown.
  Fine-grained escape-based early release remains future work
- A fully general ABI-description format. Version 3 covers string layouts,
  number-array result ownership and common LLVM calling conventions, but
  target-specific struct packing, variadics and nested aggregate ownership
  are not yet described
- The broader N-API surface beyond the current number/string/boolean/JSON/
  Buffer and async-work host. The async-work lifecycle, including
  `napi_cancel_async_work`, is supported by a bounded shared worker pool
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
5. Continue filling Node resolution/global compatibility from real package
   tests; dependency discovery and the current ESM/CommonJS bundle graph are
   parser-backed.
6. Add a versioned ABI metadata format for fast-path libraries.
7. Implement the minimal synchronous N-API host described in
   `native-addons.md`, then expand it from observed addon requirements.

The first N-API host is now implemented, including shared worker-pool execution,
main-thread completion and cancellation for the core async-work lifecycle.
Thread-safe functions provide bounded blocking/nonblocking queues, worker-thread
submission, main-thread callback dispatch, acquire/release and ref/unref
lifecycle management, abort cleanup, and finalization.
Compiled callback identities are stable across native calls, and
`pollNativeAddonEvents()` lets a running program dispatch ready callbacks
before its final drain. Environment cleanup hooks run before addon unload;
unload is refused while native async work or thread-safe functions remain.
N-API deferred Promises are settled through the same main-thread poller. The
official `@parcel/watcher@2.5.1` Linux prebuild is verified by creating a real
filesystem event subscription and by embedding its platform-specific optional
dependency into a standalone executable that writes a snapshot after the
registry directory has been removed. A second standalone executable performs
subscribe, filesystem mutation, event polling, callback delivery, unsubscribe,
cleanup and normal process exit.
Class-style addons can use `napi_define_class`, wrapped native instance data,
prototype methods/accessors, construction, `instanceof`, and wrap finalizers.
The declaration bridge now extracts external classes, including inheritance,
constructor and method overloads, static methods, getters, and properties.
The N-API host exposes stable export and instance handles with constructor and
`this`-preserving instance-method calls; a real `NativeBox` addon exercises
construction plus method invocation. Registry class exports are mapped to
typed N-API constructor calls, and ordinary TypeScript named or namespace
`new` expressions are rewritten only for those external classes. The sqlite3
CLI E2E now compiles `import { Database } from "sqlite3"` followed by
`new Database(":memory:")` and constructs the real in-memory database from the
standalone executable. Automatic instance-method and callback syntax lowering
now covers callback-free methods on variables initialized from an external
constructor or aliases of a tracked instance; assignments propagate or safely
invalidate the class fact. A CLI E2E compiles and runs `new NativeBox(42)` followed
by ordinary `box.get()` syntax through the typed HIR/LLVM N-API path. Instance
methods whose final argument is a zero-to-two argument dynamic callback also
use the receiver's persistent N-API environment and retain `this`; callback
arguments such as `Error | null` and `any` cross this boundary as `Json`, and
both `Json` and `void` callback returns are supported. The same E2E exercises
`box.getLater(callback)`. The generated process now drives the default libuv
loop alongside N-API async work; a real libuv timer regression test verifies
delivery. Instance facts also follow statically named nested object properties,
property assignments, and control-flow joins; overwriting a parent invalidates
all descendant facts. Typed static class methods use the exported constructor
as their N-API receiver and share the same arity, overload, callback, and native
value marshalling rules as instance methods; named and namespace calls are
rewritten. Typed instance getters use a dedicated N-API property-result ABI and
the same native return conversion surface; getter access through tracked aliases
and object properties is rewritten as well. Typed instance setters marshal the
assigned native value through a property-result ABI, preserve the assignment
expression's value, and use the same tracked receiver paths. Typed static
getters and setters reuse the property-result ABI with the exported constructor
as receiver, including named and namespace syntax. Private non-default event
loops remain an explicit gap.
`thaw registry add`
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
The official Linux x64 prebuild from `bcrypt@6.0.0` is also verified against
the host with real synchronous hashing and callback-based asynchronous salt
generation (`THAW_BCRYPT_NODE=/path/to/bcrypt.glibc.node cargo test -p
thaw-napi runs_bcrypt_prebuild_when_supplied`).
An opt-in CLI integration test performs `registry add bcrypt@6.0.0`, embeds the
selected prebuild, deletes the registry, and runs synchronous plus callback-
based asynchronous salt generation, hashing, comparison, and error delivery
from the standalone executable
(`THAW_RUN_NPM_INTEGRATION=1 cargo test -p thaw-cli
registry_add_fetches_and_runs_bcrypt_when_enabled -- --nocapture`).
The official Linux x64 prebuild from `@serialport/bindings-cpp@12.0.1`
verifies a real `node-addon-api` class export (`Poller`). Its direct libuv
references are resolved by exposing the system `libuv.so.1`, matching the
symbols Node normally provides (`THAW_SERIALPORT_NODE=/path/to/node.napi.glibc.node
cargo test -p thaw-napi loads_serialport_class_prebuild_when_supplied`).
Packages using `prebuild-install` may keep binaries in GitHub Releases instead
of the npm tarball. For packages declaring `binary.napi_versions` and an HTTPS
GitHub repository, `registry add` selects the newest supported N-API version,
downloads the target asset without running install scripts, safely unpacks its
`.node` file, and records the source URL and binary SHA-256. `sqlite3@5.1.7` is
verified through this path: its official N-API v6 Linux x64 prebuild initializes
the real `Database`, `Statement`, and `Backup` classes, and an opt-in CLI E2E
embeds it into an executable that starts after the registry is removed
(`THAW_RUN_NPM_INTEGRATION=1 cargo test -p thaw-cli
registry_add_fetches_and_loads_sqlite3_when_enabled -- --nocapture`). Typed
construction is integrated, callback-free instance methods are supported, and
dynamic error-first callbacks work on instance methods. The host drains pending
constructor work before a callback method and consumes exceptions at their
async-completion boundary, preventing an earlier completion error from poisoning
the next call. The official sqlite3 E2E now constructs a real `Database`, calls
`close(callback)`, and receives the callback after the registry is removed.
Full TypeScript-style overloaded method selection remains outside that path.
Registry class shims now retain every supported overload under a distinct
internal symbol. Calls select an overload by exact argument count and whether
the final argument is an inline or locally-bound callback. Non-callback
overloads with the same arity are also selected from number, string, boolean,
number-array, and object literals or local variables initialized from those
values. The local inference also follows arithmetic, string concatenation,
comparisons, conditional expressions, templates, parentheses/type assertions,
primitive conversion calls, and `.length`. Trailing optional method parameters
generate every callable arity from the required prefix through the complete
signature. Number-typed rest
parameters are expanded only for the argument counts observed at compiled call
sites, so they do not impose an arbitrary maximum arity. User-function calls
participate in overload selection through explicit return annotations or a
uniformly inferred return type; repeated collection resolves forward call
chains. Parameter-dependent/conflicting returns and rest element types beyond
the currently native-representable surface remain explicit gaps. Object
literals, including shorthand properties such as `{ value }`, retain
recursively inferred field types, so
property reads and same-arity structural object overloads are selected by field
name and type. Straight-line `=` assignments update local and nested property
types; statically named computed properties are included, while unknown or
compound assignments invalidate the affected fact. String-literal computed
object keys such as `{ ["value"]: 1 }` are supported. Object spreads from a
statically typed local variable, nested object literal, or typed function call
preserve their fields and allow later properties to override them. A call used
as a spread source is bound through an internal closure so it executes exactly
once. Multiple expression spreads retain left-to-right evaluation order, and
matching-type conditional expressions can also supply an object. Conditional
expressions are supported generally when both branches have the same native
type. Dynamic computed keys and dynamically typed spread sources remain outside
this path. `if/else`
branches are analyzed from the same incoming state and retain
only value, class-instance, and callback facts that agree on every outgoing
path; an omitted `else` joins against the unchanged incoming path. `while` and
classic `for` loops join the body/update result against the zero-iteration path,
retaining only facts unchanged by a possible iteration. `do/while` lowers to
the same loop form while preserving its mandatory first iteration and
condition-before-continue behavior, including async bodies. Ordinary `for...of`
supports typed arrays, evaluates its iterable once, and preserves
break/continue across synchronous and async bodies. Its loop head may declare
an identifier or assign each element to an existing same-typed variable.
`for await...of` additionally unwraps typed Promise-array elements in order,
accepts synchronous typed arrays, and routes rejection to async `try/catch`.
`for...in` evaluates a fixed-shape object once and enumerates its statically
known keys in layout order, with declaration/assignment heads and async bodies.
Dynamic indexed reads such as `object[key]` remain a separate unsupported path.
`try/catch` conservatively
joins normal exit with a catch entry that retains only facts unchanged by the
try block; `finally` then applies to the merged state and can establish facts
on every continuing path. `switch` evaluates its discriminant once, short-circuits
case tests after the first match, and preserves default placement, fallthrough,
break, case-local bindings, and awaits in case tests/bodies.
Compiled programs can pass a `(Json, Json) => Json` closure through
`callNativeAddonWithCallback(name, args, callback)`. Callback environments stay
alive until async-work drain, and N-API error/result values are converted back
to `Json` on the generated program's main thread. The optional bcrypt CLI E2E
embeds the official prebuild and exercises asynchronous salt, encrypt, compare,
and error callbacks after deleting the registry directory.
Node's JSON Buffer shape (`{"type":"Buffer","data":[...]}`) is converted
to a real `napi_value` Buffer, and addons that return a function as their
module root are bound to the single declaration name from `package.d.ts`.

Each step must include an end-to-end native execution test in addition to unit
tests for its individual lowering/runtime layers.
