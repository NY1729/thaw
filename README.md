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

Known gaps against Node semantics:

- A value typed `any` that aliases a statically typed native array (`T[]`) is a live view.
  A native array or record written into it is stored as that same object. A native `any`/JSON
  array of numbers, strings or booleans, or an `any`/JSON object whose fields are such values,
  stays linked to the typed slot it was written into (`v[0] = j; j.push(1)` is visible as
  `nat[0]`, and typed writes are visible through `j`); this holds at every nesting level
  (`number[][][]`, records with array or record fields). Values owned by the JS realm (objects
  returned by npm packages, and arrays or objects inside an `any` object literal, which are real
  JS values) are stored as a native copy, because a native element slot cannot hold a JavaScript
  reference and QuickJS cannot redirect existing references: the original object and the stored
  element do not alias afterwards, and a plain object written into a class array (`C[]`)
  becomes a `C` with the object's fields. Declare the array `any[]` to keep references; run with
  `THAW_WARN_COPY=1` to get a one-time note on stderr the first time such a copy happens.
- Frozen native arrays always throw `TypeError` on writes (code is strict).
- A bare call of a `this`-using function expression with a simple unconditional property
  access reports Node's property-specific `TypeError`, including reads in an assignment's RHS.
  Complex bodies (branches, nested closures, suspended async functions) and extracted
  object-literal methods still report `TypeError: Incompatible function receiver`.
- Decorated classes are live views in local `any` slots; arguments handed to npm or ambient
  functions are still snapshots.
- A `new Map()`/`new Set()` written in an `any` literal (a field, a nested field or an array
  element) is a real collection that `get`/`set`/`add`/`has`/`delete`/`forEach`, spread and
  `console.log` handle. A native `Map`/`Set` variable that becomes `any` (`const a: any = existing`,
  `{ m: existing }`, `[existing]`, an argument or return of a user function) is a live view: both sides
  see every change and `a === existing` holds. A view serves keys of type `number`, `string` or `any`
  and values of type `number`, `string`, `boolean` or `any` (arrays, objects and dictionaries read back
  live but are read-only through the view); storing another type through it throws a `TypeError`.
  Iterators (`keys`, `values`, `entries`, `for..of`) and `forEach` observe additions, deletions and
  updates during traversal. Iterator helpers preserve lazy state and reference identity through
  chained calls, and close their sources on early return or failure. Collections with statically typed object keys, or whose values are
  themselves Maps or Sets, stay snapshots.
- An `any`/JSON array that contains itself no longer crashes: `String`, `join` and `+` render the
  self reference as an empty string like Node. Written into a typed slot (`number[][]`) the self
  element becomes `NaN`, and nesting deeper than 32 levels is stored as a plain copy.
- Supported native method results preserve their identity even when freshly allocated
  (`v.inner.get().push(x)`, `o.cfg().x = 1`): a native array, class instance or supported Map/Set a class method
  hands back (declared as `T[]`, a class type or a collection type), and a JavaScript object stored in the receiver
  within two levels (a scan of at most 256 own data properties). Other results are data
  snapshots: a fresh JavaScript array or object, a result declared `T[] | undefined`, a tuple
  or a `Promise` of an array (`async get()`), and an owned value stored deeper or further down than
  the scan reaches. Mutating a snapshot does not reach the owner. Native class accessors remain
  live through `any`, including inherited getters and setters. Map/Set fields and callback parameters
  accept native views with matching key/value types; JavaScript-owned collections and incompatible
  native collection types throw `TypeError` rather than being copied into native storage.

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
