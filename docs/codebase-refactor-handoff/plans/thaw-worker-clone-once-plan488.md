# Worker clone single-read plan (Sol source-only)

Finding 01a0f47b-1289-7392-86ae-d7eea2455163:0. Managed baseline 22c425d9a172e05e3eb2d54d5ffc622890b2b8d1.

Root: registry system/workers.rs assertCloneable traverses Object.keys and reads every value before delegating structuredClone or MessagePort.postMessage. Existing clone traversal in QuickJS platform_globals/performance.js reads input[key] once; native Worker wire traversal in workers/structured_clone.js encode does the same. Removing prewalk alone would lose markAsUncloneable nested checks. Copying into an intermediate graph would change transfer/built-in/prototype semantics and duplicate allocation.

Luna next plan: trace every clone/port/native encode caller and hook/load order, then enforce existing global WeakSet __thaw_uncloneable_objects at each actual traversal node before source property reads. Reuse direct WeakSet checks, no new recursive validator. Preserve transfer-list mark checks without graph reads. Remove assertCloneable prewalk wrappers only when all actual clone and encode paths enforce the same mark contract. Confirm whether an externally preexisting structuredClone bypasses the local implementation; do not silently remove protection there without a supported single-read design.

Controls (unrun only): enumerable getter increments once and returns different values on later calls; cloned result is first value; getter throws original value; nested marked object returned by getter rejects after one read; Map/Set/cycles marked node rejection; MessagePort and native codec paths; transfer rejected without detachment. Native encode must reject marked node before transfer detach. Do not claim full structured clone compatibility.

Only scratch source edits by Luna, independent Sol review before paired freeze/commit. No product tools or tests executed.

Additional source trace at HEAD488: platform_globals.rs concatenates performance before structured_clone before messaging. MessagePort.postMessage catches structuredClone failures and asynchronously dispatches peer messageerror, whereas registry assertCloneable prewalk currently throws marked-node failures synchronously. A single-read integration must explicitly preserve the expected marked-node failure disposition or prove a corrected common API contract; simply deleting the wrapper changes behavior. The WeakSet can be initialized after platform globals, so actual traversal should consult current global mark registry at invocation time. No product execution.
