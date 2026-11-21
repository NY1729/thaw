#[path = "core/process.rs"]
mod process;

pub(super) fn source(name: &str) -> Option<&'static str> {
    process::source(name)
}
