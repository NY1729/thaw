#[path = "system/async_hooks.rs"]
mod async_hooks;
#[path = "system/environment.rs"]
mod environment;
#[path = "system/runtime.rs"]
mod runtime;
#[path = "system/workers.rs"]
mod workers;

pub(super) fn source(name: &str) -> Option<&'static str> {
    async_hooks::source(name)
        .or_else(|| environment::source(name))
        .or_else(|| runtime::source(name))
        .or_else(|| workers::source(name))
}
