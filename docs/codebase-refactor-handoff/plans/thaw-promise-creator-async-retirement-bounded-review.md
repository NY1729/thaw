# Bounded source review: async catch retirement queue

Source checkpoint: `/tmp/thaw-promise-creator-origin-draft466/candidate`; `async_frames/codegen.rs` SHA-256 `dfebb7a6bab5528c0449406b34c42114cb9264c5d64505382e59f1050bd09409`; `entrypoints.rs` `5b65961e3d808fde3aeb6e9951b936827b8cd291849dfb989fffffe22703217c`; `std/json.rs` `22f1af276038089ecbe03034e213ec86d7cb118c82a8330814b1ee15ca293972`. Mutable source only, no product execution.

Decision: **HOLD** for a general async-frame retirement/reporting claim. `retire_async_catch_owner` can enqueue an owned Json cleanup node from any async frame, but the only generated deferred cleanup driver is `compile_http_deferred_cleanup_driver`. `entrypoints.rs` registers that driver only when `createServer` is present. Without a registered driver, `thaw_json_run_deferred_cleanup_turn` returns false while queued nodes remain. A non-HTTP long-lived async loop therefore has no source-proven exact drain/report boundary for these nodes, despite arena rooting preserving their memory.

This finding does not assess the whole creator-ticket or HTTP adapter. A generic event-loop drain with exact pending-tuple handling, or another explicit non-HTTP terminal disposition, is needed before merging this owner-retirement path as complete.
