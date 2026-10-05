# Legacy callback result proof: source planning

Source authority: managed HEAD492 plus `/tmp/thaw-luna-jit-result-chain/candidate`; no code edits or execution.

The candidate `runtime/arrays/callbacks/jit.rs:36` classifies only the final comma token. This cannot represent conditional join, local get, result block, early return, loop or capture provenance. Also `:127` splits off the final token for every callback before determining whether a recognized descriptor exists: root has already identified that separate destructive suffix bug.

A proof from parsed NumericProgram alone is currently impossible. Parser source erases result representation: `compiler/program/parser.rs:1115` maps a/b/s arguments to the same Argument variant; `:1188` maps typed local prefixes to LocalGet; object/tuple field parsing merges string/object/tuple into one machine kind. Do not call that enum a typed IR merely because it has some specialized operations.

Minimum sound repair preserving legacy features:

1. Remove a suffix only when it is a recognized result marker. Unknown cbresult marker is an error; an unmarked callback preserves its complete expression.
2. Keep the existing parser as syntax authority, but preserve the semantic type annotations it currently discards, either attached to parsed operations or in a parallel kind stream created during that same parse. Avoid a second hand-maintained token recognizer with starts_with guesses.
3. Track abstract value representation through the existing stack and locals. Arithmetic/conversions have fixed result representations; duplicate/drop preserve or remove them; typed arguments/captures/field reads seed actual producer representations; local writes/reads transfer representations. The callback caller's argument kind remains a boundary input, not guessed Number for every Argument.
4. Mirror existing control structure: ConditionalStart/Alternate/ShortCircuitEnd have saved base stacks and both result stacks; Select joins its two values; result blocks join ResultReturn exits and fallthrough; EarlyReturn joins terminal paths; try/catch, switches and loops join reachable state and local writes. Loops require fixed-point merging of the finite kind domain or producer descriptors sufficient to prove stable representations.
5. Same representations can join directly. Raw Number versus raw String cannot be relabeled Dynamic: a Dynamic result requires actual tagging/boxing of each arm. If legacy bytecode legitimately permits mixed representations, normalize at the return producers to a common tagged result, preserving semantics and operand evaluation, rather than guessing or rejecting previously supported programs. Missing type provenance must be supplied at producer/caller boundaries. Explicit descriptors must agree with proved byte representations before string dereference or numeric normalization.

This is not a one-line last-opcode fix. Reuse codegen's structured branch/return depth contracts (`codegen.rs:2042–2095,2601–2690`) to minimize drift. The existing analysis.rs only computes required_args and has no semantic kind analysis to reuse.

Unrun controls: unmarked arithmetic final op preserved; marked expression strips marker only; unknown marker fails; conditional returning strings; local String set/get; branch assignment to same local; result block with multiple string returns; numeric and boolean capture return; nested conditional plus conversion; dynamic tagged arm joins; loop early returns; valid legacy output remains usable in every map/filter/find/reduce consumer. Assert unknown/contradictory descriptors fail before dereferencing raw bits. No result or compatibility claim is established by these source fixtures.
