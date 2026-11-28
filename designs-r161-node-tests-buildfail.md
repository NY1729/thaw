# r161: Node-tests build-fail families F2-F6 (re-measured 2026-11-28)

Baseline: 184 paths from r136-nt-all.jsonl re-built with r132-target/release/thaw (r132 pre-dates part of the 40+ merges; PR-side numbers below use r161-target debug).

Result of the 184 re-builds: pass 14, runtime-fail 18, build-fail 129, timeout 23 (build timeouts under load, not counted).

The old message "unsupported member call target at source bytes" (44) no longer reproduces; only "unsupported member call property" (12) remains.


## F2 (12): computed string key call `buf[`read${x}`](..)` on Bytes receiver (calls.rs:3380)

- test-buffer-bigint64.js: unsupported member call property
- test-buffer-read.js: unsupported member call property
- test-buffer-readdouble.js: unsupported member call property
- test-buffer-readfloat.js: unsupported member call property
- test-buffer-readint.js: unsupported member call property
- test-buffer-readuint.js: unsupported member call property
- test-buffer-writeuint.js: unsupported member call property
- test-console-async-write-error.js: unsupported member call property
- test-console-sync-write-error.js: unsupported member call property
- test-stream-readable-add-chunk-during-data.js: unsupported member call property
- test-zlib-convenience-methods.js: unsupported member call property
- test-zlib-truncated.js: unsupported member call property

## F3a (17): closure references a binding declared later / being declared (TDZ-style forward reference)

- test-assert-calltracker-calls.js: unknown variable `func`
- test-http-abort-stream-end.js: unknown variable `req`
- test-http-agent-remove.js: unknown variable `req`
- test-http-client-timeout-agent.js: unknown variable `maybeDone`
- test-http-client-timeout-with-data.js: unknown variable `onresponse`
- test-http-outgoing-proto.js: unknown variable `__thaw_type_node_http_ServerResponse`
- test-net-bytes-read.js: unknown variable `__thawmod0_server`
- test-process-getactiveresources-track-timer-lifetime.js: unknown variable `timeout`
- test-stream-objectmode-undefined.js: unknown variable `stream`
- test-stream-pipe-await-drain-push-while-write.js: unknown variable `__thawmod0_readable`
- test-stream-pipe-manual-resume.js: unknown variable `rs`
- test-stream-readable-to-web-termination.js: unknown variable `reader`
- test-stream2-base64-single-char-read-end.js: unknown variable `__thawmod0_timeout`
- test-timers-api-refs.js: unknown variable `__thawmod0_timeout`
- test-timers-args.js: unknown variable `callback`
- test-timers-clearImmediate.js: unknown variable `immediate`
- test-timers-immediate.js: unknown variable `__thawmod0_immediateB`

## F3b (9): module/namespace binding used as a first-class value

- test-buffer-inspect.js: unknown variable `buffer`
- test-buffer-set-inspect-max-bytes.js: unknown variable `buffer`
- test-fs-promises-exists.js: unknown variable `fsPromises`
- test-fs-read-stream-err.js: unknown variable `fs`
- test-fs-read-stream-fd-leak.js: unknown variable `fs`
- test-global-webstreams.js: unknown variable `webstreams`
- test-os-eol.js: unknown variable `os`
- test-process-default.js: unknown variable `__thaw_namespace_1000003`
- test-zlib-const.js: unknown variable `zlib`

## F3c (12): class called without `new` (legacy-callable constructor)

- test-buffer-no-negative-allocation.js: call to unknown function `Buffer`
- test-buffer-over-max-length.js: call to unknown function `Buffer`
- test-buffer-zero-fill.js: call to unknown function `Buffer`
- test-http-buffer-sanity.js: call to unknown function `__thaw_type_node_http_Server`
- test-http-client-override-global-agent.js: call to unknown function `__thaw_type_node_http_Server`
- test-http-default-encoding.js: call to unknown function `__thaw_type_node_http_Server`
- test-http-request-end-twice.js: call to unknown function `__thaw_type_node_http_Server`
- test-http-request-end.js: call to unknown function `__thaw_type_node_http_Server`
- test-http-res-write-after-end.js: call to unknown function `__thaw_type_node_http_Server`
- test-http.js: call to unknown function `__thaw_type_node_http_Server`
- test-stream-ispaused.js: call to unknown function `Function`
- test-stream-readable-from-web-termination.js: cannot infer the type of `wrapper`: call to unknown function `__thaw_class_ctor_246e6577246e6f64653a73747265616d3a3a5265

## F3d (13): method call on a receiver whose type could not be inferred (self/forward reference or untyped callback param)

- test-buffer-compare-offset.js: call to unknown function `__thawmod0_a.compare`
- test-buffer-compare.js: call to unknown function `__thawmod0_b.compare`
- test-http-outgoing-properties.js: call to unknown function `server.close`
- test-http-request-arguments.js: call to unknown function `server.close`
- test-http-socket-encoding-error.js: call to unknown function `__thaw_nullable_receiver_0.address`
- test-net-connect-keepalive.js: call to unknown function `client.end`
- test-net-connect-nodelay.js: call to unknown function `client.end`
- test-net-socket-write-after-close.js: call to unknown function `client.on`
- test-net-socket-write-error.js: call to unknown function `__thaw_nullable_receiver_0.address`
- test-stream-base-typechecking.js: call to unknown function `client.write`
- test-stream-pipeline-listeners.js: call to unknown function `__thawmod0_d.listenerCount`
- test-stream-writable-finish-destroyed.js: call to unknown function `w.on`
- test-timers-unrefed-in-callback.js: call to unknown function `timer1.unref`

## F3e (15): missing API / undeclared global (runtime ReferenceError expected)

- test-buffer-of-no-deprecation.js: call to unknown function `Buffer.of`
- test-console-with-frozen-intrinsics.js: call to unknown function `console.clear`
- test-exception-handler2.js: call to unknown function `nonexistentFunc`
- test-fs-realpath-native.js: call to unknown function `__thaw_typed_tjs_6e6f64653a66733a3a7265616c7061746853796e63.native`
- test-http-client-abort2.js: call to unknown function `req.abort`
- test-http-client-response-timeout.js: call to unknown function `req.destroy`
- test-http-pause.js: call to unknown function `req.pause`
- test-http-server-consumed-timeout.js: call to unknown function `req.setTimeout`
- test-http-timeout-overflow.js: call to unknown function `req.clearTimeout`
- test-http-zero-length-write.js: call to unknown function `req.setEncoding`
- test-next-tick-errors.js: call to unknown function `what`
- test-performance-many-marks.js: call to unknown function `performance.mark`
- test-timers-max-duration-warning.js: call to unknown function `timers.enroll`
- test-url-revokeobjecturl.js: call to unknown function `URL.revokeObjectURL`
- test-webstream-readable-from.js: call to unknown function `ReadableStream.from`

## F4 (18): declaration arity (d.ts in resolution.rs) narrower than Node / ctor called without new

- test-buffer-includes.js: native `.includes` expects one or two arguments
- test-buffer-inheritance.js: function `__thaw_class___thawmod0_T_constructor` expects N argument(s), got N
- test-console-table.js: function `__thawmod0_test` expects N argument(s), got N
- test-file-read-noexist.js: function `__thaw_typed_tjs_6e6f64653a66733a3a7265616446696c65` expects N argument(s), got N
- test-fs-exists.js: function `__thaw_typed_tjs_6e6f64653a66733a3a657869737473` expects N argument(s), got N
- test-fs-read-file-assert-encoding.js: function `__thaw_typed_tjs_6e6f64653a66733a3a7265616446696c65` expects N argument(s), got N
- test-fs-read.js: function `__thaw_typed_tjs_6e6f64653a66733a3a7265616453796e63` expects N argument(s), got N
- test-fs-readfile-empty.js: function `__thaw_typed_tjs_6e6f64653a66733a3a7265616446696c65` expects N argument(s), got N
- test-fs-readfile-zero-byte-liar.js: function `__thaw_typed_tjs_6e6f64653a66733a3a7265616446696c6553796e63` expects N argument(s), got N
- test-http-agent-scheduling.js: function `createServer` expects N argument(s), got N
- test-http-client-keep-alive-hint.js: function `createServer` expects N argument(s), got N
- test-http-keepalive-override.js: function `__thaw_typed_tjs_246e6577246e6f64653a687474703a3a4167656e7424617269747930` expects N argument(s), got N
- test-http-multiple-headers.js: function `createServer` expects N argument(s), got N
- test-http-request-join-authorization-headers.js: function `createServer` expects N argument(s), got N
- test-http-response-setheaders.js: function `createServer` expects N argument(s), got N
- test-net-isip.js: function `__thaw_typed_tjs_6e6f64653a6e65743a3a69734950` expects N argument(s), got N
- test-net-write-arguments.js: function `__thaw_typed_tjs_246e6577246e6f64653a6e65743a3a53747265616d24617269747930` expects N argument(s), got N
- test-stream-pipeline-duplex.js: function `__thaw_typed_tjs_6e6f64653a73747265616d3a3a706970656c696e65` expects N argument(s), got N

## F6 (24): bare `events` import resolved as npm package (resolution.rs bare_node_builtin_specifier exemption)

- test-console-log-stdio-broken-dest.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-capture-rejections.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-add-listeners.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-check-listener-leaks.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-emit-context.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-errors.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-invalid-listener.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-listener-count.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-listeners.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-method-names.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-modify-in-emit.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-num-args.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-prepend.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-remove-all-listeners.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-set-max-listeners-side-effects.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-event-emitter-special-event-names.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-events-list.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-events-uncaught-exception-stack.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-http-agent-abort-controller.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-http-client-abort-destroy.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-http-client-default-headers-exist.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-http-invalidheaderfield.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-http-server-capture-rejections.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)
- test-process-prototype.js: <file>: registry package `events`: failed to read `thaw_modules<file>`: No such file or directory (os error N)

## other (1): JSON assignment key must be string or number, got JsValue

- test-url-parse-query.js: JSON assignment key must be string or number, got JsValue

## other (2): dynamic value index expression must be string, number, or sy

- test-http-agent-free-socket-data-guard.js: dynamic value index expression must be string, number, or symbol, got Json
- test-http-keep-alive-close-on-header.js: dynamic value index expression must be string, number, or symbol, got Json

## other (1): native `instanceof` right operand `__thaw_typed_tjs_246e6577

- test-http-client-timeout.js: native `instanceof` right operand `__thaw_typed_tjs_246e6577246e6f64653a687474703a3a436c69656e74526571756573742461726974

## other (1): object has no field `connection`

- test-http-no-read-no-dump.js: object has no field `connection`

## other (1): object has no field `rawTrailers`

- test-http-raw-headers.js: object has no field `rawTrailers`

## other (1): unsupported dynamic object field Void

- test-net-dns-lookup-skip.js: unsupported dynamic object field Void

## other (1): unsupported property access `.code` on a value of type Bool

- test-socket-write-after-fin-error.js: unsupported property access `.code` on a value of type Bool

## other (1): unsupported statement Decl(Class(ClassDecl { ident: Ident { 

- test-http-agent-timeout.js: unsupported statement Decl(Class(ClassDecl { ident: Ident { span: N..N, ctxt: #N, sym: "CustomAgent", optional: false },

# Root causes and plan

| family | root cause | plan | status |
|---|---|---|---|
| F6 (24) | `bare_node_builtin_specifier` exempted bare `events`, so `import ... from 'events'` went to the npm registry (`thaw_modules/events/package.d.ts` missing with --no-install) | bare `events` is `node:events` (builtin wins over the npm package, as in Node) | PR-1 |
| F3a (17 unknown variable) | a closure (or earlier statement) mentions a `const`/`let` declared later in the block and the two cannot be reordered (`const fn = () => clear(h); const h = setImmediate(fn)`) | `predeclare_captured_bindings` (lowering pass next to `forward_declare_arrows`): `let h;` before the first mention, declaration becomes `h = init` | PR-2 |
| F4 (18) | (a) JS function called with fewer args than params; (b) hand-written d.ts arity narrower than Node (`readSync(fd, options)`, `pipeline` 4-stream overload mismatch); (c) calls that are deliberate ERR_INVALID_ARG_TYPE probes (`fs.exists()`), not fixable | (a) unannotated params of declared functions are omittable (undefined) -- PR-2; (b) per-declaration + arity_sync.py inventory, follow-up | PR-2 / follow-up |
| F2 (12) | `buf[\`readBigInt64${e}\`](..)` on a Bytes receiver: no static method to pick and Bytes is not a thaw-js handle, so the dynamic named-call path produces invalid IR (tried; reverted) | needs a name-dispatch over the Bytes method table (if-chain with a JsValue result) or zero-copy Buffer handles (Buffer redesign S-stages) | design only |
| F3b (9) | builtin module default import / `process` used as a first-class value (`Object.keys(os)`, `assert.strictEqual(fsPromises, fs.promises)`) has no value form | materialise the builtin module object from its thaw-js bundle on value use | design only |
| F3c (12) | classes callable without `new` in Node (`http.Server(fn)`, `Buffer(n)`, `net.Stream(o)`) are type-only / unknown functions in the d.ts | declare them callable from the Node inventory (kind=function) | follow-up |
| F3d (13) | receiver type unknown (self-referencing object literal, nullable receiver, untyped callback param) | per-case after F3a; self-reference inside the own initializer (`const rs = Readable({read(){rs.push()}})`) still needs a RecursiveClosure-style cell for non-timer values | follow-up |
| F3e (15) | missing APIs (`performance.mark`, `console.clear`, `Buffer.of`, `URL.revokeObjectURL`, `ReadableStream.from`, `timers.enroll`) and undeclared global calls that must throw ReferenceError at run time | implement APIs; unknown global call -> runtime ReferenceError | follow-up |

The old `unsupported member call target at source bytes` (44) no longer reproduces (fixed by earlier merges).
After PR-1 the events tests move on to a new, separate family: EventEmitter `Symbol` event names and `Symbol` arguments ("value has type Symbol, expected Json") and `new Writable({writev})` option shapes.

# F3c: classes callable without `new` (measured on Node 22 by calling every capitalised builtin export with no arguments)

Callable without `new` (no "cannot be invoked without 'new'" TypeError): buffer.Buffer, buffer.SlowBuffer, child_process.ChildProcess, cluster.Worker, console.Console, crypto.{Certificate,Cipheriv,Decipheriv,DiffieHellman,DiffieHellmanGroup,ECDH,Hash,Hmac,Sign,Verify}, dgram.Socket, events.EventEmitter, fs.{Stats,ReadStream,WriteStream,FileReadStream,FileWriteStream}, http.{Agent,ClientRequest,IncomingMessage,OutgoingMessage,Server,ServerResponse}, https.{Agent,Server}, module.Module, net.{Server,Socket,Stream}, readline.Interface, stream.{Readable,Writable,Duplex,Transform,PassThrough,Stream}, string_decoder.StringDecoder, tls.{SecureContext,TLSSocket,Server}, tty.{ReadStream,WriteStream}, url.Url, zlib.{Deflate,Inflate,Gzip,Gunzip,DeflateRaw,InflateRaw,Unzip,BrotliCompress,BrotliDecompress}.
Everything else (URL, Blob, Map-like classes, worker_threads.MessagePort/MessageChannel, ...) is new-only. The inventory's `kind: class` does not distinguish the two, so this list (script: thaw-tmp/r161-callable.js) is the source for the d.ts change: declare these as callable (call signature returning the instance) next to their class declarations in resolution.rs, and lower `Ident(args)` / `ns.Ident(args)` on such a class as `new`. Tests blocked on it: test-http-buffer-sanity, test-buffer-no-negative-allocation, test-net-write-arguments, test-http-keepalive-override and the other 8 F3c paths.
