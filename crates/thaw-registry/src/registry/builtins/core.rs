#[path = "core/events.rs"]
mod events;
#[path = "core/paths.rs"]
mod paths;
#[path = "core/process.rs"]
mod process;
#[path = "core/url.rs"]
mod url;
#[path = "core/util.rs"]
mod util;

pub(super) fn source(name: &str) -> Option<&'static str> {
    util::source(name)
        .or_else(|| process::source(name))
        .or_else(|| paths::source(name))
        .or_else(|| url::source(name))
        .or_else(|| events::source(name))
}
