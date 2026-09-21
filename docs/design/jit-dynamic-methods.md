# JIT dynamic host calls

The JIT must not know which JavaScript engine, native addon host, or future
runtime owns an opaque value. Dynamic member access therefore crosses one
host-supplied ABI instead of calling QuickJS directly.

```text
host_call(operation, receiver, member, arguments) -> value | error
```

Values use the JIT's existing tagged-value representation. The host owns
opaque handles; primitive, array, and dictionary results use the existing JIT
tags. `operation` initially supports:

- `get`: read a property.
- `call`: invoke a method with `receiver` as `this`.
- `iterator_next`: advance an opaque iterator and return `{ done, value }`.

Receivers and arguments are borrowed for the call. Returned opaque handles
remain live until the enclosing JIT invocation ends. Errors use the existing
JIT pending-error channel.

Set composition uses only this ABI: read `size`, call `has`, call `keys`, and
advance the returned iterator. Native Set and Map values keep their direct
dictionary fast path; only genuinely opaque Set-like operands cross the host
boundary.

The first implementation exposes that operation as one narrow
`dynamic_object_query(SET_LIKE_KEYS, handle)` snapshot. It validates the
Set-like protocol and materializes the keys once, then all composition runs in
the existing native dictionary path. The generic ABI above replaces this
specialized query when another JIT feature needs arbitrary member calls; until
then, implementing the larger dispatcher would add unused machinery.

This keeps the machine-code compiler and Set implementation independent of
QuickJS. A QuickJS adapter may supply the ABI when a value originates there;
other hosts can provide the same operations without embedding QuickJS.
