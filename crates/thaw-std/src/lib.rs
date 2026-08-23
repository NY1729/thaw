//! Native runtime support for a handful of builtins the HIR lowering/
//! codegen special-case by name (`fetch`, `JSON.parse`, `JSON.stringify`,
//! and the JSON accessors behind `.field`/`[i]`/`Number`/`String`/
//! `Boolean` on a `HirType::Json` value). Linked into compiled Thaw
//! programs as a static archive by thaw-cli, the same way thaw-arena and
//! thaw-runtime are.

mod fetch;
mod fs;
mod json;
