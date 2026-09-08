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

Use `--static` on Linux to request a fully static ELF:

```sh
target/release/thaw build app.ts --static -o app
```

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

## Limitations

Thaw supports a growing practical subset of TypeScript and Node.js, not every
language or runtime feature. Fully dynamic JavaScript, complete ESM semantics,
all Node built-ins, and all npm packages are not guaranteed. Third-party
packages and native add-ons remain subject to their own platform and license
requirements.

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
```

CPU and peak-memory measurements:

```sh
cargo build --release -p thaw-cli
benchmarks/cpu/run.sh target/release/thaw
```

## Design documents

- [User modules](docs/design/user-modules.md)
- [Registry and npm interop](docs/design/registry.md)
- [npm interop gaps](docs/design/npm-interop-gaps-2026-09.md)
- [Native add-ons](docs/design/native-addons.md)
- [C ABI bridge](docs/design/bridge.md)
- [Async and await](docs/design/async-await.md)
- [Exceptions](docs/design/exceptions.md)

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option. Third-party dependencies and code compiled or bundled by Thaw
retain their own licenses.
