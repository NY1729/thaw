# User TypeScript modules

## Scope

`thaw build entry.ts` follows relative imports before HIR lowering. Resolution
accepts explicit files, an omitted `.ts` extension, and directory `index.ts`:

```ts
import value from "./value";
import { makePair as pair } from "./model.ts";
import { helper } from "./utilities";
```

Named and named-default function exports, interface exports, local export
lists, named re-exports, and `export *` participate in the graph. Registry
packages may be imported by bare name after `thaw registry add`; the CLI
automatically generates the same bridge and module initializer previously
requested with `--use`. Named, default and namespace imports map to the
package-qualified shim symbols, including two packages exporting the same
function name. `--use` remains available for source that calls package exports
as globals.

When a package function's `.d.ts` parameters and result are representable as
`number`, `string`, `boolean`, `Json`, `number[]`, or a fixed object composed of
those types, imports target a typed dynamic-call declaration. LLVM constructs
the positional JSON array, invokes QuickJS or N-API through the error-aware
result ABI, and converts the result back to its declared native layout. Thus
`add(20, 22): number` needs neither `JSON.parse("[20,22]")` nor `Number(...)`.
Unsupported declarations retain the explicit one-`Json`-array fallback ABI.

`node:path`, `node:util`, `node:process`, and `node:buffer` resolve to the small
QuickJS polyfills also used by bundled npm dependency graphs. Their deliberately
dynamic declarations currently retain that explicit `Json` ABI. This is
intentionally smaller than Node's complete core module API.

## Compilation model

The CLI parses every canonical file path once and visits dependencies before
their importers. A second import of the same file reuses the first graph node.
An import edge to a node currently being visited is reported with the complete
cycle rather than recursing indefinitely.

Before all AST bodies are joined, module-local function and interface names are
rewritten to stable compiler-private names. Imported aliases map directly to
the dependency's rewritten export. Calls, generic type references, interface
references, declarations, and re-exports consequently agree on one symbol.
The entry module's `main` or `handler` keeps its ABI name. HIR then sees one
ordinary module, so its existing fixed-point inference, forward-reference
resolution, generic tuple specialization and specialization deduplication apply
across source-file boundaries without a second type system.

## Current boundaries

Classes, anonymous default functions, runtime top-level statements, package
subpath exports, and full ESM live bindings are outside the current typed AOT
subset. Cyclic user-module graphs are diagnosed rather than executed. Missing
relative or registry modules report the importing file, line and column. These
are explicit compatibility limits, not silently rewritten semantics.

## Acceptance coverage

CLI integration tests compile and execute a graph covering extensionless and
directory resolution, named/default/aliased imports, re-exports, duplicate
imports, same-named declarations in different modules, forward references and
multi-argument generic specialization. A separate test compiles and invokes a
multi-file resumable async `Json` Lambda handler against a mock Runtime API.
Further E2E coverage imports two registry packages with colliding export names
from that Lambda and verifies package initialization, async execution and the
Runtime API response.
