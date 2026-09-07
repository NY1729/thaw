//! V1 package registry: a plain local directory, one subdirectory per
//! package (see docs/design/registry.md):
//!
//! ```text
//! <registry-dir>/<package>/
//!   package.d.ts   (required)  -- fed to thaw-bridge's parse_dts/classify
//!   native.a       (optional)  -- prebuilt static lib providing the
//!                                  package's Fast path native symbols;
//!                                  auto-linked by thaw-cli (replaces a
//!                                  manual `--link`)
//!   native.node    (optional)  -- matching bundled N-API prebuild selected
//!                                  by `add`; loaded by thaw-napi
//!   native-addon.json (optional) -- source path, target tuple, and SHA-256
//!                                  for `native.node`
//!   bundle.js[.gz] (optional)  -- real JS implementation backing the
//!                                  package's Fallback functions; its
//!                                  source is fed to thaw-bridge's
//!                                  `generate_module_init` so it's loaded
//!                                  automatically at program startup
//!                                  (replaces a manual `loadScript` call)
//!   subpaths/<path>/package.d.ts -- declarations for an exact `exports`
//!                                  subpath such as `./feature`
//!   subpaths/<path>/bundle.js[.gz] -- independently bundled runtime entry
//!                                  for that subpath
//!   version.txt    (optional)  -- the exact version `add` resolved and
//!                                  fetched for the package itself (see
//!                                  `add`'s doc comment); purely
//!                                  informational
//!   lock.json      (optional)  -- every *other* real npm package folded
//!                                  into `bundle.js` (transitive same-
//!                                  registry-install dependencies), each
//!                                  mapped to the version `npm` actually
//!                                  resolved it to; written only when
//!                                  there's at least one (a single-file
//!                                  package with no dependencies has
//!                                  nothing to record here beyond
//!                                  `version.txt`'s own package). Purely
//!                                  informational, same as `version.txt`.
//! ```
//!
//! Still no source build step for native code, and no real dependency-graph
//! *resolution* (no semver range solving of our own -- `npm install`
//! already did that once, for one `add` call, and `lock.json` just
//! records what it picked) -- `add` resolves and records versions for
//! exactly the packages one `npm install <package>@<spec>` call actually
//! pulled in, independently each time `add` runs. This crate only
//! resolves a package name to the files already sitting on disk (plus,
//! now, the versions `add` recorded there). The native-lib build pipeline
//! is still exactly what the project's design doc calls "the actual
//! differentiator" left undone; this is a placeholder for the local half
//! of it, real enough to remove the remaining manual
//! `--bridge`/`--link`/`loadScript` steps for a package that's already
//! been fetched/built by some other means. `add` selects already-bundled
//! `.node` prebuilds and `prebuild-install`-style GitHub Release assets
//! described by the package manifest; it never runs package install scripts
//! or `node-gyp`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

include!("registry/resolution.rs");

include!("registry/install.rs");

include!("registry/bundle.rs");

#[cfg(test)]
mod tests;
