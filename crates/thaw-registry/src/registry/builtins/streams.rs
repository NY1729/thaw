#[path = "streams/basic.rs"]
mod basic;
#[path = "streams/core.rs"]
mod core;
#[path = "streams/extras.rs"]
mod extras;

pub(super) fn source(name: &str) -> Option<&'static str> {
    basic::source(name)
        .or_else(|| core::source(name))
        .or_else(|| extras::source(name))
}
