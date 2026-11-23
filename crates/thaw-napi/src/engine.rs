//! Engine seam: every JS-engine symbol thaw-napi uses goes through this module.

#[cfg(feature = "engine-thaw-js")]
mod tjs;
#[allow(unused_imports)]
#[cfg(feature = "engine-thaw-js")]
pub use tjs::*;
