//! `THAW_JS_COMPILE=check` oracle (plan 4.1). C0 only fixes the error text; the comparison itself
//! arrives with the first pure kernels (C2).

/// The thrown message for a compiled-versus-AST mismatch: `function` is the name, `src` the position.
#[allow(dead_code)]
pub(crate) fn mismatch_message(name: &str, src: &str, detail: &str) -> String {
    format!("thaw-js compile check mismatch in {name} ({src}): {detail}")
}
