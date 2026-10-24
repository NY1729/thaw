# Thaw

Thaw is an experimental ahead-of-time compiler for a typed subset of
TypeScript. It lowers TypeScript through SWC and Thaw HIR to LLVM, then links a
native executable.

Thaw focuses on small server binaries, AWS Lambda, npm interoperability, and
fast builds. It is not yet a drop-in replacement for Node.js or `tsc`.

## Requirements

- Rust and Cargo
- LLVM 22
- A C linker available as `cc`
- npm when installing packages or building Vite frontends

## Quick start

```sh
cargo build --release -p thaw-cli
target/release/thaw build app.ts -o app
./app
```

Create `app.ts`:

```ts
function main(): void {
  console.log("Hello from Thaw");
}
```

### Run an HTTP server

Thaw's built-in `node:http` support compiles a request handler to
native code -- no npm install required. Create `app.ts`:

```ts
import { createServer } from "node:http";

function main(): void {
  const server = createServer((request, response): boolean => {
    response.setHeader("Content-Type", "text/plain");
    return response.end("Hello from Thaw's HTTP server\n");
  });
  server.listen(3000);
}
```

```sh
target/release/thaw build app.ts -o app
./app &
curl http://127.0.0.1:3000/
```

This built-in server handles one request per connection (each response
closes the connection) rather than keeping it alive for further
requests -- for keep-alive, streaming responses, or a framework's own
routing (Express, Fastify, Hono, ...), install it as a package instead;
see [Projects and npm packages](#projects-and-npm-packages) below.

Use `--static` on Linux to request a fully static ELF:

```sh
target/release/thaw build app.ts --static -o app
```

`--icu4c` opts into a system ICU4C backend (loaded at runtime with
`dlopen`) for the pieces ICU4X cannot reproduce: `Intl.Collator`'s
`usage: 'search'`, `Intl.PluralRules`'s `selectRange`,
`Intl.DateTimeFormat`'s `month: 'narrow'`/`weekday: 'narrow'` and
`formatRange`/`formatRangeToParts`, and `Intl.NumberFormat`'s
`formatRange`/`formatRangeToParts`. It is off by
default: the default build has no ICU4C dependency, and those fall back
(`usage: 'search'` to sort behavior, `selectRange` to `"other"`, narrow
widths to short, the range methods to a shared/startRange/endRange partition
of the two endpoints with a locale-independent separator) when the feature is not selected or ICU4C is unavailable
at runtime.

Inspect a binary without running it:

```sh
target/release/thaw inspect app
```

## Projects and npm packages

Thaw can use a project directory and its `package.json`:

```sh
target/release/thaw install ./my-app
target/release/thaw build ./my-app
```

Packages may also be registered explicitly:

```sh
target/release/thaw registry add zod@3.23.0
target/release/thaw build app.ts --use zod -o app
```

Generated packages such as Prisma Client can be imported from an existing
`node_modules` directory:

```sh
npx prisma generate
target/release/thaw registry add @prisma/client --from-node-modules node_modules
```

Run a package's own command without installing it, like `npx` / `bun x`:

```sh
target/release/thaw x cowsay hello
target/release/thaw x prettier@3 --write app.ts
target/release/thaw x --no-install eslint .
```

`thaw x` resolves a matching installed `node_modules/<package>` first,
otherwise installs the package into a cache under `$XDG_CACHE_HOME/thaw/x`
and runs its `bin` entry, forwarding arguments and the exit code. Fixed
versions are reused while tags, ranges, and unversioned requests are refreshed
through npm. `thaw x -p <package> <command>` selects a specific command
(`npx -p` style); repeat `-p` to expose several packages in one execution.

A bare `import` that is not in the registry or an adjacent `node_modules` is
fetched from npm during the build, so no prior `thaw install`/`registry add`
is needed. Pass `--no-install` to keep the build offline.

Run a project script with `thaw run` (`bun run` style):

```sh
target/release/thaw run                 # list the project's scripts
target/release/thaw run build           # run one (a source-file script is
                                        # compiled by thaw; anything wider
                                        # goes through `npm run`)
target/release/thaw run start -- --port 3000   # forward arguments
```

For a Vite frontend, pass its project directory:

```sh
target/release/thaw build server.ts --vite ./frontend -o app
```

The generated frontend assets and backend are packaged together. Native
add-ons can be embedded or placed beside the executable with
`--external-native`.

## Current capabilities

- Native TypeScript functions, classes, closures, collections, exceptions,
  async/await, and Promise combinators
- Multi-file user modules
- npm package registry and package-directory builds
- Node.js-compatible APIs used by the included server examples
- N-API add-ons, callbacks, async work, thread-safe functions, and classes
- Hono, React/Vite, Prisma/PostgreSQL, SQLite, and sharp integration examples
- C ABI bridges through `--bridge`, `--link`, and `--ffi-metadata`
- AWS Lambda entrypoints

Statically understood code is compiled through HIR and LLVM. Package code that
requires general JavaScript semantics uses the embedded QuickJS compatibility
layer. `thaw inspect` reports whether QuickJS or N-API is present and explains
known fallback reasons.

Native strings preserve embedded NULs and UTF-16 lone surrogates. Their C ABI
remains pointer-based; pointer-length FFI signatures carry the full byte length.
Lambda invocation cleanup traces registered globals and retains the arena
generations containing reachable native values, so module state survives warm
invocations. Unreachable generations are reclaimed; retained generations may
also contain unused allocation space.

## Limitations

Thaw supports a growing practical subset of TypeScript and Node.js, not every
language or runtime feature. Fully dynamic JavaScript, complete ESM semantics,
all Node built-ins, and all npm packages are not guaranteed. Third-party
packages and native add-ons remain subject to their own platform and license
requirements.

Named generic function declarations can be specialized at a call site through
immutable aliases and alias chains, including `bind`, `call`, and `apply`;
`typeof` reports these templates as functions. Bound `this` and leading
arguments are evaluated once when `bind` is created. `apply` supports statically
known tuples and inline array literals, plus the existing rest-array
normalization; arbitrary runtime-length arrays are not generally supported.
Named generic operations with an explicit generic `this` receive a targeted
diagnostic. Arbitrary runtime-polymorphic generic function values, including
unspecialized values stored in arrays or objects or returned without a
monomorphic context, remain unsupported, and mutable unannotated aliases are
rejected. Explicit type arguments and monomorphic typed contexts remain
supported.

Arrays, records and collections exposed through `any` use dynamic storage from
allocation. JavaScript-owned values retain reference identity when written through an
alias, including plain objects in class-annotated arrays, self references and deep graphs.
Native classes retain their actual class and private-field brands. Fresh, optional, tuple,
callable and async method results preserve dynamically represented references across awaits.
Programs whose values remain native keep their unboxed storage. A registry package function
whose arguments or result cross a dynamic-storage boundary (for example an `any` value passed
to a `number[]` or `Record<string, number>` parameter, or a `number[]` that the program also
exposes as `any` while the function mutates it)
runs through QuickJS instead of the JIT, because the JIT's dense arrays and dictionaries cannot
share storage with dynamic values; its output is unchanged. Public native FFI layouts
stay fixed; incompatible dynamic arguments throw `TypeError` before native code is invoked.
Compatible JavaScript-owned `Map` and `Set` arguments are copied into temporary native
collections for the call; compatible native collections keep the zero-copy path. C-side
collection changes are not copied back to a JavaScript-owned collection. The declared key
and value types must have a supported layout; JavaScript-owned collections with unsupported
layouts, including object or class keys and bare `JsValue` entries, throw `TypeError` before
the C function runs. JavaScript `Map` keys and `Set` elements support `number`, `string`, and
`Json`/`any` layouts. `Json`/`any` entries retain JavaScript object identity, so copying the
collection does not deep-clone its contents.
Native array and record returns are copied into owned internal storage when dynamic storage
is required; later dynamic writes do not change the original foreign memory. Class annotations
do not give an unbranded foreign object a native class brand.

At supported decorated-class boundaries, including typed-JavaScript calls and
Registry calls through `any`, `unknown`, `object`, method-bearing structural
declarations, or an explicit `Json` alias, JavaScript sees a live view of the
native instance: repeated passage preserves identity, prototype methods work,
and JavaScript field writes update the native instance. Data-only structural
parameters follow their declared source contract and may materialize the
declared native layout; this does not imply JavaScript object identity. An
unsupported or incompatible layout may be copied or rejected.

Bare calls of `this`-using function expressions report Node's property-specific `TypeError`
at the property access that executes, including reads in an assignment's RHS. Extracted
object-literal methods follow the same rule. Object-literal methods returning `this` preserve
owner identity across chained calls, with mutations visible on the original object.

Known gaps against Node semantics:

- A default parameter that reads itself or a later parameter outside a nested closure is a
  compile error, as in TypeScript (TS2373); Node 22 throws a `ReferenceError` at run time instead.
- Frozen native arrays always throw `TypeError` on writes (code is strict).
- `structuredClone` of a host array longer than 16,777,216 elements throws `RangeError`: arrays
  are dense, so a huge sparse length cannot be materialized. Node returns a sparse array.
- A `structuredClone`d `Map`/`Set` is a native collection: its methods are callable, but
  `typeof m.get` reports `undefined` (Node: `function`).
- `structuredClone` of a function throws `DataCloneError` with a generic message
  (`function could not be cloned.`); Node quotes the function source.
- Native FFI collection parameters require an authenticated native collection or a
  JavaScript-owned `Map`/`Set` whose declared key and value types have a supported
  copy-in layout. Unsupported JavaScript-owned collection layouts throw `TypeError`
  before native code is invoked.
- Writing the same linked array through both a JavaScript (`any`) view and a typed native
  view within one expression: the JavaScript-side write wins. Node applies the last write in
  statement order.
- Writes to a frozen native array always throw `TypeError`. Node ignores them silently in
  sloppy-mode scripts (CommonJS without `'use strict'`). Thaw code is always strict.
- Class instances passed to npm or ambient functions are snapshots: later changes made in
  Thaw are not visible to npm code, and the reverse. Node passes live objects.
- `new.target` is supported only inside a top-level, non-async, non-generator function
  declaration; class constructors, methods, nested and exported functions are a compile error.
- `f.length` is supported only on directly named, declared functions. Aliases, arrow functions
  and methods are a compile error.
- A constructor that returns an object is ignored: `new` still yields the constructed instance
  of the class.
- `Error` values are tagged strings, so extra own properties, `cause` and subclass naming in
  `util.inspect` differ from Node.
- `class X extends Array` and `class X extends Map` are compile errors.
- A class passed through an `any` parameter is a JavaScript copy: it has no shared static state
  with the original, and an instance does not satisfy `instanceof` of a parent class.
- An uninitialised `number` or `boolean` class field reads as `0` or `false` and is included in
  `Object.keys` and `JSON.stringify`; Node reads `undefined`.
- `localeCompare` compares by code point unless built with `--icu4c`.
- `Buffer`/`Uint8Array` `slice` and `subarray` copy; writes through the result are not visible
  in the original. Node shares the underlying memory.
- `BigInt` is 64-bit: values outside the signed 64-bit range wrap instead of growing.
- `Date.prototype.toLocale*String` and `Number.prototype.toLocaleString` on native values ignore
  the locale and options (`Intl.DateTimeFormat`/`Intl.NumberFormat` do honor them).

## Examples

- [Full-stack server](examples/fullstack-server)
- [React and Vite](examples/react-vite-fullstack)
- [Hono, React, Prisma, and SQLite](examples/hono-react-prisma-board)
- [Hono, Prisma, and PostgreSQL](examples/hono-prisma-postgres)
- [Hono and sharp](examples/hono-sharp)
- [AWS Lambda](examples/hello-lambda)

## Testing

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
target/release/thaw compat tests/typescript-compat.json
target/release/thaw node-compat tests/typescript-runtime-compat.json
target/release/thaw node-compat tests/node-compat.json
target/release/thaw node-compat tests/builtins-compat.json  # ECMAScript built-ins (needs Node >= 22)
```

A single, trend-tracking snapshot of how much of the target surface is
covered:

```sh
target/release/thaw completeness            # fast: TypeScript subset + static repo counts
target/release/thaw completeness --with-node  # also builds/runs every Node-compat case (slow)
target/release/thaw completeness --json       # machine-readable, for tracking over time
```

`completeness` measures the in-scope TypeScript cases the compiler can
lower (excluding documented `out-of-scope` cases, from
`tests/typescript-compat.json`) and, with `--with-node`, the share of
Node-compat cases whose behavior matches real Node. The reported
`indexPercent` is the mean of those measured axes only -- a rough trend
indicator, not a precise statement of completeness; the repository counts
it also prints (pinned npm integration tests, examples, design docs) are
not scored.

CPU and peak-memory measurements:

```sh
cargo build --release -p thaw-cli
benchmarks/cpu/run.sh target/release/thaw
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option. Third-party dependencies and code compiled or bundled by Thaw
retain their own licenses.
