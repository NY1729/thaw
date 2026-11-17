//! Shadow call stack for `Error.prepareStackTrace` / `Error.captureStackTrace`: the closures being run,
//! innermost last, pushed by the call paths (`lib.rs` `call_as`, `compile/call.rs`, `classes.rs`). The
//! Error prelude reads it through `__thaw_call_frames`.

use crate::*;

/// The function of a closure value, for matching against `Interp::call_frames`.
fn closure_def(v: &Value) -> Option<Rc<FuncDef>> {
    let Value::Object(o) = v else { return None };
    let kind = o.kind.borrow();
    match &*kind {
        Kind::Closure { def, .. } => Some(def.clone()),
        _ => None,
    }
}

impl Interp {
    /// A mark for `frame_truncate`: the depth of the running coroutine's segment. Relative to
    /// `frame_base`, so it stays valid when the coroutine is resumed at another depth of the shared stack.
    pub(crate) fn frame_mark(&self) -> usize {
        self.call_frames.len().saturating_sub(self.frame_base)
    }

    /// Pops the frames pushed since `mark` (a `frame_mark`); never below the running segment's base.
    pub(crate) fn frame_truncate(&mut self, mark: usize) {
        self.call_frames.truncate(self.frame_base + mark);
    }

    pub(crate) fn install_call_frames(&mut self) {
        let g = self.global.clone();
        let f = self.native(|it, _, a| {
            // `skip` (the Error prelude's own function) is dropped from the top; `cut` (a `captureStackTrace`
            // `ctorOpt`) removes itself and everything above its topmost call, or every frame when it is absent.
            let limit = match arg(a, 1) {
                Value::Number(n) => n.max(0.0) as usize,
                _ => 0,
            };
            let skip = closure_def(&arg(a, 0));
            let cut = closure_def(&arg(a, 2));
            let frames = &it.call_frames;
            let mut end = frames.len();
            if let Some(s) = &skip {
                while end > 0 && Rc::ptr_eq(&frames[end - 1].0, s) {
                    end -= 1;
                }
            }
            if let Some(c) = &cut {
                end = frames[..end]
                    .iter()
                    .rposition(|(d, _)| Rc::ptr_eq(d, c))
                    .unwrap_or(0);
            }
            let picked: Vec<(Option<Rc<str>>, bool)> = frames[..end]
                .iter()
                .rev()
                .take(limit)
                .map(|(d, ctor)| (d.name.clone(), *ctor))
                .collect();
            let items = picked
                .into_iter()
                .map(|(name, ctor)| {
                    let name = name.map_or(Value::Null, |n| Value::str(&n));
                    it.array(vec![name, Value::Bool(ctor)])
                })
                .collect();
            Ok(it.array(items))
        });
        g.declare("__thaw_call_frames", f, true);
    }
}
