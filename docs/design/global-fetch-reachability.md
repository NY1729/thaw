# Making `globalThis.fetch` actually reachable

**Status: done.** No new `fetch` implementation was needed -- thaw
already ships a full one (`crates/thaw-registry/src/registry/bundle/
render.rs`, built on `node:http`/`node:https`'s real client). This
document records three general bugs that kept it from ever firing for
a real package (`ky`), found and fixed while chasing what first looked
like a missing feature.

## Starting premise (wrong)

`ky`'s own bundle referenced `globalThis.fetch`, which was `undefined`
at runtime. The natural read was "thaw has no browser-standard
`fetch()` reachable from Fallback/QuickJS code -- build one." A first
implementation was written (a JS polyfill directly on thaw-quickjs's
raw TCP/TLS bridges, `__thaw_net_*`/`__thaw_tls_*`) and worked
end-to-end for `ky` over both HTTP and HTTPS.

Running the full regression suite immediately after surfaced two
already-passing thaw-registry tests failing:
`global_fetch_sends_requests_follows_redirects_and_returns_responses`
and `global_fetch_resolves_headers_before_delayed_body_chunks`. Thaw
already had a `globalThis.fetch` -- a considerably more complete one,
with real streaming response bodies, `gzip`/`deflate`/`br`
auto-decompression, and full WHATWG redirect semantics, built on
`node:http`'s existing real client (`http1.rs`) -- injected into a
package's bundle by `render_bundle` whenever the bundle already needed
`node:http`. The new from-scratch implementation was shadowing it and
was strictly worse. It was reverted in full (the new `.js` file, its
`platform_globals.rs` registration, its `thaw-quickjs` unit tests, and
a `build.rs` change that had forced TLS support into every QuickJS
build to route around the symptom rather than its cause).

**Lesson repeated from this session's npm bug hunts:** a feature that
looks entirely missing may already exist; find out why it isn't
reachable before writing a second one.

## Real root causes (three, each general)

### 1. `uses_global_fetch` only matched a bound identifier

`render_bundle` only injects the fetch shim (and bundles `node:https`
to back it) when `ModuleAnalysis::uses_global_fetch` is set --
computed by an SWC AST visitor (`module_transform.rs`) that matched a
bare `fetch` identifier or a direct `fetch(...)` call. `ky`'s own code
is `options.fetch ?? globalThis.fetch.bind(globalThis)`: both
occurrences of `fetch` there are a *member expression's property*, a
distinct AST node from a bound identifier reference, so the existing
`visit_ident` never saw them. `ky` never itself requires `node:http`,
so nothing else would have pulled the shim in either.

Fixed by adding `visit_member_expr` to the same visitor, matching
`<globalThis|self|window|global>.fetch` specifically (not fetch as a
property of an arbitrary object, which would be a real false-positive
source).

### 2. Feature detection is blind to a compressed bundle

thaw-cli's `source_uses_tls`/`source_uses_wasm`/`source_uses_brotli`
(`build.rs`) decide which optional `thaw-quickjs` Cargo features
(`tls`/`wasm`/`brotli`) to compile in with a blind substring scan over
the generated shim text. `generate_module_init` (thaw-bridge) gzip+
base64-encodes any bundle at or above 1KB (`encode_embedded_script`)
before splicing it into that shim text -- opaque to a literal
substring scan. Any real package's bundle is usually well over 1KB, so
none of these three markers could ever be found once `node:https`
(which literally contains `__thaw_tls_`) got bundled in -- TLS
silently never linked, and every `https://` request failed at runtime
with an opaque "TLS support is not linked", not a build error.

Fixed generally in `generate_module_init` itself: before compressing a
bundle, re-emit whichever of the known markers its *raw* source
contains as a plain, unexecuted, uncompressed comment line. This fixes
detection for `tls`/`wasm`/`brotli` alike, for every package, not just
the fetch shim's own use of `node:https`.

### 3. Nothing ever spells out the word "brotli"

Even with (2) fixed, a real HTTPS request to a Cloudflare-fronted site
(e.g. `https://example.com/`, which replies `Content-Encoding: br`)
still failed. The fetch shim always sends `Accept-Encoding: gzip,
deflate, br` and decompresses via `DecompressionStream(contentEncoding)`
-- but its own source (and any typical package's) only ever writes the
short code `'br'`, never the literal word "brotli"/"Brotli" that
`source_uses_brotli` scans for. Detection (2) had nothing to
rediscover. Fixed by having the fetch shim's own source note in plain
text that it uses Brotli decompression, so both the direct scan (for a
small, uncompressed bundle) and fix (2)'s marker re-emission (for a
large one) catch it.

### Aside: a real, unrelated TLS bug

Chasing (1) surfaced a fourth, genuinely separate bug on the way:
`tls_finish` (`thaw-quickjs/src/quickjs/networking.rs`) sent its own
`close_notify` and then called `read_to_end`, but a peer that closes
its TCP connection without sending *its* `close_notify` back (common
in the wild -- rustls treats it as `io::ErrorKind::UnexpectedEof`
rather than a clean EOF, a deliberate anti-truncation-attack default)
made every such response fail even though the bytes already read were
a complete, correctly framed response. Fixed by treating
`UnexpectedEof` there as a clean close, for both the client and
server-accept code paths.

## Verification

- `crates/thaw-registry/src/tests/network.rs`'s existing
  `global_fetch_*` tests (unaffected by any of the above, confirming
  no regression from investigating this).
- New pinned test:
  `registry_add_fetches_over_http_and_https_with_real_ky_when_enabled`
  (`thaw-cli`) -- a local mock server for plain HTTP, and a real
  request to `https://example.com/` for HTTPS + Brotli, both through
  `ky`, both through the full `thaw build` pipeline (not a lower-level
  harness -- earlier debugging showed the same shim behaves correctly
  when driven directly by thaw-quickjs but not when produced by the
  full compile pipeline, which is exactly what exposed bug (2)).
- Full `thaw-registry`, `thaw-quickjs`, `thaw-bridge`, `thaw-hir`,
  `thaw-llvm`, `thaw-cli`, `thaw-std` suites green.
