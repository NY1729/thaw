# Independent source review: test plan assertion count

Decision: bounded **ACCEPT** for context assertion and child-test plan counting. No product build or test execution.

- Frozen `/tmp/thaw-test-plan-count-frozen459` on HEAD458 `c0c609da34553ddd81cc7a515f358bdd668612d6`: manifest SHA-256 `61931928d9b7c6f1ad519659a22e0c404d88f995f572638fe87574413d76c554`; report SHA-256 `ed092e3874334a183fe4fc98d5945c451d90a4bedcb3dfd7941a9eeb86ddbbd2`. Both patches replayed byte-exact; source diff whitespace checks were clean.
- The facade is a distinct function with a per-context WeakMap cache. `Reflect.apply` calls the original target once; its wrapper counts before invocation, including a caught failing assertion and `.call`. `assertion.strict`, `.default`, and `.ok` alias the same underlying root function in the built-in assert source; cache prevents duplicate wrappers. Own assertion method exports are copied and wrapped; `AssertionError` stays the raw constructor; inherited `call/apply/bind` retain Function semantics.
- A fresh facade is created for each TestContext. `t.test(...)` increments its parent once after normalization and before the child runs; a child gets its own counter. Direct external `require('node:assert')` calls do not affect the context. The frozen-assertion fixture confirms no Proxy own-property invariant dependency.
- The unrun bundled regression imports `node:test` and `node:assert`, covers success, missing/excess assertions, child count, external isolation, caught failure, and frozen shared assert. The root module plus these two built-ins justify `file_count >= 3`.

This review does not claim a broader Node test-runner compatibility audit.
