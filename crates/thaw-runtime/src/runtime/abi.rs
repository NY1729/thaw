pub const THAW_FD_READABLE: u8 = 1;
pub const THAW_FD_WRITABLE: u8 = 2;

pub type HandlerFn = extern "C" fn(*const c_char) -> *const c_char;
pub type HandlerErrorSlot = *mut *const c_char;
pub type PromiseResumeFn = extern "C" fn(*mut u8, *const u8);
pub type PromiseTransformFn = extern "C" fn(*mut u8, *mut ThawPromise, *const u8);
pub type PromiseFinallyFn = extern "C" fn(*mut u8, *mut ThawPromise, *const u8, u8);
pub type FdWatcherFn = extern "C" fn(*mut u8, i16);
