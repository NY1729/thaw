//! CABI v1 for `libthaw_rt.so`: four symbols, scalars only.
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU32, Ordering};

include!(concat!(env!("OUT_DIR"), "/abi_hash.rs"));

pub const THAW_ABI_VERSION: u32 = 1;

/// Init refcount. `thaw_rt_init` / `thaw_rt_shutdown` only count; they never
/// start or stop the runtime (the generated `main` does that).
static INIT_COUNT: AtomicU32 = AtomicU32::new(0);

#[repr(C)]
pub struct ThawConfig {
    pub abi_version: u32,
    /// v1: must be zero.
    pub reserved: [u32; 7],
}

#[no_mangle]
pub extern "C" fn thaw_rt_abi_version() -> u32 {
    THAW_ABI_VERSION
}

/// NUL-terminated 16-hex-char ABI hash, same value as `ABI_MARKER` embeds.
#[no_mangle]
pub extern "C" fn thaw_rt_abi_hash() -> *const c_char {
    ABI_HASH_CSTR.as_ptr().cast()
}

/// 0 = ok, 1 = bad config, 2 = version mismatch. Null config = defaults.
///
/// # Safety
/// `config` must be null or point to a readable `ThawConfig`.
#[no_mangle]
pub unsafe extern "C" fn thaw_rt_init(config: *const ThawConfig) -> i32 {
    if !config.is_null() {
        let cfg = unsafe { &*config };
        if cfg.abi_version != THAW_ABI_VERSION {
            return 2;
        }
        if cfg.reserved != [0; 7] {
            return 1;
        }
    }
    INIT_COUNT.fetch_add(1, Ordering::SeqCst);
    0
}

#[no_mangle]
pub extern "C" fn thaw_rt_shutdown() {
    let _ = INIT_COUNT.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn init_validates_config_and_counts() {
        assert_eq!(unsafe { thaw_rt_init(std::ptr::null()) }, 0);
        let bad = ThawConfig {
            abi_version: 99,
            reserved: [0; 7],
        };
        assert_eq!(unsafe { thaw_rt_init(&bad) }, 2);
        let reserved = ThawConfig {
            abi_version: 1,
            reserved: [1, 0, 0, 0, 0, 0, 0],
        };
        assert_eq!(unsafe { thaw_rt_init(&reserved) }, 1);
        thaw_rt_shutdown();
        thaw_rt_shutdown();
        thaw_rt_shutdown();
        assert_eq!(INIT_COUNT.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn exports_file_is_sorted_and_excludes_cabi_names() {
        let lines: Vec<&str> = include_str!("../exports.txt").lines().collect();
        assert!(!lines.is_empty());
        assert!(
            lines.windows(2).all(|w| w[0] < w[1]),
            "exports.txt must be sorted and unique"
        );
        for name in [
            "thaw_rt_abi_version",
            "thaw_rt_abi_hash",
            "thaw_rt_init",
            "thaw_rt_shutdown",
        ] {
            assert!(!lines.contains(&name), "{name} is defined in this crate");
        }
    }

    #[test]
    fn abi_hash_is_16_hex_and_stable() {
        let a = unsafe { CStr::from_ptr(thaw_rt_abi_hash()) }
            .to_str()
            .unwrap()
            .to_owned();
        let b = unsafe { CStr::from_ptr(thaw_rt_abi_hash()) }
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, b);
        assert_eq!(thaw_rt_abi_version(), 1);
    }
}
