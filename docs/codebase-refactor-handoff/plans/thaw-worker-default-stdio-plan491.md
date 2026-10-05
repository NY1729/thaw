# Worker default stdio plan — Sol source-only

Finding01a0f1cc-7674-76e1-8caa-168b1e8442d1:0, HEADbd4565fa.

Rootcaller trace: native host_worker_bootstrap processes.rs80-83 sends process.stdout/stderr.write as string events regardless options; only console overrides are option-dependent. Registry startNativeWorker creates public stdout/stderr only when options enabled, and platform poll drops event when corresponding stream null. Fallback points workerProcess streams to parent when option false, so direct output is preserved there.

Luna next change: use existing WorkerReadable/parent stream piping rather than new abstractions. Trace parent stream API/pipe/reentry/backpressure and Worker public stdout property contract, then create native readables and connect default output to parent stdout/stderr while explicit capture options keep separate readable output. Preserve messages/exits order and close readables on _finish. Console default output must not become duplicate host/native event emission. Keep binary-byte fidelity finding01a0f1cb separate until producer/event/writable encoding contract is actually fixed, and do not claim it from default forwarding.

Controls unrun: fake/native event stdout stderr with unspecified options arrives exactly once at parent stream; explicit capture doesn't pipe; normal exit closes output; fallback unchanged; console messages not duplicated. Productexecution forbidden, scratchLuna edits and Sol independent paired review required.
