# Native Worker ref/unref plan (Sol source-only)

Finding01a0f1d4-376c-7593-a648-9744d92e2c92:0, HEAD74bac1b4ff89e7f5c29777249c4bbbbdd5085a04.

Root: registry Worker.prototype.ref/unref return this without host change for _nativeHandle. Rust HostWorker in thaw-quickjs/src/lib.rs carries command sender/event receiver/thread only. processes.rs host_workers_active returns workers table nonempty. context.rs publishes __thaw_worker_active and api.rs platform_activity_pending uses it as parent eventloop activity.

Luna next implementation: add one refed boolean to existing HostWorker, initialized true at native spawn. Publish minimal existing-style host function to change refed by handle, return false for absent handle. Host active probe should count only refed live workers, not delete or terminate unrefed workers. Native JS ref/unref forward to that bridge and return same Worker; fallback behavior remains port ref/unref. Ended handles should be no-op. Trace all lifecycle/map cleanup and context registration consumers before editing. Keep unrefed event polling intact whenever parent runs for another reason; unref is liveness policy, not worker termination. Check native thread ownership teardown and any join that might still block process exit after liveness false.

Unrun controls: default refed, unref toggles active false with only that worker; ref restores true; one refed sibling keeps true; missing/ended handle no panic; native JS returns same worker and invokes correct host handle; fallback port paths unchanged. No mocks that accidentally spawn background threads if host structure can be constructed with channels. No product execution authorized.

Scratch only product edits by Luna, Sol independent source review and immutable paired replay before parent commit. Initial41 are hash-tracked preservation, not edit bans.

Teardown source follow-up: processes.rs poll_host_workers joins only workers collected after Exit or channel Disconnected, removes finished handles after deduplication. No unconditional join over active workers appears in processes.rs. A refed-only activity probe must retain normal polling for unrefed workers whenever another parent activity keeps the loop running, then keep this existing finished-only join behavior. Whole module/TLS teardown still requires implementer verification.
