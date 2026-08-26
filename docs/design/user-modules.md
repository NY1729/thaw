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

The same QuickJS realm installs the commonly expected platform globals
`setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`, `queueMicrotask`,
`TextEncoder`, and `TextDecoder`. The Promise driver exhausts QuickJS jobs
before firing a due timer, so synchronous work, microtasks, and zero-delay
timers retain their JavaScript ordering. Timers forward trailing arguments and
can cancel themselves. Text encoding uses UTF-8 `Uint8Array` values and covers
`encodeInto`, replacement decoding, BOM removal, and fatal decode mode;
streaming decode remains an explicit error.

## Compilation model

The CLI parses every canonical file path once and visits dependencies before
their importers. A second import of the same file reuses the first graph node.
An import edge to a node currently being visited is reported with the complete
cycle rather than recursing indefinitely.

Before all AST bodies are joined, module-local function, variable and interface names are
rewritten to stable compiler-private names. Imported aliases map directly to
the dependency's rewritten export. Expression reads, assignments, calls, generic
type references, interface references, declarations, and re-exports consequently
agree on one symbol. Module-scoped `const` and `let` initializers lower to typed
HIR globals and run in dependency/source order through a guarded LLVM initializer
before `main` or the Lambda runtime starts. A binding imported by several modules
therefore has one storage cell and is initialized once. A general
`export default expression` becomes a private module-scoped constant, preserving
single evaluation for calls, arrays and fixed-shape object expressions. Executable
top-level expressions and supported control flow are interleaved with those global
stores in the same HIR initializer sequence, rather than being reordered around them.
Each generated initializer call is followed by a pending-exception check; a failure
branches directly to process cleanup, skips later initialization and user entry code,
and contributes a nonzero native process status.
Top-level object/array destructuring is normalized before graph symbol collection:
one private temporary evaluates the source, then typed field/index globals preserve
nested binding order. Exported patterns expose only user bindings, never the private
temporaries. Defaulted bindings use the native nullish-default lowering. Array rest
uses the typed non-mutating slice path, while object rest rebuilds a shallow fixed-shape
object from fields not consumed earlier in the pattern.
The entry module's `main` or `handler` keeps its ABI name. HIR then sees one
ordinary module, so its existing fixed-point inference, forward-reference
resolution, generic tuple specialization and specialization deduplication apply
across source-file boundaries without a second type system.

## Current boundaries

Generic/abstract classes, private or dynamically computed members, class expressions, top-level
declarations/statements outside the general HIR-supported subset, package multi-capture
package export keys, and full ESM live bindings are outside the current typed AOT subset.
Typed fixed-layout class declarations, including inheritance and named or anonymous default
exports, can be imported, namespace-imported and re-exported across user modules. Their
`implements` clauses structurally validate inherited and local fields, including specialized
generic interfaces. Typed static fields with initializers are module-scoped globals initialized
in source order. Derived classes share the declaring class's static storage through generated
accessors, including assignment, compound updates, prefix/postfix updates, readonly enforcement
and shadowing by a derived declaration. Static blocks execute in class-body order and can use
explicit class references plus `super` fields, methods and accessors; runtime constructor-valued
`this` inside those blocks remains an explicit gap. Constructors, instance/static methods,
`super(...)` and super methods expand tuple spreads with source-order preservation before their
fixed ABI call.
Native `instanceof` evaluates its left operand exactly once and checks the encoded fixed-layout
inheritance chain; runtime class values and union-polymorphic instance tests remain separate gaps.
Cyclic user-module graphs are diagnosed rather than executed. Missing
relative or registry modules report the importing file, line and column. These
are explicit compatibility limits, not silently rewritten semantics.

## Acceptance coverage

CLI integration tests compile and execute a graph covering extensionless and
directory resolution, named/default/aliased imports, re-exports, duplicate
imports, same-named declarations in different modules, forward references and
multi-argument generic specialization. Another executable graph covers exported
`const`/`let` initialization, a forward function call from an initializer and
shared mutation through an imported function. Default-expression coverage uses a
diamond import graph to verify dependency-before-importer ordering and exactly-once
initialization of the shared dependency. A destructuring graph exports object aliases
and array elements and consumes them from the entry executable. A separate test compiles and invokes a
multi-file resumable async `Json` Lambda handler against a mock Runtime API.
Further E2E coverage imports two registry packages with colliding export names
from that Lambda and verifies package initialization, async execution and the
Runtime API response. A generated executable also imports a registry bundle
which combines cancelled and repeating timers, microtasks, and a non-ASCII
`TextEncoder`/`TextDecoder` round trip.
Class graph coverage compiles named and anonymous default classes, a derived class whose base
is imported from another module, direct and namespace imports, and barrel re-exports into one
native executable, then executes inherited methods and reads constructor parameter properties.
