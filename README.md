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
- `number`, `string`, `boolean`, `null`, `void`, `Json`, number arrays and fixed-shape
  objects; function types may be parenthesized where TypeScript grammar
  requires it (including `FunctionType | undefined`)
- `interface`, interface inheritance (including substituted generic bases such
  as `Child<T> extends Base<T>` and concrete bases such as
  `NumberBox extends Box<number>`) and generic interface instantiation;
  generic interfaces accept trailing type-parameter defaults (including
  defaults that reference earlier parameters) and validate concrete
  primitive/union or structural-object constraints. Object-shaped generic
  type aliases may also be used as concrete interface bases. Generic
  interface bodies resolve forward-declared interface field types before
  concrete instantiation
- Numeric and string `enum` declarations lower to typed native constants,
  including implicit numeric numbering, preceding-member constant expressions,
  bracketed string member reads, runtime numeric reverse lookup (unknown values
  produce `undefined`), and use before the declaration
- Same-layout literal unions and intersections normalize to their native
  primitive representation (`"a" | "b"` to string, numeric literals to
  number, and boolean literals to boolean), including `.d.ts` Fast-path
  signatures, nested object/array positions, and async frames
- Compatible fixed-object intersections merge distinct fields in source order
  across native calls, `.d.ts` Fast paths, and async frames; duplicate fields
  must have the same native type
- Non-generic top-level `type` aliases resolve through forward alias chains and
  mutually forward-referenced interfaces, covering primitive, object, array,
  union, intersection, function, and promise layouts. Nested alias references
  inside object/array/tuple/function types are pre-resolved, and mixed
  alias/interface cycles are explicit errors
- Generic `type` aliases substitute concrete arguments through nested object,
  alias, array, optional/nullable union, function, and promise layouts. They
  participate in generic-function call-site inference, validate arity and
  reject direct or indirect generic-alias cycles. Trailing type-parameter
  defaults may reference earlier parameters, and concrete primitive/union or
  structural-object constraints are validated at each instantiation
- Heterogeneous native unions use an explicit tag and word payload across
  locals, function parameters/returns, fixed-shape object fields, nested calls,
  async frames, and homogeneous arrays (including index updates and spread
  concatenation). `typeof` observes the runtime member and narrows two-member
  and larger unions through nested equality/inequality branches, terminating
  guard clauses, residual tag sets, and assignment to a known member. Union
  values can be logged directly with runtime dispatch for primitive members;
  strict equality/inequality against concrete members short-circuits on the
  tag and applies the member's native comparison semantics. Equality between
  two values of the same union compares tags first and dispatches to the
  matching member semantics, including NaN and string-content behavior.
  Conditional expressions with two different native branch types construct a
  tagged union directly and can feed annotations, returns, and nested calls
- Local-variable inference from supported expressions
- `let`/`const`, assignment, arithmetic (including remainder, exponentiation,
  bitwise/shift operations, and their compound assignments), typed unary
  `+`/`-`/`!`/`~`/`typeof`/`void`, strict equality/inequality, same-type `==`/`!=`, and
  ordered comparisons, `if`, `while`, classic
  `for`, `do/while`, and typed-array `for...of`
- `let`/`const` object and fixed-length tuple destructuring, including nested
  patterns, holes, renaming, object/tuple rest, awaited sources, and
  short-circuited defaults for tagged optional fields/elements (including
  awaited defaults)
- `for...of` and `for await...of` accept fixed object/tuple destructuring in
  declaration and assignment heads, preserving break/continue behavior
- Fixed-layout object/tuple destructuring assignments update existing bindings,
  evaluate the right-hand side once, and return that original right-hand value
- Function, typed arrow, and contextually typed Promise callback parameters
  accept nested fixed-layout object/tuple destructuring and rest patterns
- Object fields and array indexing/mutation
- Fixed-shape object fields can also use static string-computed reads,
  assignments, compound assignments, and updates; JSON accepts string keys
- Dynamic string-computed reads are supported for fixed objects whose fields
  share one native type, including uniformly optional, nullable, or nullish
  fields. Object and key evaluate once; known keys preserve their existing
  nullable state, nullable fields widen to distinguish an unknown key as
  `undefined`, and unknown keys return `undefined`, including when key
  evaluation suspends
- Optional member, computed-member, and function calls are accepted for native
  types whose static layout excludes `null`/`undefined`; tagged optional fixed
  objects additionally short-circuit named and static string-computed field
  reads and flatten already-optional fields, while tagged optional arrays and
  tuples short-circuit computed element reads. Tagged optional strings,
  arrays and tuples support short-circuited `.length`; tagged optional function
  values support `callback?.(...)`, and tagged receivers support the existing
  native/string/array/fixed-object method surface through `receiver?.method()`;
  void returns and awaited receivers/arguments preserve short-circuit ordering
- String-literal `in` checks use fixed object shapes while still evaluating
  both operands once in source order
- Comma/sequence expressions evaluate every operand from left to right and
  return only the final value, including awaited and throwing operands
- Prefix and postfix `++`/`--` return the correct expression value and evaluate
  computed array and numeric object-field targets once, including awaited
  indexes and object expressions
- Compound assignments evaluate array/object references and computed indexes
  once before evaluating the right-hand side, including awaited components
- Typed array literals support multiple spreads and ordinary elements with
  single, left-to-right evaluation, including synchronous elements before and
  after direct or nested awaited spread sources
- `Promise.all`, `allSettled`, `race`, and `any` accept homogeneous array
  literal spreads while retaining dynamic `Promise<T>[]` sources and order
- Named function calls support fixed-length argument spreads from array
  literals and typed tuples, preserving single left-to-right evaluation across
  ordinary, spread, and awaited arguments
- `throw`, `try/catch` and `finally`, including propagation and rethrow across
  generated Thaw function calls and nested cleanup ordering
- `process.env`, `console.log`, JSON operations, and both legacy blocking and
  Promise-based non-blocking HTTP GET
- Synchronous lowering of `async` functions and `await`
- Ambient declarations and C ABI calls; number-array parameters become
  `(pointer, length)`, object parameters become scalar fields, and metadata-
  selected portable array/object return structs are copied into the Thaw arena
- Ambient and `.d.ts` Fast-path declarations ending in number, boolean, string,
  `JsValue`, `number[]`, `boolean[]`, `string[]`, `JsValue[]`, or fixed-object array rest parameters lower to
  real C varargs. Scalar extras are passed as `double`, C-promoted `int`,
  NUL-terminated `const char *`, or opaque 64-bit handles. Each supported array
  expands to `(const element *, int64_t)` and each fixed object expands to its
  declaration-ordered fields with C default promotions, recursively applying
  the same rules to nested fixed objects and `number[]` fields. Optional and
  nullable values prepend a promoted `int` present tag; nullish values prepend
  a promoted `int` state tag (`0` value, `1` null, `2` undefined), followed by
  the recursively expanded payload.
  FFI metadata v4 can instead convert number extras to `i32`, `i64`, `u32`, or
  `u64` with `variadicAbi`
- Direct `void` FFI calls execute as statements; `thaw-result` void functions
  return `{ error }` and propagate native failures through `try/catch/finally`
- FFI metadata v3 selects null-terminated or `(pointer, length)` string ABIs
  per parameter and return, internal/portable aggregate returns, plus
  packed C aggregate returns and C/fast/cold LLVM calling conventions. Packed
  values use the target C ABI's indirect return convention
- FFI metadata v4 can additionally describe fixed object return layouts with
  explicit `fieldOffsets`, total `size`, `alignment`, and an `indirect` return
  convention. LLVM emits exact byte padding, aligns the return storage, and
  reconstructs the object fields from the declared offsets for direct and
  `thaw-result` returns. `fieldLayouts` recursively describes nested object
  fields; scalar fields use `null`. `bitFields` can map multiple boolean or
  number fields onto shared 1/2/4/8-byte C storage using explicit `bitOffset`,
  `bitWidth`, `storageBytes`, and signedness
- Small explicit aggregate returns can set `indirect: false` and provide one
  `registerClasses` entry (`integer` or `sse`) per eight-byte unit. LLVM writes
  the returned registers into the declared byte layout before reconstructing
  ordinary and bitfield values
- QuickJS fallback for signatures which cannot use the C ABI path
- Parser-backed CommonJS/ESM dependency discovery and bundling, including
  literal and same-package runtime dynamic imports, constant-folded external
  dynamic imports, lexical live named/default
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
  `setInterval`/`clearInterval`, `setImmediate`/`clearImmediate`, and
  `queueMicrotask`. Promise waits drive the
  timer queue after pending microtasks, preserve timer registration order, and
  forward callback arguments. `TextEncoder`/`TextDecoder` provide UTF-8
  `Uint8Array` conversion, including `encodeInto`, replacement decoding, BOM
  removal, and fatal decoding errors
- QuickJS bundles provide validating Latin-1 `btoa` and Base64 `atob`
  globals, including standard padding and ASCII-whitespace decoding
- QuickJS bundles expose `performance.timeOrigin` and elapsed-millisecond
  `performance.now()` values scoped to the shared JavaScript context
- QuickJS bundles provide cycle-preserving `structuredClone` support for
  objects, arrays, Map, Set, Date, RegExp, ArrayBuffer and typed-array views
- QuickJS bundles provide `AbortController`/`AbortSignal` with reasons,
  `onabort`, listener registration, `throwIfAborted`, and static abort/timeout
  signals integrated with the timer queue
- The ambient `process` global and importable `node:process`/`process` module
  share a microtask-backed `nextTick` that is asynchronous and forwards
  callback arguments
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
- Boolean `&&`/`||` short-circuit without evaluating the inactive operand;
  selected awaited operands settle or reject through the surrounding catch path
- Unresolved and conflicting parameter constraints identify the responsible
  function or call with its source byte range
- Identity-shaped generic functions are monomorphized once per concrete
  module-local type, with stable mangled symbols and call-site rewriting; the
  same specialization is deduplicated. Inferred type tuples validate primitive,
  union, structural-object, and earlier-parameter-dependent `extends`
  constraints before specialization. Uninferred parameters use trailing
  defaults in declaration order, including defaults that reference an earlier
  inferred parameter. Calls may provide explicit type arguments, with arity,
  argument compatibility, defaults and constraints validated before emitting
  distinct specializations (including zero-parameter generic functions)
- Named generic functions used as array or Promise callbacks specialize from
  the contextual callback parameter types, with the same constraint checks and
  per-type deduplication as direct calls
- Inline generic arrow callbacks in array and Promise APIs infer their concrete
  type tuple from the callback context, validate defaults/constraints and lower
  the body with concrete parameter and return layouts
- A monomorphic function-type annotation contextually specializes generic
  arrows (and supplies omitted parameter annotations for ordinary arrows),
  including multi-parameter inference and constraint validation
- Unannotated local generic arrow variables specialize independently at each
  inferred or explicit call, including captured locals, tuple/array-literal
  spread arguments, array/Promise callback use and `typeof` observation
- Anonymous generic function expressions use the same local polymorphic
  template, callable-assignment validation and contextual callback paths
- Named function expressions use the same path when their internal name is not
  recursively referenced; recursive local function values remain unsupported
- User-defined function parameters with a monomorphic function type provide
  the same contextual specialization for named generic functions, local
  generic arrows and inline generic arrows
- Generic function type aliases such as `type Identity = <T>(value: T) => T`
  retain a polymorphic template when assigned a compatible generic arrow or
  named generic function; parameter/return shapes and constraints are checked
  modulo type-parameter names before call-site specialization. Generic defaults
  are checked the same way so omitted call-site types retain the annotated
  contract. Assignments to these polymorphic annotations require an explicit
  implementation return type when the body does not directly return one of
  its annotated parameters. Direct identity returns infer that parameter's
  generic pattern for arrow and function expressions, including conditional
  expressions and `if`/block paths whose every return selects the same
  parameter. Primitive number, string, boolean, `null` and `undefined` returns
  infer their distinct concrete type through the same control-flow check.
  Comparisons, supported unary operators, templates and primitive conversion
  calls also contribute their statically fixed result type. Numeric binary
  operators except the string-sensitive `+` infer `number`. `+` infers
  `string` only when a string literal, template or `String()` operand
  makes concatenation statically certain. Conditional, `&&`, `||` and `??`
  results are inferred when every possible result has the same generic or
  concrete type pattern. The equivalent
  call-signature literal form `type Identity = { <T>(value: T): T }` shares
  the same path
- Generic callable assignment preserves and compares optional-parameter
  positions instead of treating required and optional implementations as the
  same shape; calls that supply all optional arguments use normal specialization
- Callable assignment compares the effective Promise-wrapped return of named
  async generic functions, rejecting synchronous aliases while accepting an
  explicit `Promise<T>` callable contract without adding a second Promise layer
- Pure callable interfaces with one generic call signature, such as
  `interface Identity { <T>(value: T): T }`, use the same polymorphic template,
  assignment checks and direct/callback specialization paths; annotated local
  variables can forward either arrow or named-function templates through
  compatible callable aliases and interfaces without losing polymorphism;
  subsequent unannotated local aliases infer and retain that template.
  Non-generic alias chains ending in either callable form are resolved across
  multiple links and forward declarations; parentheses around callable types,
  alias targets and variable annotations do not change classification
- Empty interfaces with one callable `extends` base inherit its polymorphic
  signature through multiple levels and forward declarations
- Generic instantiation expressions such as `identity<number>` produce ordinary
  monomorphic function values that can be stored, called and passed to array or
  Promise APIs; explicit arity, defaults and constraints are validated
- Named async generic functions specialize for inferred or explicit types and
  retain exactly one Promise layer when used directly, as instantiated function
  values, or as assimilated Promise callbacks
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
- `&&` and `||` preserve and return same-typed native operands, apply
  JavaScript truthiness to booleans, numbers (including `NaN`), strings,
  JSON and reference values, evaluate the left side once, and short-circuit
  synchronous or awaited right sides
- Template literals with string, boolean, and number interpolations concatenate into
  arena-owned native strings, preserve left-to-right evaluation, and allow
  interpolations that suspend with `await`; `String(boolean)` and
  `String(number)` use the same native conversions. Number formatting follows
  JavaScript's `NaN`, infinity, signed-zero, fixed/exponential boundary and
  shortest-round-trip rules
- Binary `+` and `+=` concatenate when either operand is a native string,
  converting number/boolean operands with the same JavaScript formatting and
  preserving reference evaluation plus synchronous/awaited operand order
- Function calls evaluate direct synchronous and awaited arguments once in
  left-to-right order, including calls returning `void`; argument temporaries
  survive async frame suspension and resume before later arguments run
- Eager binary operators preserve their left operand before a direct or nested
  awaited right operand suspends, including arithmetic, comparison and nested
  parenthesized expressions
- Array elements and fixed-object fields are evaluated once in source order
  when a later direct or nested value awaits; completed values remain live in
  the async frame until aggregate construction resumes
- `Boolean(...)` applies the same JavaScript truthiness rules to native
  booleans, numbers, strings, JSON and reference values, evaluating its
  synchronous or awaited argument exactly once
- `Number(...)` accepts native numbers, booleans and strings. String parsing
  covers ECMAScript whitespace, signs, decimal/exponent syntax, arbitrary-size
  hexadecimal/octal/binary input, infinities and `NaN` for invalid input, and
  works after an awaited string expression
- Primitive `==` and `!=` implement JavaScript abstract equality conversion
  across number, string and boolean operands, including `NaN`, while preserving
  left-to-right single evaluation and awaited operands; same-typed values keep
  the existing strict-layout comparison
- Primitive `<`, `>`, `<=` and `>=` compare two strings in JavaScript UTF-16
  code-unit order; other number/string/boolean combinations use numeric
  conversion, preserve `NaN`'s unordered result, and support awaited operands
- `String(...)`, template interpolation and string addition convert fixed
  objects to `[object Object]` and homogeneous number/string/boolean/object
  arrays with JavaScript comma-join semantics, using arena-owned results and
  evaluating aggregate expressions once
- Heterogeneous typed tuples use the same comma-join conversion recursively,
  selecting each slot by its static type and evaluating the tuple source once;
  nested tuples, arrays and objects compose without a dynamic value box
- Zero-argument `.toString()` is available on native numbers, booleans,
  strings, fixed objects, typed arrays and heterogeneous tuples, including
  receivers produced by side-effecting calls or `await`
- Zero-argument `.valueOf()` preserves native number, string and boolean
  primitives, evaluating synchronous or awaited receivers exactly once
- `Number.isNaN`/`Number.isFinite` perform non-coercing checks, while global
  `isNaN`/`isFinite` apply native numeric conversion first. Aggregate numeric
  conversion follows each value's native string form, so empty/single/multiple
  element arrays match JavaScript behavior; arguments may suspend and are
  evaluated once
- `Math.abs`, `Math.floor`, `Math.ceil`, `Math.trunc` and `Math.sqrt` apply
  JavaScript numeric coercion to native primitive/aggregate arguments and use
  LLVM floating-point intrinsics, preserving `NaN`, infinities, signed zero,
  single evaluation and awaited arguments
- `Math.pow`, variadic `Math.min`/`Math.max`, and `Math.sign` use the same
  left-to-right numeric coercion. Empty extrema return signed infinity, `NaN`
  propagates, zero ties retain JavaScript's sign ordering, and unary negation
  preserves negative zero
- `Math.round` implements JavaScript's ties-toward-positive-infinity rule,
  including negative-zero results between `-0.5` and zero, numeric coercion,
  non-finite values, single evaluation and awaited arguments
- `Math.exp`, `Math.log`, `Math.log2`, `Math.log10`, `Math.sin` and `Math.cos`
  lower to LLVM floating-point intrinsics after JavaScript numeric coercion;
  domain errors, infinities, `NaN`, single evaluation and `await` are preserved
- `Math.tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `cbrt`, `atan2`
  and variadic `hypot` use the platform libm ABI. Empty hypot calls, stable
  overflow handling, signed zero, non-finite values, coercion order and awaited
  operands follow JavaScript behavior
- `Math.acosh`, `asinh`, `atanh`, `expm1` and `log1p` call their dedicated
  libm operations, retaining near-zero precision, signed zero, domain errors,
  numeric coercion and awaited operands
- `Math.fround`, `clz32` and `imul` implement float32 rounding and exact
  ECMAScript ToUint32/wrapping multiplication semantics, including negative
  zero, non-finite inputs, signed 32-bit results, coercion order and `await`
- `Math.random()` advances an atomic process-local xorshift state and produces
  a 53-bit fraction in the required half-open `[0, 1)` interval
- Standard `Math.E`, `PI`, `LN2`, `LN10`, `LOG2E`, `LOG10E`, `SQRT1_2` and
  `SQRT2` properties lower directly to correctly rounded native constants
- `Number.NaN`, positive/negative infinity, `MAX_VALUE`, `MIN_VALUE`, safe
  integer bounds and `EPSILON` likewise lower to their exact IEEE-754 values
- Unshadowed global `NaN` and `Infinity` identifiers lower to the same native
  values; lexical parameters and locals with those names still take precedence
- Global `parseFloat` and `parseInt` coerce native values before scanning the
  longest valid numeric prefix. They support whitespace/sign handling,
  incomplete exponents, infinity, radix inference and validation, signed zero,
  trailing text, single evaluation and awaited arguments
- `Number.parseFloat` and `Number.parseInt` are aliases of the same native
  parsing paths, including radix inference, coercion and suspension behavior
- Native homogeneous arrays and heterogeneous tuples implement `.join()` with
  JavaScript's default comma or a coerced custom separator. Number, string,
  boolean, fixed-object and nested aggregate elements, empty arrays, receiver
  ordering and awaited receivers are supported
- Homogeneous number, string, boolean and fixed-object arrays implement
  `.indexOf()`, `.lastIndexOf()` and `.includes()` with positive/negative and
  infinite starting positions.
  Numeric searches distinguish strict equality from SameValueZero (`NaN`),
  treat signed zeros equally, while objects use reference identity; every
  receiver/argument is evaluated once in order and may suspend with `await`
- Homogeneous native arrays implement in-place `.reverse()` for every existing
  eight-byte element layout, returning the same receiver and supporting empty,
  side-effecting and awaited arrays
- Homogeneous arrays implement in-place `.copyWithin()` with memmove-safe
  overlap, JavaScript negative/clamped indices, optional end, left-to-right
  coercion and awaited index expressions
- Homogeneous native arrays implement in-place `.fill()` for numbers, strings,
  booleans and fixed objects, with negative/clamped bounds, shallow reference
  assignment and left-to-right awaited argument evaluation
- Homogeneous native arrays implement `.concat()` with one-level flattening of
  same-element arrays, scalar element arguments, arena-owned shallow copies,
  empty inputs and left-to-right awaited receiver/argument evaluation
- Homogeneous arrays implement arena-owned shallow `.slice()` copies with
  omitted/negative/clamped indices, empty ranges, unchanged source arrays,
  shared object elements, ordered coercion and awaited bounds
- ES2023 `.toReversed()` returns an arena-owned reversed shallow copy without
  mutating its homogeneous source array; object sharing, empty and awaited
  receivers follow the same native array rules
- Homogeneous native arrays implement stable default `.sort()` and ES2023
  `.toSorted()`: numbers use their JavaScript string keys, strings compare
  UTF-16 code units, booleans order `false` before `true`, fixed objects retain
  source order, and `toSorted` returns an arena-owned shallow copy. A typed
  synchronous comparator may be a contextually typed arrow (including
  captures) or named function; stable HIR sorting propagates callback errors
- Homogeneous native arrays implement short-circuiting `.some()`, `.every()`,
  `.find()`, `.findIndex()` and ES2023 `.findLast()`/`.findLastIndex()` with
  contextually typed or named boolean predicates
  receiving zero to three `(element, index, array)` parameters. Captures, empty
  arrays, all native element layouts, optional `thisArg` evaluation and awaited
  receivers work; the index methods return the first match in their respective
  traversal direction or `-1`, while value methods return a collision-free
  tagged `T | undefined`
- Homogeneous native arrays implement `.at()` with numeric coercion,
  truncation, negative indexing, ordered/awaited receiver and index evaluation,
  and the same tagged `T | undefined` result for out-of-range access
- Homogeneous native arrays implement `.forEach()` with the same typed callback
  arguments and receiver/optional `thisArg` ordering, visiting every element
  once in index order and returning `void`; captures, empty arrays, every native
  element layout and awaited receivers are supported
- Homogeneous native arrays implement `.reduce()` and `.reduceRight()` with
  typed zero-to-four-argument reducers, distinct accumulator and element types,
  captures, named callbacks, ordered receiver/initial-value evaluation and
  awaited receivers. Calls without an initial value seed from the appropriate
  endpoint and throw on empty arrays, matching JavaScript behavior
- Homogeneous native arrays implement `.map()` through runtime-length,
  arena-owned typed allocation. Mappers receive zero to three typed arguments
  and may change the element type; named callbacks, captures, empty arrays,
  nested aggregates, optional `thisArg` ordering and awaited receivers work
- Homogeneous native arrays implement `.filter()` with stable shallow copies,
  typed zero-to-three-argument predicates and arena capacity trimming. Named
  callbacks, captures, empty arrays, every native element layout, optional
  `thisArg` ordering and awaited receivers are supported
- ES2023 `.with(index, value)` returns an arena-owned shallow copy of a
  homogeneous array, applies JavaScript integer and negative-index rules,
  preserves the source, evaluates receiver/index/value once in order and
  throws for out-of-range or empty-array writes; awaited operands are supported
- ES2023 `.toSpliced()` returns an arena-owned shallow copy and supports every
  argument form from a no-op copy through delete-to-end and variadic insertion.
  Start/delete values use JavaScript truncation and clamping, source and
  inserted values are evaluated once in order, and awaited operands work
- Homogeneous native arrays implement `.flatMap()` by mapping once and
  flattening exactly one array level into an arena-owned result. Typed named or
  contextual callbacks may change the final element type; captures, empty
  inner/outer arrays, nested aggregates, `thisArg` ordering and awaited
  receivers are supported
- Nested homogeneous arrays implement `.flat()` with the default depth and
  non-negative, fractional or negative numeric-literal depths. Each requested
  static level is flattened into an arena-owned shallow result; depth zero and
  already-flat arrays are copied, while dynamic depths remain an explicit
  layout error because their result element type is not statically fixed
- `Array.of()` constructs homogeneous native arrays for every supported
  element layout, preserves scalar/spread evaluation order and accepts awaited
  spreads. Explicit element type arguments support empty construction
- `Array.from()` copies homogeneous native arrays or iterates strings by
  Unicode code point, and can map either source through a typed zero-to-two-
  argument callback into a new element type. Explicit input/output type
  arguments, shallow object identity, empty and awaited sources, named
  callbacks, captures and optional `thisArg` evaluation are supported
- `Array.isArray` recognizes native homogeneous arrays, typed tuples and
  runtime JSON arrays, returns false for other native/JSON values, evaluates
  its operand once and accepts awaited arrays
- `Object.keys` returns an arena-owned string array for fixed-layout objects,
  preserving declaration/insertion order across property overrides and
  evaluating synchronous or awaited receivers exactly once;
  `Object.getOwnPropertyNames` and `Reflect.ownKeys` share this result because
  the current fixed-object model has only enumerable string-named own fields
- `Object.values` returns field values in the same order, retaining homogeneous
  native arrays or heterogeneous typed tuples as appropriate, including empty
  objects and synchronous/awaited receivers
- `Object.entries` returns ordered, explicitly typed `[string, value]` tuples;
  homogeneous entry shapes remain native arrays while heterogeneous field
  types retain a statically indexed outer tuple, including empty/awaited input
- `Object.hasOwn` checks fixed-layout own fields after native primitive-to-key
  string conversion, including empty objects and synchronous/awaited object
  plus key evaluation in source order
- `Object.is` implements SameValue for statically native operands: all `NaN`
  values compare equal, signed zeros remain distinct, primitive strings compare
  by contents, and aggregate/Promise values use reference identity; both
  operands are evaluated once in order and may await
- Native strings implement `.indexOf()`, `.lastIndexOf()`, `.includes()`,
  `.startsWith()` and `.endsWith()` using JavaScript UTF-16 code-unit positions rather than UTF-8
  byte offsets. Search values and positions are coerced left-to-right, clamped
  positions and empty searches follow JavaScript behavior, and receivers may
  suspend with `await`
- Native strings implement `.trim()`, `.trimStart()` and `.trimEnd()` with the
  exact ECMAScript whitespace/line-terminator set (rather than the broader
  host-language predicate), arena-owned results, single evaluation and `await`
- Native strings implement locale-independent `.toLowerCase()` and
  `.toUpperCase()` with Unicode multi-character case expansion, arena-owned
  results, empty strings and awaited receivers
- Strict equality and inequality recognize every native string-producing
  builtin and compare C-string contents, not arena/global pointer identities
- Native strings implement variadic `.concat()` with left-to-right receiver
  and argument evaluation, aggregate/primitive conversion, arena ownership and
  awaited operands
- Native strings implement `.repeat()` with numeric coercion, JavaScript
  truncation and `NaN` handling, Unicode-preserving arena-owned output,
  receiver-before-count evaluation, awaited operands and range errors for
  negative or infinite counts
- ES2024 `.isWellFormed()` returns true for every native string and
  `.toWellFormed()` preserves it, reflecting the runtime's invariant that all
  native string values are valid UTF-8; receiver evaluation and `await` remain
  observable in source order
- Native string `.length` and `.charCodeAt()` operate on JavaScript UTF-16
  code units, including surrogate pairs, default/converted indices, out-of-range
  `NaN`, receiver-before-index evaluation and awaited receivers
- `Number.isInteger` and `Number.isSafeInteger` are non-coercing predicates;
  they reject non-number values, fractions, `NaN` and infinities, preserve
  signed-zero behavior, enforce the ±(2^53−1) safe range, and accept awaited
  number expressions
- Nested `while` loops receive their own enabled/condition/body guards and
  back edges recursively; inactive parents skip the inner condition Promise
- Labeled statements support `break label`, and labeled iteration statements
  support `continue label`. Multi-level exits preserve classic `for` updates
  and propagate through nested synchronous and suspended async loop guards
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
- Void native functions use the corresponding `{ error }` result ABI, including
  owned error copying and exactly-once destruction
- Metadata v2 copies owned native string results/errors into the request arena
  and invokes their configured destructors exactly once
- Portable/packed object returns recursively rebuild nested objects and
  `number[]` fields in the arena. Return ownership applies to every string and
  array-data leaf, invoking its configured destructor exactly once per pointer

### Not yet compatible

- Contextual/generic TypeScript inference, overload resolution, classes,
  tuples, incompatible/non-object intersections, multi-capture export keys and the
  complete JavaScript expression/statement set. Anonymous default functions
  are assigned stable bundle-local symbols. General runtime enum-object
  reflection and heterogeneous enums remain outside the native constant
  subset. Compatible declarations merge in source order, with duplicate
  member names and mixed native layouts rejected
- Tagged native `T | undefined` values now support annotations, returns,
  strict undefined comparison, logging and array lookup APIs without sentinel
  collisions. Nullish coalescing and `??=` unwrap or update the payload with
  true RHS short-circuiting, including across suspension points and typed
  object fields. Direct strict/loose comparisons with `undefined` narrow local
  variables in the corresponding `if` branch, including negated comparisons;
  terminating guard clauses and subsequent assignments update the narrowed
  state. Logical `&&`/`||` propagate safe narrowing into their short-circuited
  RHS and the implied `if` branch. `typeof value ===/!== "undefined"` reports
  the tagged runtime state and provides the same branch narrowing. Optional
  chaining now short-circuits tagged
  fixed-object fields, array/tuple elements, native `.length`, function-value
  calls and receiver-bound methods. Native `null` and tagged `T | null` have
  distinct display, `typeof`, equality, assignment, narrowing, `??`/`??=` and
  optional-chain semantics. Native `T | null | undefined` uses a dedicated
  three-state tag across locals, parameters, returns, object fields and async
  frames, with distinct display, `typeof`, strict/loose nullish equality,
  `??`/`??=`, loose-nullish guard narrowing and optional-chain behavior.
  Optional chains also preserve all three outcomes
  when a nullable field, method result or function result is reached: a value,
  `null`, or receiver-produced `undefined`. Non-literal dynamically computed
  method names still require broader lowering; bracketed string-literal
  methods are resolved statically, including through optional chains
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
  dynamic imports. Relative-module bundling also supports namespace imports,
  `export * as name`, default re-export aliases and anonymous default
  functions. `export default localName` also preserves the referenced
  top-level declaration. Star exports follow explicit-export precedence,
  merge identical bindings and propagate ambiguity through barrel modules;
  importing an ambiguous name is a source-located error. Top-level-await
  cycles are explicit errors. `import.meta.url` is replaced per source module
  with its percent-encoded absolute `file://` URL; Node-compatible
  `import.meta.filename` and `.dirname` expose unescaped absolute paths. Static relative/absolute
  `import.meta.main` is `true` only in the executable entry module.
  `import.meta.resolve(specifier)` folds string templates, concatenation,
  parentheses and type assertions, uses lexical path normalization, and
  preserves queries/fragments. Bare registry packages and subpaths resolve to their
  selected `bundle.js`, `native.node` or `native.a` artifact, while `node:`
  builtins preserve their URL. Other meta properties, non-JSON attributes and
  runtime-computed imports of undeclared packages/subpaths remain outside the
  supported subset. A package with a nonliteral dynamic import pre-bundles
  installed `dependencies`, `optionalDependencies`, and `peerDependencies`,
  including exact and file-backed wildcard entries from each dependency's
  conditional `exports` map. Dependencies without an `exports` field also
  expose their installed JS/JSON deep subpaths, including extensionless and
  directory-index forms, to runtime-computed imports. Runtime query/fragment
  suffixes resolve through the same candidate map while retaining distinct
  module-cache identities.
  Module specifier queries and fragments are excluded from filesystem
  resolution but retained in cache keys, so repeated imports of the same URL
  share one namespace while distinct suffixes create distinct module instances.
  Expression-free
  templates, parentheses and string-only concatenations are folded and bundled;
  conditional branches and template interpolations composed from those values
  produce a bounded finite candidate set (up to 64 external specifiers)
- Static and dynamic JSON imports accept modern `{ with: { type: "json" } }`
  and legacy `{ assert: { type: "json" } }` forms. Dynamic options are
  validated before rewriting; spreads, unknown options, non-string attributes,
  extension mismatches and attributed imports without finite specifiers are
  explicit bundle errors
- `JsValue` retains callable/object identity across the native boundary,
  including callable return values, handle arguments, properties, methods,
  Promise resolution, constructors, mixed JSON/handle arguments and explicit
  release. Callable interfaces and default callable exports are recognized
  from `.d.ts`; generated programs release all remaining handles at shutdown.
  Fine-grained escape-based early release remains future work
- A fully general ABI-description format. Version 4 covers string layouts,
  recursive object/string/number-array result ownership, common LLVM calling
  conventions, ordinary or packed aggregate returns, and explicit scalar
  and recursively nested object-return field offsets/alignment, including
  boolean and signed/unsigned integer bitfields, explicit direct register
  classes for aggregates up to 16 bytes, and integer representations for
  number varargs. Boolean arrays are copied to contiguous C `int32_t` elements
  before the call because Thaw's native boolean-array stride differs
- Full Node/V8/libuv behavioral compatibility behind the N-API ABI. Thaw now
  exports the complete Node-API v10 symbol surface used by the current headers,
  plus the implemented experimental SharedArrayBuffer/finalizer/module-file
  entry points, but provides its own value model, worker pool, event polling and
  request-scoped lifetime rules rather than embedding Node. The async-work lifecycle, including
  `napi_cancel_async_work`, is supported by a bounded shared worker pool.
  UTF-8, Latin-1 and UTF-16 string creation/extraction follow N-API length,
  truncation and null-termination rules. Signed/unsigned 32-bit and signed
  64-bit number constructors share the existing JavaScript-number conversion path.
  Date values preserve millisecond timestamps and report JavaScript object type.
  Signed and unsigned 64-bit BigInt creation/extraction preserves low bits and
  reports whether conversion was lossless. Arbitrary-precision little-endian
  word arrays support sign, size queries and capacity-limited extraction.
  Scalar, string, Date, BigInt, collection, Buffer, ArrayBuffer and view
  extractors reject handles originating from a different Env.
  ArrayBuffer and all N-API TypedArray element kinds retain shared backing
  storage, byte offsets, alignment and bounds; Buffer remains a Uint8Array view.
  DataView uses the same backing storage with unaligned, byte-bounded views.
  External ArrayBuffer and Buffer values retain caller-owned memory without a
  copy, participate in views, and invoke their finalizers once at Env teardown.
  Plain external values likewise preserve their data pointer and invoke an
  optional finalizer with its original hint once at Env teardown.
  External-memory adjustments are tracked per Env with checked signed totals.
  ArrayBuffer detachment zeroes data/length for the backing value and existing
  TypedArray/DataView views while preserving external finalizer ownership.
  Async contexts validate resource names and Env ownership, reject double
  destruction, and are checked by callback scopes and `napi_make_callback`.
  Deferred Promise handles remain Env-owned after settlement so repeated
  resolve/reject calls fail safely, and settlement values must share the Env.
  Async-work handles likewise remain Env-owned after deletion, making foreign
  Env access and repeated queue, cancel or delete operations safe state errors.
  Async-work and thread-safe-function creation validates supplied async
  resources, resource-name strings and callback functions against the Env.
  Thread-safe-function handles retain stable process-lifetime identity after
  finalization, so subsequent calls return `napi_closing` instead of touching
  freed memory.
  N-API reference handles are retained by their Env after deletion, allowing
  repeated delete/get/ref/unref calls to reject safely without use-after-free.
  Async cleanup handles also retain stable identity after removal, making a
  repeated remove return `napi_invalid_arg` safely.
  Wrap, finalizer and object type-tag APIs reject objects owned by another Env
  before reading or mutating identity metadata.
  Named, generic and descriptor-based property APIs similarly require the
  object, key and assigned value handles to belong to the calling Env.
  Element access, property-name enumeration and script evaluation apply the
  same ownership check before object, value or source inspection.
  Buffer views, Error creation, fatal exceptions and callback/constructor
  return values also reject handles crossing Env boundaries.
  Core value creators validate result pointers before arena allocation or data
  output writes; Symbol descriptions must belong to the creating Env.
  Raw string, word and function-name inputs are read only after the Env and
  result handle have been validated.
  Property and class descriptor batches are fully preflighted before applying
  the first property or allocating a constructor/prototype.
  Accessor getters normalize null callback returns to `undefined` and reject
  non-null return handles that do not belong to the active Env.
  Object seal/freeze integrity levels apply consistently to named properties,
  generic property keys, deletion, functions and array elements. Descriptor
  writable/enumerable/configurable bits are retained for defined properties.
  Symbol values have identity independent of their descriptions and occupy a
  property-key namespace distinct from strings; JSON conversion omits Symbol
  keys in the same way as JavaScript.
  Property-name enumeration supports N-API string/Symbol skipping, descriptor
  attribute filters, own-key mode and numeric array-index conversion.
  Class instances retain their constructor relationship and expose the class
  prototype through `napi_get_prototype`. Instance property access follows a
  live prototype chain instead of copying class members, so late prototype
  updates, inherited accessors and own-only enumeration remain distinct.
  Native classes can attach and verify 128-bit identity tags with
  `napi_type_tag_object` and `napi_check_object_type_tag`.
  Arrays preserve holes separately from explicit `undefined`, including
  `napi_has_element`/`napi_delete_element`, stable length and JSON null slots.
  Promise detection and thread-safe-function context retrieval are supported.
  Asynchronous environment cleanup hooks retain addon libraries until their
  opaque handles are explicitly completed, with reverse-order invocation and
  pre-teardown removal.
  `napi_run_script` evaluates JavaScript in the embedded persistent QuickJS
  context, converts JSON-representable results into host values and reports
  JavaScript exceptions through the pending-exception channel.
  Node-API v9 syntax errors and the process-wide `Symbol.for` registry are
  exposed through their `node_api_*` entry points.
  Node-API v10 can create zero-copy Buffer views over bounded ArrayBuffer
  ranges and intern UTF-8, Latin-1 and UTF-16 optimized property keys.
  External Latin-1 and UTF-16 strings use the host's copied representation,
  report `copied=true` and immediately honor their native finalizers.
  Experimental Node 22 SharedArrayBuffer creation and detection provide
  non-detachable shared backing for TypedArray, DataView and Buffer views.
  `node_api_post_finalizer` defers GC-sensitive cleanup into the main-thread
  poller while retaining a live environment for ordinary Node-API calls.
  Addons can query their stable absolute `file://` load URL through
  `node_api_get_module_file_name`, including from generated top-level calls.
  Per-environment extended error information records failed argument, type,
  lifecycle and queue operations with stable status messages; successful calls
  do not erase the preceding error.
  Error, TypeError and RangeError creation/throwing share pending-exception
  tracking, and `napi_is_error` recognizes host-created errors.
  Own-property checks and property deletion are supported for objects and functions
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
- [`docs/design/native-addons.md`](docs/design/native-addons.md): chronological
  N-API host design and implementation record for `.node` addons

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
Element APIs work on arrays, objects, and functions, including inherited numeric
properties, accessors, descriptor configurability, and optional delete results.
Object coercion preserves the identity of every existing host object, including
arrays, buffers and views, promises, errors, dates, and functions.
Property-name enumeration follows JavaScript key order: integer indexes first in
ascending order, then strings and symbols in their respective insertion order.
N-API coercion covers JavaScript radix strings, infinities, negative zero,
arbitrary-width BigInt strings, array joins, and Symbol/BigInt conversion errors.
Native wrapping and finalizers accept every JavaScript object kind rather than
being limited to plain objects and class functions.
Value classification distinguishes N-API externals from objects and validates
environment, value, and result handles consistently across type predicates.
Generic named and Symbol properties now work across arrays, buffers, array-buffer
views, promises, errors, and dates with descriptors, enumeration, and integrity levels.
Numeric property descriptors participate in the same writable, enumerable, and
configurable name filters as string and Symbol properties.
Property and element deletion accepts an omitted result and distinguishes own
sealed properties from inherited or missing keys.
Buffer index properties share owned, external, or ArrayBuffer-view backing bytes,
including JavaScript Uint8 conversion, offsets, enumeration, and fixed-index deletion.
All eleven TypedArray kinds expose backing-memory index properties with numeric,
clamped, floating-point, and BigInt conversions plus offset and detach semantics.
Arrays and binary views expose JavaScript-compatible length, byte-length, offset,
and backing-buffer metadata, including array resizing and detached-view zeroing.
N-API functions retain their requested names and standard name/length descriptors;
classes expose immutable prototype links and constructor backlinks.
Created and thrown Error variants expose name, message, and optional code properties,
with shadowable names and JavaScript-style error stringification.
Strict equality compares BigInts by value while preserving object identity rules;
Date-to-number coercion returns the stored millisecond timestamp.
N-API references retain environment ownership, reject cross-environment use, and
check refcount underflow and overflow while allowing omitted count results.
Callback invocation validates receiver/function/argument ownership, propagates
pending exceptions, fills missing callback arguments with undefined, and honors
all object-valued constructor returns.
Instance checks walk the live constructor-prototype chain, including class
inheritance, primitive receivers, invalid constructors, and cycle protection.
Version reporting advertises the implemented Node-API v10 surface and returns a
process-stable host version descriptor with consistent argument validation.
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
delivery. Native shims which own a private/non-default loop can register it with
`thaw_napi_register_uv_loop` and unregister it before `uv_loop_close`; the same
drain loop drives every registered loop and includes it in liveness checks. An
actual private `uv_loop_t` timer test covers registration through clean
shutdown. Instance facts also follow statically named nested object properties,
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
as receiver, including named and namespace syntax. Arbitrary private loops
which are neither registered nor driven by their owner remain outside host
visibility, matching libuv's lack of a global loop registry.
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
ordinary properties, awaited spread sources, later overrides and awaited
values share that same source order. Replacing an existing key preserves its
original field position. Matching-type conditional expressions can also supply an object. Conditional
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
Dynamic indexed reads such as `object[key]` now work for uniform untagged
fixed-shape objects. Heterogeneous and already-nullable field sets remain a
separate typed-union problem.
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
