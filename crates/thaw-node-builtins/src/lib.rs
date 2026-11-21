//! Node.js builtin module sources, served by name (no `node:` prefix).
//!
//! thaw-registry bundles these at build time; thaw-js loads them at run time in worker realms.
//! This crate is a leaf (no dependencies) so both can depend on it.

#![forbid(unsafe_code)]

/// The CommonJS source of builtin `name` (`path`, `path/posix`, ...), or `None` when this crate does not serve it.
pub fn source(name: &str) -> Option<&'static str> {
    paths::source(name)
        .or_else(|| util::source(name))
        .or_else(|| events::source(name))
        .or_else(|| url::source(name))
}

mod events;
mod paths;
mod url;
mod util;

#[cfg(test)]
mod tests {
    use super::source;

    #[test]
    fn path_builtins_are_served_by_name() {
        for name in ["path", "path/posix", "path/win32"] {
            assert!(source(name).is_some(), "{name} is not served");
        }
        assert!(source("no-such-builtin").is_none());
    }

    #[test]
    fn path_exports_the_posix_flavour() {
        assert!(source("path").unwrap().contains("module.exports = posix;"));
    }
}
