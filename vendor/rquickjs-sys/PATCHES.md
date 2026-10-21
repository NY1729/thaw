# Local changes to the vendored rquickjs-sys

Upstream: `rquickjs-sys` 0.12.2 (crates.io). `thaw-local-changes.patch` is the exact diff
against that release for the three files below (`patch -p1 -d vendor/rquickjs-sys < thaw-local-changes.patch`
re-applies it after a vendor refresh).

## build.rs
- Emit `cargo:rerun-if-changed` for every QuickJS source/header and `quickjs.bind.h`, so editing
  `quickjs.c` rebuilds the sys crate.

## quickjs/quickjs.h and quickjs/quickjs.c
1. **Async context propagation** (`async_context` on `JSContext`, `JSJobEntry`, `JSPromiseReactionData`;
   new `JS_GetAsyncContext` / `JS_SwapAsyncContext`; `js_enqueue_job_with_async_context`).
   A job runs with the async frame captured when it was enqueued, or the frame retained when a
   Promise reaction was registered. Used by `async_hooks`/`AsyncLocalStorage`.
2. **Proxy property descriptor invariants** (`js_proxy_getOwnPropertyDescriptor`,
   `js_proxy_defineProperty`): accessor descriptors from the trap get `JS_PROP_GETSET`, and the
   writable invariant follows ES2024 10.5.6 step 17.c (only `writable: false` conflicts with a
   non-configurable writable target property).
3. **Iterator helpers** (`js_iterator_helper_next`): `return()` closes the source (and a flatMap inner
   iterator) without advancing or calling mapper callbacks; the helper is marked done when it fails;
   `JS_FreeValue(item)` added on the leak paths of `filter`/early exits.

When refreshing the vendored copy: re-apply the patch, rebuild, and run the `thaw-quickjs` lib tests
and the `node-compat` iterator-helper, proxy and async-hooks cases.
