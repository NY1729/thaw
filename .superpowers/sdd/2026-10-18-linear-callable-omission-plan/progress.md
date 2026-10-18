# SDD ledger — plan: /tmp/2026-10-18-linear-callable-omission-plan.md
Approved audit roadmap; isolated clone /tmp/thaw-generic-arrow-bind. Implementation not dispatched yet.
## Preflight shared surfaces
| Tasks | Surface | Result |
|1/2| FnSignature native ABI, parameter prologues, invocation, inheritance, function values, tests | Single coordinated implementation produces frozen head for root/CI/reviewer; no concurrent code edits. |
## Task self-consistency
|Task|Check|
|1| Canonical tagged default params require matching body prologues and metadata-driven padding; deleting both module subset wrappers and named-value recursive dispatch solves both exponential sources. Externs remain physical. Constructor generated property writes must use narrowed bindings. |
|2| All final-head CI then merge ancestors preserving commits; inspect exact tested tree; remote cleanup authorized. |
Ruling: use existing undefined-capable native parameter ABI and callee default prologues instead of tentative lazy subset generation — lazy masks alone leave exponential named-value adapters and add a convergence protocol; canonical representation removes both sources — coordinated internal ABI changes require broad class/generic/native validation if wrong.
Root found named generic Pat::Assign is already unsupported; preserve rejection, existing generic optional/rest controls required. No named generic value expansion.
Base832c9dbe branchfix/linear-callable-omission created clean. Task1 implementation begins after priorstage reviewpassed. No sourceworkflow artifacts tracked.
Initial coordinated dispatch returned not implemented; all temporary source/test changes removed, cleanBASE832c9dbe. RED8optional=>264functions retained in initial-spike-report.md. No architecture contradiction found.
Ruling: split the implementation into metadata prerequisite, free-function canonical ABI, and class canonical ABI/cleanup — worker could not safely complete all coordinated paths in one dispatch; each smaller task has buildable compatibility gates and only the complete combined stage will publish — extra interim commits/reviews cost time, but prevent unsafe partial deployment.
## Revised preflight
|Tasks|Shared interface|Result|
|1/2|FnSignature omission mask/logical value inference|Task1 preserves physical ABI/wrappers; Task2 consumes authoritative mask.|
|1/3|class/inheritance/receiver masks|Task1 projects coordinates; Task3 consumes them with canonical class ABI.|
|1/4|reviewable committed metadata|final combined review consumes tested intermediate commits.|
|2/3|native call padding, optional ABI, remaining legacy class adapters|Task2 may use temporary ABI discriminator; Task3 removes it and all legacy wrappers before publication.|
|2/4|native free functions/resource regression tests|no partial-stage PR; final review includes combinedtree.|
|3/4|class ABI and deleted wrapper interfaces|complete tree frozen before final CI/integration.|
|Task|Self consistency|
|1|metadata only; default physical ABI unchanged; logical conversion retained; wrapper masks projected.|
|2|free functions canonical, legacy class paths intentionally retained; discriminator temporary.|
|3|all native class prologues and generated writes change together; legacy machinery removed.|
|4|only complete combined stage publishes, exacthead CI beforemerge.|
RevisedTask1: in progress, sameBASE832c9dbe. Implementation agent resumed with narrowed brief.
Ruling: transitional private omission/default wrappers use an empty source mask — their omitted slots are already baked in and remaining ABI inputs have no own wrapper family — this avoids nonexistent wrapper-on-wrapper adaptation; public metadata/inheritance still preserves and shifts syntax masks.
Task1 paused for stage2 CI repair with seven dirtymetadatafiles retained. Root stash saved (SHA /tmp/thaw-metadata-wip-stash.sha), fast-forwarded branch to newBASEd0f001dc, applied WIP cleanly. Agent resumed narrowed metadata task; stash retained as safety until restoration confirmed. No production source edits by root.
Task1 then completed as `5eb70bcd1961ea1a1480509d7f970ff81e19de39`; Luna review approved with no correctness findings. Root read the task report and verified HIR563, four focused LLVM controls, Clippy/fmt/diff. Full LLVM/workspace suites were not run. PR25 was merged before Task1 completion. Task2 brief is in `task-2-brief.md`; class ABI remains deferred to Task3.
Task2 completed as `39e77267d3184bf2e5b049c819239e48fe43f444`; Luna review approved with no findings. HIR566, eight focused LLVM controls (including direct/callable 8/17/65 growth, dynamic undefined/null, defaults/getter order, generic, class and FFI), Clippy/fmt/diff passed. Full LLVM/workspace suites were not run. Task3 is split into method ABI/prologues (`task-3-brief.md`), then constructor/inheritance ABI, then wrapper cleanup; no intermediate PR will be published.
Task3 method ABI/prologues is complete and approved by an independent Luna review. The review verified receiver/index mask transformations and bound/unbound default prologues; transitional `.bind()` omission and direct optional function-type assignment remain scheduled for Task5. Task4 constructor/initializer ABI (`task-4-brief.md`) and Task5 remaining-wrapper cleanup (`task-5-brief.md`) are prepared and strictly sequential. Task5 is the only stage that removes the residual class exponential growth and publishes this stage after a combined review.

Task4 constructor/initializer ABI is implemented and approved by an independent Luna review. Constructor and initializer slots now carry undefined-capable types for optional/defaulted source parameters; initializer masks shift past `this`; explicit initializer prologues resolve defaults and destructuring before statements, while fields and parameter-property writes retain post-super timing. Transitional constructor/initializer wrappers now only adapt omitted physical slots and are isolated for Task5 removal. Focused LLVM coverage verifies undefined/null, defaults, optional omission, parameter properties, one-time getter/default evaluation, rest, explicit super, implicit multilevel inheritance, and field timing. Fresh root checks passed: full `thaw-hir` (567), 35 constructor-filtered LLVM tests, focused super/rest LLVM controls, Clippy, fmt, and diff checks. Full LLVM was not run; no growth checks were added before Task5.

Task5 removes the remaining class constructor/initializer/method wrapper signatures, bodies, inherited clones, symbol parsing/rewrites, subset enumeration, recursive callable mask dispatch, and misleading omission-count cap. Constructor/super/method calls now pad the canonical ABI from source omission masks; bound callable closures preserve shifted masks, and physically compatible named callable methods use canonical signatures. HIR count/debug-size guards pass at 8/17/65 optional/default class slots; negatives cover required `number | undefined` and `T | undefined`. The `.bind()` omitted-argument regression was observed failing before its canonical closure-mask fix. Final checks passed: thaw-hir 569, focused LLVM class/bind/call/apply/rest/inheritance/super/receiver/generic/FFI controls, Clippy, fmt, and diff. See `task-5-report.md`; awaiting combined review/exact-head CI before publication.
