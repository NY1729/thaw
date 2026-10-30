//! `crypto.getRandomValues` / `crypto.randomUUID` and the random half of `node:crypto`
//! (`randomBytes`, `randomFillSync`, `randomInt`), backed by the OS random source.

use crate::*;

const SOURCE: &str = include_str!("crypto.js");

fn fill_os_random(buf: &mut [u8]) {
    use std::io::Read;
    // ponytail: /dev/urandom only (Unix targets); getrandom(2) crate if other targets matter
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .unwrap_or_else(|_| panic!("OS random source unavailable"));
}

pub(crate) fn install(interp: &mut Interp) {
    let fill = interp.native(|it, _, a| {
        let Some(t) = Interp::typed_of(&arg(a, 0)) else {
            return it.throw_error("TypeError", "argument must be an integer typed array");
        };
        let size = t.ty.size();
        fill_os_random(&mut t.data.borrow_mut()[t.off..t.off + t.len * size]);
        Ok(arg(a, 0))
    });
    interp.global.declare("__thaw_random_fill", fill, true);
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("crypto prelude must evaluate"));
}
