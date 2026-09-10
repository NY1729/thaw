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
    /// mapped to the version `npm install` resolved *it* to. Always
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

fn select_prebuilt_addon(package_dir: &Path) -> Result<Option<SelectedPrebuild>, String> {
    let prebuilds = package_dir.join("prebuilds");
    if !prebuilds.is_dir() {
        return Ok(None);
    }
    let (platform, arch, libc) = target_prebuild_components();
    let target_dir = prebuilds.join(format!("{platform}-{arch}"));
    if !target_dir.is_dir() {
        let mut available = fs::read_dir(&prebuilds)
            .map_err(|error| format!("failed to inspect `{}`: {error}", prebuilds.display()))?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
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
        return Err(format!(
            "no bundled native addon matches {platform}-{arch}-{libc}; available targets: {targets}{hint}"
        ));
    }
    let mut candidates = fs::read_dir(&target_dir)
        .map_err(|error| format!("failed to inspect `{}`: {error}", target_dir.display()))?
        .filter_map(Result::ok)
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
    let Some(path) = candidates.into_iter().next() else {
        return Err(format!(
            "bundled addons exist for {platform}-{arch}, but none match libc `{libc}`"
        ));
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

fn select_optional_dependency_addon(
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
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
        let dependency_dir = node_modules_dir.join(name);
        if !dependency_dir.is_dir() {
            continue;
        }
        let dependency_manifest = read_manifest(&dependency_dir)?;
        let main = dependency_manifest
            .get("main")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("index.js");
        let main_path = dependency_dir.join(main);
        let path = if main_path
            .extension()
            .is_some_and(|extension| extension == "node")
            && main_path.is_file()
        {
            Some(main_path)
        } else {
            find_node_file(&dependency_dir)?
        };
        if let Some(path) = path {
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
        .filter(|version| *version <= 8)
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

fn find_node_file(root: &Path) -> Result<Option<PathBuf>, String> {
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

fn select_generated_addon(
    node_modules_dir: &Path,
    package_source: &str,
) -> Result<Option<SelectedPrebuild>, String> {
    let mut matches = fs::read_dir(node_modules_dir)
        .map_err(|error| format!("failed to inspect `{}`: {error}", node_modules_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().is_dir()
                && entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_name() != ".bin"
                && package_source.contains(entry.file_name().to_string_lossy().as_ref())
        })
        .filter_map(|entry| match find_node_file(&entry.path()) {
            Ok(Some(path)) => Some(Ok(path)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    for name in optional.keys().filter(|name| markers.iter().any(|marker| name.contains(marker))) {
        let root = node_modules_dir.join(name);
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

fn download_prebuild_install_addon(
    package_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<(), String> {
    if package_dir.join("prebuilds").is_dir() {
        return Ok(());
    }
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
    let decoder = flate2::read::GzDecoder::new(archive_bytes.as_slice());
    tar::Archive::new(decoder)
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
