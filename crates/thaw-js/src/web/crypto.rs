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

/// `__thaw_hash(alg, data, out)`: digest of `data` written into `out` (JS sizes `out`); false = unknown algorithm.
fn digest(alg: &str, data: &[u8]) -> Option<Vec<u8>> {
    use sha2::Digest;
    Some(match alg {
        "md5" => md5::Md5::digest(data).to_vec(),
        "sha1" => sha1::Sha1::digest(data).to_vec(),
        "sha256" => sha2::Sha256::digest(data).to_vec(),
        "sha384" => sha2::Sha384::digest(data).to_vec(),
        "sha512" => sha2::Sha512::digest(data).to_vec(),
        _ => return None,
    })
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
    let hash = interp.native(|it, _, a| {
        let alg = it.to_str(&arg(a, 0));
        let (Some(d), Some(o)) = (Interp::typed_of(&arg(a, 1)), Interp::typed_of(&arg(a, 2)))
        else {
            return it.throw_error("TypeError", "hash data must be a Uint8Array");
        };
        let Some(sum) = digest(&alg, &d.data.borrow()[d.off..d.off + d.len]) else {
            return Ok(Value::Bool(false));
        };
        o.data.borrow_mut()[o.off..o.off + sum.len()].copy_from_slice(&sum);
        Ok(Value::Bool(true))
    });
    interp.global.declare("__thaw_hash", hash, true);
    interp.global.declare("__thaw_random_fill", fill, true);
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("crypto prelude must evaluate"));
}
