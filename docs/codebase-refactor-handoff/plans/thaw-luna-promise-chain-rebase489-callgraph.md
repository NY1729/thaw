# Promise-chain HEAD489 rebase closure (source-only)

Managed read-only baseline observed: `bd4565fa048400309f923681614839b3951192a6`.
The mutable implementation stays at `/tmp/thaw-luna-promise-chain-successor`.
No product/test/compiler/runtime command was run for this mapping.

## Required dependency closure

1. **Producer ABI and compiler callsites** — `thaw-runtime/src/runtime/abi.rs`
   (private five-argument transform type while preserving the public four-
   argument ABI); `thaw-llvm/src/hir_codegen/runtime_declarations.rs`;
   `thaw-llvm/src/hir_codegen/promises.rs` (`compile_promise_then`, both-handler
   caller, adapter projection/callback/settlement edges). Both `.then` and
   `.catch` and both-handler forms call the same runtime constructor.
2. **One-shot delivery, context/root lifetime, source lane** —
   `thaw-runtime/src/runtime/promises.rs` and `thaw-runtime/src/lib.rs`.
   Required runtime units are `PromiseChainState`, `PromiseChainCallback`,
   `PromiseChainExceptionContext`, context reservation before output or
   subscription publication, state-root then callback-context child edges,
   `subscribe_with_cancel_and_completion`, the delivered subscription's
   independent source/completion shares, cancellation/drop, and the two
   `thaw_promise_queue_chain_*` handoffs. Existing managed HEAD has only the
   simple four-argument chain and lacks these subscription/completion fields.
3. **Queue ABI / exact owner retirement** — `thaw-std/src/json.rs` defines the
   104-byte reserved packet, LIFO enqueue, queue roots, cleanup tokens, and
   packet pop. `thaw-runtime/src/lib.rs` reports tag 4 through the native-text
   lane and releases a packet's byte-96 Promise share. The process-lifetime
   fallback literal is NUL-terminated borrowed tag-4 text (`original ==
   native_text`, byte 88 owner null); `NativeStr::from_ptr` and report-text
   consumers accept unregistered C strings. The callback's original packet
   remains a separate node from source and causal packets.
4. **Typed producer ownership and exception isolation** —
   `thaw-llvm/src/hir_codegen.rs` plus
   `thaw-llvm/src/hir_codegen/async_frames/codegen.rs`: ten-channel pending
   globals/roots; chain-active authority dispatch; reserve/activate saved
   snapshot; copy all typed channels and move NativeStr owner to packet byte
   88; clear/restore; exact reentrant defer helper; chain callback failure
   status branch; independent Json cleanup token; cause packet ordering.
   The managed HEAD has only primary/rejection/object channels and the old
   status-discarding path, so porting just the adapter cannot close ownership.
5. **Report registration / fatal completion** —
   `thaw-llvm/src/hir_codegen/entrypoints.rs`,
   `thaw-llvm/src/hir_codegen.rs` native and HTTP deferred drivers,
   `thaw-runtime/src/runtime/lambda.rs`, and `thaw-std/src/http.rs` fatal flag.
   Required only to ensure queued exact packets are drained once and fatal
   disposition reaches the C-main/HTTP/Lambda terminal path. Avoid unrelated
   HTTP request-context or callback-owner changes.
6. **Root substrate** — `thaw-arena/src/lib.rs` / `strings.rs` and runtime
   imports. Chain construction explicitly enables tracing before its first
   cell allocation/root registration; the runtime source control begins with
   tracing disabled, creates a pending chain, resets the arena, then verifies
   a callback can read its arena context.
7. **Unrun source controls** —
   `thaw-runtime/src/tests.rs` for fresh-thread tracing-off/context reset;
   `thaw-llvm/src/hir_codegen/tests/async/promises.rs` for the five-pointer
   adapter, callback original/cause packets, status-zero pending-vs-settled,
   and exact fallback tag-4 borrowed text.

## Keep out of this chain rebase unless a direct callgraph edge is proven

Do not transplant unrelated old-draft changes for generic explicit async
throws/rethrows, Promise `all`/`any`/aggregate owner redesign, HTTP request
context claims, HTTP handler cancellation, nominal graph projection, arbitrary
QuickJS/JIT helpers, or general dynamic-host callback ownership. Those files
may share containers, but their implementations are not required to make the
chain callback's one-shot packet/context contract work.

## Shared-owner collision list for private Sol review

- `thaw-llvm/src/hir_codegen.rs`: pending exception carrier layout, typed
  globals, cleanup cell offsets, packet serializer/restorer, graph/drain
  authority, and `branch_on_pending_exception` overlap.
- `thaw-std/src/json.rs`: Json owner tokens, packet queue and exact owner
  release contract overlap.
- `thaw-runtime/src/runtime/promises.rs`: Promise tag-7 share/result
  ownership, source shares, subscription completion and exact source packet
  overlap.
- `thaw-runtime/src/lib.rs` and `thaw-llvm/src/hir_codegen/entrypoints.rs`:
  terminal exact reporter and packet source-share release overlap.
- `thaw-arena/src/strings.rs`: registered length versus borrowed C-string
  behavior for static tag-4 text overlap.
- `thaw-std/src/http.rs`: shared unhandled/fatal flag only; no generic HTTP
  ownership edits should be pulled in by association.

## Rebase evidence

The independent review found that the old candidate's entire runtime and
compiler owners are older than HEAD489: `runtime/promises.rs` is 3965 lines in
the draft versus 1751 in HEAD; `hir_codegen.rs` is 3784 versus 1569. A
three-way probe shows overlapping semantic changes in the exception branch,
chain adapter, and subscription/publication machinery. The dependent helper
closure above must be ported in those exact units; replacing the managed files
with the old draft is not a valid rebase. This probe is evidence of required
work, not an integrated candidate.
