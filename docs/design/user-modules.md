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
packages remain explicit `--use` dependencies; a bare module specifier in user
source is therefore diagnosed instead of silently selecting Node resolution.

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

Namespace imports, classes, anonymous default functions, runtime top-level
statements, Node package resolution, and full ESM live bindings are outside the
current typed AOT subset. Cyclic user-module graphs are diagnosed rather than
executed. These are explicit compatibility limits, not silently rewritten
semantics.

## Acceptance coverage

CLI integration tests compile and execute a graph covering extensionless and
directory resolution, named/default/aliased imports, re-exports, duplicate
imports, same-named declarations in different modules, forward references and
multi-argument generic specialization. A separate test compiles and invokes a
multi-file resumable async `Json` Lambda handler against a mock Runtime API.
