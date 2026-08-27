#[path = "http/compat.rs"]
mod compat;
#[path = "http/http1.rs"]
mod http1;
#[path = "http/http2.rs"]
mod http2;

pub(super) fn source(name: &str) -> Option<&'static str> {
    http1::source(name)
        .or_else(|| http2::source(name))
        .or_else(|| compat::source(name))
}
