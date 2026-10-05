# Internal Promise arena slot source review — unit496

Scope: live scratch `/tmp/thaw-throw-provenance-head494/candidate`; managed baseline f4e2faf98b6973306ac634f2d78968904b846480. Source inspection only. No product execution, tests, compilation, or product edits. This is a bounded intermediate review, not full native throw acceptance.

## Read pins

- runtime/promises.rs: 8e2680eed2d69feff646b63ba7276c24590e6d39aee509f7dd573350e5586305 for full helper/reset read. Subsequent author naming edit produced e322721353ad122e435466952416a6af213bf9861081a96d942643efadfc5330; inspected retain/release excerpt now consistently names internal_references. Live tree is not frozen.
- runtime src/lib.rs: a1d329dda73189b4d0f7fe2efe7dd01c51c70388d50a3d49085d2ecf1acb81aa.
- runtime src/tests.rs: 8bf16258a9a89bb633a553306ab872ebabab6b46c40754af44e999c948981cd7.
- LLVM hir_codegen/runtime_declarations.rs: f30c349f31c38cb27ee95730ede80206f715f2d4e86dbac2222772f0c3f4905a.
- arena src/lib.rs: eeb6f85309d710cfe2b16e2ef4024ed2402e9ea067f9dfced78dde5572bcb21f.

## Conclusions

The concrete early same-pointer bypass is corrected: promises.rs1740+ validates slot record owner and previous pointer before the no-op return. An untracked non-null self-store now fails before any acquire or publication. The new negative fixture expects zero and unchanged counts; it is source coherent and unrun.

Replacement acquires the replacement internal token before modifying REFERENCES, the map, or the physical pointer. Checked count overflow fails without changing the old slot. Graph/map/pointer publication precedes old-token release. The same-pointer path cannot inflate tokens. The declared LLVM ABI is i8 return and three pointer arguments, matching the Rust C function.

The map key is the physical slot, while its record stores the containing owner and Promise. Two slots sharing a Promise have two tokens and two graph edges: arena replace_reference removes one matching child occurrence, preserving the other. Reset tests reclamation of the allocation containing the interior slot address. It removes both descriptor and slot records under their respective map borrows, then releases all tokens outside those borrows. This avoids destruction while either owner map is borrowed. With external roots removed after the last external handle, descriptor/arena-slot cycles have internal tokens without an unconditional external arena pin; dead records can therefore release those tokens during reset.

No additional concrete helper defect was found within its documented unsafe contract. Owner and slot must belong to the same live allocation; slot must be initialized. Merely checking both addresses are arena allocations is not a proof they share an allocation, but this is explicitly the caller safety obligation, not a newly claimed implementation defect. A non-null raw preexisting slot is intentionally rejected unless tracked; first stores must initialize null and use the helper.

## First caller inventory and remaining gate

A subsequent bounded read found store_arena_promise_slot in statements.rs2+, async frame parameter/capture stores (async_frames/codegen.rs157,793,1106), closure promotion stores (values/closures.rs18,119), and assignment dispatch (values/expressions.rs139,141). The shared emitter checks the helper result and throws before continuing; it does not separately publish the Promise pointer on failure. This inventory is not a complete caller review or immutable hash pin for those actively edited files.

Full ownership closure remains HOLD: synchronous/global bindings, returns, aggregate payloads, borrowed catch assertion escape, and all replacement/final release paths require the implementation-ready alias plan. The helper alone cannot make borrowed aliases escape safely. Do not claim that every producer/callee/consumer is wired from this report.

## Additional unrun controls

Add two distinct slots in one rooted owner sharing one Promise: replace one, verify the second edge/token survives, then reclaim both. Verify a rooted owner survives reset and only releases after dropping its root. Exercise a Promise→frame/cell→Promise cycle combined with a native reason descriptor, confirming all internal records and Boxes disappear after external owners vanish. Exercise null replacement, wrong tracked owner, and overflow rejection with graph/map/slot unchanged. Existing balanced replacement and untracked self-store fixtures cover only a subset. All controls remain unrun.

Marker: BOUNDED_SOURCE_REVIEW_NO_NEW_HELPER_DEFECT; FULL_CALLER_OWNERSHIP_HOLD.
