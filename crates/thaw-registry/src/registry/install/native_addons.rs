mod napi_version {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../thaw-napi/src/napi/version.rs"));
}

/// Where a freshly `add`ed package's declarations/JS entry came from,
/// inside the fetched package itself -- informational only, since the
/// scratch directory these were read from is deleted before `add`
/// returns. `bundled_file_count` is how many of the package's own files
/// (`main` plus everything it reaches via same-package relative
/// `require`s) got folded into `bundle.js` -- 1 for a package that's
/// just a single file.
#[derive(Debug, PartialEq)]
pub struct AddedPackage {
    pub dts_relative_path: String,
    pub js_relative_path: String,
    pub bundled_file_count: usize,
    /// The exact version `npm install` actually resolved `package`'s
    /// version-or-range specifier to (read back from the fetched
    /// package's own `package.json`, so a bare package name with no
    /// specifier at all still reports the real version `npm` picked as
    /// "latest", not just an echo of the empty request).
    pub resolved_version: String,
    /// Every real npm package folded into `bundle.js` -- `package` itself
    /// plus every transitive same-install dependency `bundle_commonjs_
    /// package`'s worklist actually walked into (Node builtin polyfills
    /// are not real npm packages and are never included here) -- each
    /// mapped to the version `npm install` resolved *it* to. Unique names
    /// retain their historical map keys; duplicate installed names use
    /// node_modules-relative package instance locators. Always
    /// contains at least `package`'s own entry. Written to `lock.json`
    /// only when it has more than that one entry (see this module's
    /// top-level doc comment).
    pub dependency_versions: BTreeMap<String, String>,
    /// Metadata for an automatically selected bundled `.node` prebuild.
    pub native_addon: Option<NativeAddonMetadata>,
    /// A non-fatal explanation when prebuilds existed but none matched the
    /// current target. The JavaScript fallback remains usable in that case.
    pub native_diagnostic: Option<String>,
}

/// Environment defaults a package needs at run time, e.g. a loader that finds its native library
/// through an environment variable, where `$self` stands for this executable (it embeds every
/// native addon). Declared by the package itself (`"thaw": { "runtimeEnv": { NAME: VALUE } }`) or
/// by thaw's curated data table; the package's own declaration wins.
fn package_runtime_env(manifest: &serde_json::Value) -> BTreeMap<String, String> {
    let defaults: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_str(include_str!("runtime-env-defaults.json")).unwrap_or_default();
    let mut env = manifest
        .get("name")
        .and_then(serde_json::Value::as_str)
        .and_then(|name| defaults.get(name).cloned())
        .unwrap_or_default();
    if let Some(declared) = manifest.pointer("/thaw/runtimeEnv").and_then(serde_json::Value::as_object) {
        env.extend(declared.iter().filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string()))));
    }
    env
}

// Match a complete target suffix in the package leaf, not a prefix of another
// architecture or libc variant (for example, arm versus arm64).
fn optional_dependency_matches_target(name: &str, marker: &str) -> bool {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    leaf == marker || leaf.strip_suffix(marker).is_some_and(|prefix| prefix.ends_with('-'))
}

fn select_optional_dependency_executable(
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<Option<PathBuf>, String> {
    let manifest_name = manifest.get("name").and_then(serde_json::Value::as_str).unwrap_or_default();
    let Some(optional) = manifest
        .get("optionalDependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(None);
    };
    let (platform, arch, libc) = target_prebuild_components();
    let markers = if platform == "linux" && libc == "musl" {
        vec![format!("linux-{arch}"), format!("linuxmusl-{arch}")]
    } else {
        vec![format!("{platform}-{arch}")]
    };
    for name in optional
        .keys()
        .filter(|name| markers.iter().any(|marker| optional_dependency_matches_target(name, marker)))
    {
        let root = join_contained_path(node_modules_dir, name)?;
        if !root.is_dir() {
            continue;
        }
        let manifest = read_manifest(&root)?;
        let binary = match manifest.get("bin") {
            Some(serde_json::Value::String(path)) => Some(path.as_str()),
            Some(serde_json::Value::Object(entries)) => {
                entries.values().find_map(serde_json::Value::as_str)
            }
            _ => None,
        };
        // Without a declared `bin`, the conventional location is `bin/<the package's own name>`.
        let own_name = manifest_name.rsplit('/').next().unwrap_or(manifest_name);
        let path = match binary {
            Some(path) => join_contained_path(&root, path)?,
            None => root.join("bin").join(own_name),
        };
        if path.is_file() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NativeAddonMetadata {
    pub source: String,
    pub sha256: String,
    pub platform: String,
    pub arch: String,
    pub libc: String,
}

#[derive(Debug)]
struct SelectedPrebuild {
    path: PathBuf,
    source: String,
    platform: String,
    arch: String,
    libc: String,
}

// A wrong target or unsupported ABI is a candidate mismatch: keep looking
// through the other bundled, optional, and generated routes. An inspection
// failure is not a mismatch and must still be reported to the caller.
fn usable_napi_prebuild(path: &Path) -> Result<bool, String> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if name.starts_with("node.abi") || name.starts_with("electron.abi") {
        return Ok(false);
    }
    // Separate a real read failure from a readable but unrecognized binary.
    fs::File::open(path)
        .map_err(|error| format!("failed to read native addon `{}`: {error}", path.display()))?;
    let args: &[&str] = if cfg!(target_os = "macos") {
        &["-g"]
    } else {
        &["-D", "-g"]
    };
    let output = Command::new("nm")
        .args(args)
        .arg(path)
        .output()
        .map_err(|error| format!("failed to inspect native addon `{}`: {error}", path.display()))?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(napi_symbol_suitability(&String::from_utf8_lossy(&output.stdout)))
}

fn napi_symbol_suitability(symbols: &str) -> bool {
    let mut direct = false;
    let mut registered = false;
    let mut node_internal = false;
    for line in symbols.lines() {
        let mut fields = line.split_whitespace().rev();
        let raw_symbol = fields.next().unwrap_or_default()
            .split('@').next().unwrap_or_default();
        // Mach-O's nm displays the C linker prefix; ELF's leading `_` is
        // part of the real dlsym name and must not be discarded.
        let symbol = if cfg!(target_os = "macos") {
            raw_symbol.strip_prefix('_').unwrap_or(raw_symbol)
        } else {
            raw_symbol
        };
        let kind = fields.next().unwrap_or_default();
        let undefined_function = kind == "U" || kind == "w";
        let undefined = undefined_function || kind == "v";
        direct |= (kind == "T" || kind == "W") && symbol == "napi_register_module_v1";
        registered |= undefined_function && symbol == "napi_module_register";
        node_internal |= undefined
            && (symbol == "node_module_register"
                || symbol.starts_with("_ZN2v8")
                || symbol.starts_with("_ZN4node")
                || symbol.starts_with("_ZNK2v8")
                || symbol.starts_with("_ZNK4node"));
    }
    (direct || registered) && !node_internal
}

fn target_prebuild_components() -> (&'static str, &'static str, &'static str) {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let libc = if cfg!(target_env = "musl") {
        "musl"
    } else if platform == "linux" {
        "glibc"
    } else {
        "system"
    };
    (platform, arch, libc)
}

/// The subdirectory-per-target layout (`prebuilds/<platform>-<arch>/*.node`)
/// below is `prebuildify`'s own convention (and what `node-gyp-build`
/// expects at require time) -- but a package can also just vendor a flat
/// `prebuilds/<platform>-<arch>.node` file directly, no per-target
/// subdirectory at all (real example: `better-sqlite3` 13 dropped its
/// `prebuild-install` dependency for exactly this simpler scheme). Same
/// platform/arch/libc naming rules as `select_optional_dependency_addon`'s
/// own suffix matching (`linuxmusl-<arch>` for musl, since that's the
/// literal directory-name convention real optional-dependency packages
/// already use for the same distinction).
fn flat_prebuild_file_name(platform: &str, arch: &str, libc: &str) -> String {
    if platform == "linux" && libc == "musl" {
        format!("linuxmusl-{arch}.node")
    } else {
        format!("{platform}-{arch}.node")
    }
}

fn select_prebuilt_addon_with(
    package_dir: &Path,
    diagnostics: &mut Vec<String>,
    usable: impl Fn(&Path) -> Result<bool, String>,
) -> Result<Option<SelectedPrebuild>, String> {
    let prebuilds = package_dir.join("prebuilds");
    if !prebuilds.is_dir() {
        return Ok(None);
    }
    let (platform, arch, libc) = target_prebuild_components();
    let flat_path = prebuilds.join(flat_prebuild_file_name(platform, arch, libc));
    if flat_path.is_file() && usable(&flat_path)? {
        let relative_path = flat_path
            .strip_prefix(package_dir)
            .unwrap_or(&flat_path)
            .to_string_lossy()
            .into_owned();
        return Ok(Some(SelectedPrebuild {
            path: flat_path,
            source: relative_path,
            platform: platform.into(),
            arch: arch.into(),
            libc: libc.into(),
        }));
    }
    if flat_path.is_file() {
        diagnostics.push(format!(
            "bundled native addon `{}` is not a supported N-API module",
            flat_path.display()
        ));
    }
    let target_dir = prebuilds.join(format!("{platform}-{arch}"));
    if !target_dir.is_dir() {
        let mut available = fs::read_dir(&prebuilds)
            .map_err(|error| format!("failed to inspect `{}`: {error}", prebuilds.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed to inspect `{}`: {error}", prebuilds.display()))?
            .into_iter()
            .filter(|entry| {
                entry.path().is_dir()
                    || entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "node")
            })
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        available.sort();
        let targets = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(", ")
        };
        let hint = if available.is_empty() {
            // Nothing was fetched at all -- the package publishes no
            // prebuilt binary for any target and expects a source build
            // at install time. thaw only ever fetches a `.node`, it never
            // runs node-gyp, so it can't recover from this itself.
            " -- this package version publishes no prebuilt binaries; \
             pin an older version that does, or vendor a prebuilt `.node` \
             under `prebuilds/<platform>-<arch>/`"
        } else {
            ""
        };
        diagnostics.push(format!(
            "no bundled native addon matches {platform}-{arch}-{libc}; available targets: {targets}{hint}"
        ));
        return Ok(None);
    }
    let mut candidates = fs::read_dir(&target_dir)
        .map_err(|error| format!("failed to inspect `{}`: {error}", target_dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect `{}`: {error}", target_dir.display()))?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "node")
        })
        .filter(|path| {
            let musl = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".musl."));
            (libc == "musl") == musl || platform != "linux"
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        (usize::from(!name.starts_with("node.")), name.into_owned())
    });
    let mut chosen = None;
    for path in candidates {
        if usable(&path)? {
            chosen = Some(path);
            break;
        }
        diagnostics.push(format!("bundled native addon `{}` is not a supported N-API module", path.display()));
    }
    let Some(path) = chosen else {
        diagnostics.push(format!(
            "bundled addons exist for {platform}-{arch}, but none support {libc} and the N-API host"
        ));
        return Ok(None);
    };
    let relative_path = path
        .strip_prefix(package_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned();
    let source = fs::read_to_string(package_dir.join(".thaw-prebuild-source"))
        .ok()
        .map(|source| source.trim().to_string())
        .filter(|source| !source.is_empty())
        .unwrap_or(relative_path);
    Ok(Some(SelectedPrebuild {
        path,
        source,
        platform: platform.into(),
        arch: arch.into(),
        libc: libc.into(),
    }))
}

fn select_optional_dependency_addon_with(
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
    diagnostics: &mut Vec<String>,
    usable: impl Fn(&Path) -> Result<bool, String>,
) -> Result<Option<SelectedPrebuild>, String> {
    let Some(optional) = manifest
        .get("optionalDependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(None);
    };
    let (platform, arch, libc) = target_prebuild_components();
    let suffixes = if platform == "linux" && libc == "musl" {
        vec![format!("-{platform}-{arch}-{libc}"), format!("-linuxmusl-{arch}")]
    } else if platform == "linux" {
        vec![
            format!("-{platform}-{arch}-{libc}"),
            format!("-{platform}-{arch}-gnu"),
            format!("-{platform}-{arch}"),
        ]
    } else {
        vec![format!("-{platform}-{arch}")]
    };
    let mut names = optional
        .keys()
        .filter(|name| suffixes.iter().any(|suffix| name.ends_with(suffix)))
        .collect::<Vec<_>>();
    names.sort();
    for name in names {
        let dependency_dir = join_contained_path(node_modules_dir, name)?;
        if !dependency_dir.is_dir() {
            continue;
        }
        let dependency_manifest = read_manifest(&dependency_dir)?;
        let main = dependency_manifest
            .get("main")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("index.js");
        let main_path = join_contained_path(&dependency_dir, main)?;
        let main_is_node = main_path
            .extension()
            .is_some_and(|extension| extension == "node")
            && main_path.is_file();
        let mut paths = find_node_files(&dependency_dir)?;
        // The package's declared entry is the first choice when usable, but
        // a legacy Node-ABI main must not conceal a N-API sibling.
        if main_is_node {
            paths.retain(|path| path != &main_path);
            paths.insert(0, main_path.clone());
        }
        let mut compatible = Vec::new();
        for path in paths {
            if !usable(&path)? {
                diagnostics.push(format!("optional native addon `{}` is not a supported N-API module", path.display()));
                continue;
            }
            if main_is_node && path == main_path {
                compatible.push(path);
                break;
            }
            compatible.push(path);
        }
        if compatible.len() > 1 {
            return Err(format!("optional dependency `{name}` contains multiple compatible native addons"));
        }
        if let Some(path) = compatible.pop() {
            let relative_path = path
                .strip_prefix(node_modules_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            return Ok(Some(SelectedPrebuild {
                path,
                source: relative_path,
                platform: platform.into(),
                arch: arch.into(),
                libc: libc.into(),
            }));
        }
    }
    Ok(None)
}

fn github_repository(manifest: &serde_json::Value) -> Option<String> {
    let repository = manifest.get("repository")?;
    let raw = repository
        .as_str()
        .or_else(|| repository.get("url").and_then(serde_json::Value::as_str))?;
    let normalized = raw
        .strip_prefix("git+")
        .unwrap_or(raw)
        .strip_prefix("https://github.com/")?
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let mut parts = normalized.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

fn prebuild_install_asset(
    manifest: &serde_json::Value,
) -> Option<(String, String, String, String)> {
    let binary = manifest.get("binary")?;
    let napi = binary
        .get("napi_versions")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .filter(|version| *version <= u64::from(napi_version::SUPPORTED_NAPI_VERSION))
        .max()?;
    let name = manifest.get("name")?.as_str()?;
    let version = manifest.get("version")?.as_str()?;
    let repository = github_repository(manifest)?;
    let (platform, arch, libc) = target_prebuild_components();
    let platform = if platform == "linux" && libc == "musl" {
        "linuxmusl"
    } else {
        platform
    };
    let asset = format!("{name}-v{version}-napi-v{napi}-{platform}-{arch}.tar.gz");
    let url = format!("https://github.com/{repository}/releases/download/v{version}/{asset}");
    Some((url, asset, platform.to_string(), arch.to_string()))
}

fn find_node_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut directories = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("failed to inspect `{}`: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to inspect an entry in `{}`: {error}",
                    directory.display()
                )
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "node")
            {
                matches.push(path);
            }
        }
    }
    matches.sort();
    Ok(matches)
}

fn select_generated_addon_with(
    node_modules_dir: &Path,
    package_source: &str,
    diagnostics: &mut Vec<String>,
    usable: impl Fn(&Path) -> Result<bool, String>,
) -> Result<Option<SelectedPrebuild>, String> {
    let roots = fs::read_dir(node_modules_dir)
        .map_err(|error| format!("failed to inspect `{}`: {error}", node_modules_dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect `{}`: {error}", node_modules_dir.display()))?;
    let mut matches = Vec::new();
    for entry in roots.into_iter()
        .filter(|entry| {
            entry.path().is_dir()
                && entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_name() != ".bin"
                && package_source.contains(entry.file_name().to_string_lossy().as_ref())
        })
    {
        for path in find_node_files(&entry.path())? {
            if usable(&path)? {
                matches.push(path);
            } else {
                diagnostics.push(format!("generated native addon `{}` is not a supported N-API module", path.display()));
            }
        }
    }
    if matches.len() > 1 {
        return Err("generated packages contain multiple native addons".into());
    }
    let Some(path) = matches.pop() else {
        return Ok(None);
    };
    let (platform, arch, libc) = target_prebuild_components();
    Ok(Some(SelectedPrebuild {
        source: path.display().to_string(),
        path,
        platform: platform.to_string(),
        arch: arch.to_string(),
        libc: libc.to_string(),
    }))
}

fn select_installed_addon(
    package_dir: &Path,
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
    package_source: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> Result<Option<SelectedPrebuild>, String> {
    select_installed_addon_with(
        package_dir,
        node_modules_dir,
        manifest,
        package_source,
        diagnostics,
        usable_napi_prebuild,
    )
}

fn select_installed_addon_with(
    package_dir: &Path,
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
    package_source: Option<&str>,
    diagnostics: &mut Vec<String>,
    usable: impl Fn(&Path) -> Result<bool, String>,
) -> Result<Option<SelectedPrebuild>, String> {
    if let Some(addon) = select_prebuilt_addon_with(package_dir, diagnostics, &usable)? {
        return Ok(Some(addon));
    }
    if let Some(addon) = select_optional_dependency_addon_with(node_modules_dir, manifest, diagnostics, &usable)? {
        return Ok(Some(addon));
    }
    match package_source {
        Some(source) => select_generated_addon_with(node_modules_dir, source, diagnostics, usable),
        None => Ok(None),
    }
}

fn platform_shared_libraries(
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<Vec<PathBuf>, String> {
    let Some(optional) = manifest
        .get("optionalDependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(Vec::new());
    };
    let (platform, arch, libc) = target_prebuild_components();
    let markers = if platform == "linux" && libc == "musl" {
        vec![format!("linuxmusl-{arch}"), format!("{platform}-{arch}-{libc}")]
    } else {
        vec![format!("{platform}-{arch}"), format!("{platform}-{arch}-{libc}")]
    };
    let mut libraries = Vec::new();
    for name in optional.keys().filter(|name| markers.iter().any(|marker| optional_dependency_matches_target(name, marker))) {
        let root = join_contained_path(node_modules_dir, name)?;
        if !root.is_dir() {
            continue;
        }
        let mut directories = vec![root];
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(&directory)
                .map_err(|error| format!("failed to inspect `{}`: {error}", directory.display()))?
            {
                let path = entry.map_err(|error| error.to_string())?.path();
                if path.is_dir() {
                    directories.push(path);
                } else if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(".so"))
                {
                    libraries.push(path);
                }
            }
        }
    }
    libraries.sort();
    Ok(libraries)
}

/// The npm registry used for version documents, honoring npm's own
/// `npm_config_registry` override. HTTPS only: the digest the tarball is
/// checked against comes from this document.
fn npm_registry_base() -> Result<String, String> {
    let base = std::env::var("npm_config_registry")
        .or_else(|_| std::env::var("NPM_CONFIG_REGISTRY"))
        .unwrap_or_else(|_| "https://registry.npmjs.org".into());
    let base = base.trim_end_matches('/').to_string();
    if !base.starts_with("https://") {
        return Err(format!(
            "npm registry `{base}` is not HTTPS; refusing to trust its integrity digests"
        ));
    }
    Ok(base)
}

fn https_get_bounded(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|error| format!("failed to fetch `{url}`: {error}"))?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| format!("failed to read `{url}`: {error}"))
}

/// The `dist` record npm stores for one exact `name@version`: the SRI
/// `integrity` string and the tarball URL. A version with no `sha512`
/// digest cannot be checked, so it is refused rather than loaded.
fn npm_dist_record(
    document: &serde_json::Value,
    name: &str,
    version: &str,
) -> Result<(String, String), String> {
    let dist = document.get("dist");
    let integrity = dist
        .and_then(|dist| dist.get("integrity"))
        .and_then(serde_json::Value::as_str)
        .filter(|integrity| !integrity.is_empty())
        .ok_or_else(|| {
            format!("npm registry has no `dist.integrity` for {name}@{version}; refusing to load its native code")
        })?;
    let tarball = dist
        .and_then(|dist| dist.get("tarball"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("npm registry has no `dist.tarball` for {name}@{version}"))?;
    if !tarball.starts_with("https://") {
        return Err(format!("npm tarball `{tarball}` for {name}@{version} is not HTTPS"));
    }
    Ok((integrity.to_string(), tarball.to_string()))
}

/// Standard base64 with padding, the encoding SRI digests use.
fn base64_standard(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * index)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Checks `bytes` against an npm `dist.integrity` value. Only `sha512-`
/// entries count (npm's own rule: any one matching entry is enough).
fn verify_sha512_integrity(bytes: &[u8], integrity: &str) -> Result<(), String> {
    let mut expected = integrity
        .split_whitespace()
        .filter_map(|entry| entry.strip_prefix("sha512-"))
        .peekable();
    if expected.peek().is_none() {
        return Err(format!("integrity `{integrity}` has no sha512 entry; refusing to load it"));
    }
    let digest = base64_standard(&sha2::Sha512::digest(bytes));
    if expected.any(|entry| entry == digest) {
        Ok(())
    } else {
        Err(format!(
            "tarball sha512 mismatch: registry says `{integrity}`, downloaded bytes are `sha512-{digest}`"
        ))
    }
}

/// Verifies `tarball` first and only then unpacks it, so a mismatched
/// download never reaches the filesystem.
fn unpack_verified_tarball(tarball: &[u8], integrity: &str, destination: &Path) -> Result<(), String> {
    verify_sha512_integrity(tarball, integrity)?;
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create `{}`: {error}", destination.display()))?;
    tar::Archive::new(gzip_size_capped(tarball))
        .unpack(destination)
        .map_err(|error| format!("failed to unpack verified tarball: {error}"))
}

/// Optional dependencies that are platform packages for this target --
/// the only packages whose native code (`.node`, shared libraries, the
/// platform executable) gets loaded from npm's install tree. Matched
/// generously: an over-match only means one extra verification, and a
/// package npm did not install is skipped by the caller.
fn target_platform_dependencies(manifest: &serde_json::Value) -> Vec<String> {
    let Some(optional) = manifest
        .get("optionalDependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };
    let (platform, arch, libc) = target_prebuild_components();
    let mut needles = vec![format!("{platform}-{arch}")];
    if platform == "linux" && libc == "musl" {
        needles.push(format!("linuxmusl-{arch}"));
    }
    optional
        .keys()
        .filter(|name| {
            let leaf = name.rsplit('/').next().unwrap_or(name);
            needles.iter().any(|needle| leaf.contains(needle.as_str()))
        })
        .cloned()
        .collect()
}

/// Replaces each installed target platform package with the copy inside
/// its own registry tarball, after checking that tarball against the
/// registry's `dist.integrity` for the exact installed version. The native
/// files loaded afterwards are therefore exactly the verified bytes.
fn install_verified_platform_packages(
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
    staging: &Path,
) -> Result<(), String> {
    let base = npm_registry_base()?;
    for (index, name) in target_platform_dependencies(manifest).iter().enumerate() {
        let installed = join_contained_path(node_modules_dir, name)?;
        if !installed.is_dir() {
            continue;
        }
        let version = read_manifest(&installed)?
            .get("version")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("installed `{name}` has no version"))?
            .to_string();
        let document: serde_json::Value = serde_json::from_slice(&https_get_bounded(
            &format!("{base}/{name}/{version}"),
            16 * 1024 * 1024,
        )?)
        .map_err(|error| format!("registry document for {name}@{version} is not JSON: {error}"))?;
        let (integrity, tarball_url) = npm_dist_record(&document, name, &version)?;
        let tarball = https_get_bounded(&tarball_url, 128 * 1024 * 1024)?;
        let unpack_dir = staging.join(format!("verified-platform-{index}"));
        if unpack_dir.exists() {
            fs::remove_dir_all(&unpack_dir)
                .map_err(|error| format!("failed to clear `{}`: {error}", unpack_dir.display()))?;
        }
        unpack_verified_tarball(&tarball, &integrity, &unpack_dir)
            .map_err(|error| format!("{name}@{version}: {error}"))?;
        let verified = unpack_dir.join("package");
        if !verified.is_dir() {
            return Err(format!("tarball for {name}@{version} has no `package/` root"));
        }
        fs::remove_dir_all(&installed)
            .map_err(|error| format!("failed to replace `{}`: {error}", installed.display()))?;
        fs::rename(&verified, &installed)
            .map_err(|error| format!("failed to install verified `{name}`: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod integrity_tests {
    use super::*;
    use std::io::Write;

    fn sri(bytes: &[u8]) -> String {
        format!("sha512-{}", base64_standard(&sha2::Sha512::digest(bytes)))
    }

    fn tarball_with(path: &str, body: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, body).unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thaw-integrity-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn base64_standard_matches_known_vectors() {
        assert_eq!(base64_standard(b""), "");
        assert_eq!(base64_standard(b"f"), "Zg==");
        assert_eq!(base64_standard(b"fo"), "Zm8=");
        assert_eq!(base64_standard(b"foo"), "Zm9v");
    }

    #[test]
    fn integrity_match_is_accepted_and_unpacked() {
        let tarball = tarball_with("package/bin/addon.node", b"native");
        let dest = scratch("match");
        unpack_verified_tarball(&tarball, &sri(&tarball), &dest).unwrap();
        assert_eq!(fs::read(dest.join("package/bin/addon.node")).unwrap(), b"native");
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn integrity_mismatch_is_rejected_without_extraction() {
        let tarball = tarball_with("package/bin/addon.node", b"tampered");
        let expected = sri(&tarball_with("package/bin/addon.node", b"original"));
        let dest = scratch("mismatch");
        let error = unpack_verified_tarball(&tarball, &expected, &dest).unwrap_err();
        assert!(error.contains("sha512 mismatch"), "{error}");
        assert!(!dest.exists(), "a mismatched tarball must not reach the filesystem");
    }

    #[test]
    fn missing_integrity_is_refused_for_native_loading() {
        let no_integrity = serde_json::json!({ "dist": { "tarball": "https://registry.npmjs.org/x/-/x-1.0.0.tgz" } });
        let error = npm_dist_record(&no_integrity, "x", "1.0.0").unwrap_err();
        assert!(error.contains("no `dist.integrity`"), "{error}");

        let sha1_only = serde_json::json!({ "dist": { "integrity": "sha1-AAAA", "tarball": "https://registry.npmjs.org/x/-/x-1.0.0.tgz" } });
        let (integrity, _) = npm_dist_record(&sha1_only, "x", "1.0.0").unwrap();
        let error = verify_sha512_integrity(b"anything", &integrity).unwrap_err();
        assert!(error.contains("no sha512 entry"), "{error}");

        let http = serde_json::json!({ "dist": { "integrity": sri(b"x"), "tarball": "http://example.com/x.tgz" } });
        assert!(npm_dist_record(&http, "x", "1.0.0").is_err());
    }
}

/// Downloads the GitHub prebuild for a package. Callers must have checked the
/// opt-in; the download has no trusted digest.
fn download_prebuild_install_addon(
    package_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<(), String> {
    let Some((url, asset, _, _)) = prebuild_install_asset(manifest) else {
        return Ok(());
    };
    let mut response = ureq::get(&url)
        .call()
        .map_err(|error| format!("failed to download native prebuild `{url}`: {error}"))?;
    let archive_bytes = response
        .body_mut()
        .with_config()
        .limit(128 * 1024 * 1024)
        .read_to_vec()
        .map_err(|error| format!("failed to read native prebuild `{url}`: {error}"))?;
    let unpack_dir = package_dir.join(".thaw-prebuild");
    if unpack_dir.is_dir() {
        fs::remove_dir_all(&unpack_dir)
            .map_err(|error| format!("failed to clear `{}`: {error}", unpack_dir.display()))?;
    }
    fs::create_dir_all(&unpack_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", unpack_dir.display()))?;
    tar::Archive::new(gzip_size_capped(archive_bytes.as_slice()))
        .unpack(&unpack_dir)
        .map_err(|error| format!("failed to unpack native prebuild `{asset}`: {error}"))?;
    let addon = find_node_file(&unpack_dir)?
        .ok_or_else(|| format!("native prebuild `{asset}` contains no `.node` file"))?;
    let (platform, arch, _) = target_prebuild_components();
    let target = package_dir
        .join("prebuilds")
        .join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target)
        .map_err(|error| format!("failed to create `{}`: {error}", target.display()))?;
    let destination =
        target.join(addon.file_name().ok_or_else(|| {
            format!("native prebuild path `{}` has no filename", addon.display())
        })?);
    fs::copy(&addon, &destination).map_err(|error| {
        format!(
            "failed to copy downloaded native addon to `{}`: {error}",
            destination.display()
        )
    })?;
    fs::write(package_dir.join(".thaw-prebuild-source"), url)
        .map_err(|error| format!("failed to record downloaded native prebuild source: {error}"))?;
    Ok(())
}

fn find_node_file(root: &Path) -> Result<Option<PathBuf>, String> {
    let mut matches = find_node_files(root)?;
    if matches.len() > 1 {
        return Err(format!(
            "downloaded prebuild contains multiple `.node` files: {}",
            matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(matches.pop())
}
