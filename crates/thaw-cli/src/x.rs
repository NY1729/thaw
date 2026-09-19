/// `thaw x <package>[@<version>] [--] [arguments...]` -- the `npx`/`bun x`
/// equivalent: run an npm package's declared `bin` once, without adding it
/// to the project.
///
/// Deliberately *not* a full `npm exec` clone, but it closes the one gap
/// that made `npx prisma generate`-style commands impossible: nothing in
/// thaw ever executed a package's own `bin`. Resolution:
///
/// 1. An already-installed `node_modules/<package>` walking up from the
///    current directory -- the common case, identical to what `npx` does
///    first.
/// 2. Else `npm install --prefix <cache> <spec>` into a per-spec cache
///    under the user cache directory, so a one-shot run never touches the
///    project's own `node_modules` (and is reused next time). This reuses
///    thaw's existing npm dependency rather than reimplementing tarball
///    fetch/tag resolution.
///
/// The binary is executed through its own `bin` target. On Unix an
/// executable target runs directly, so its `#!/usr/bin/env node` shebang
/// picks the runtime exactly like `npx`'s shim does; a target that lost
/// its executable bit (some archives do) falls back to its shebang
/// interpreter. Output, environment, working directory, and exit code are
/// all forwarded untouched.
///
/// Options: `-y`/`--yes` is accepted for `npx` compatibility (nothing is
/// ever prompted for or written to the project). `--` separates the
/// package spec from the command's own arguments. `-p`/`--package` and
/// alternate-command selection (`npx -p pkg binname`) are not implemented
/// yet; a package exposing several bins without one matching its name is
/// rejected with the list of available commands.
fn run_x(args: &[String]) -> Result<i32, String> {
    let invocation = parse_x_args(args)?;
    let package = thaw_registry::package_name(&invocation.spec).to_string();
    let cwd = std::env::current_dir()
        .map_err(|error| format!("failed to read the current directory: {error}"))?;
    let package_dir = match find_local_package_dir(&package, &cwd) {
        Some(directory) => directory,
        None => install_into_cache(&invocation.spec)?,
    };
    let (_, bin) = resolve_package_bin(&package_dir, &package)?;
    bin_command(&bin)?
        .args(&invocation.arguments)
        .status()
        .map(|status| status.code().unwrap_or(1))
        .map_err(|error| format!("failed to run `{}`: {error}", bin.display()))
}

/// Parsed form of a `thaw x` invocation.
struct XInvocation {
    /// The `package` or `package@version` specifier to resolve.
    spec: String,
    /// Everything after the spec, forwarded verbatim to the bin.
    arguments: Vec<String>,
}

fn parse_x_args(args: &[String]) -> Result<XInvocation, String> {
    let mut spec: Option<String> = None;
    let mut arguments = Vec::new();
    let mut options_done = false;
    for (index, argument) in args.iter().enumerate() {
        if spec.is_none() {
            match argument.as_str() {
                // `npx` compatibility: `thaw x` never prompts or writes to
                // the project, so `-y`/`--yes` only has to be accepted.
                "-y" | "--yes" => {}
                "--" => options_done = true,
                other if other.starts_with('-') && !options_done => {
                    return Err(format!("unknown option `{other}`"));
                }
                other => spec = Some(other.to_string()),
            }
            continue;
        }
        // The first `--` after the spec belongs to the specifier/argument
        // split, not to the command; every later token is forwarded as-is.
        if arguments.is_empty() && argument == "--" {
            continue;
        }
        arguments.extend(args[index..].iter().cloned());
        break;
    }
    Ok(XInvocation {
        spec: spec.ok_or(
            "usage: thaw x <package>[@<version>] [--] [arguments...] (missing package name)",
        )?,
        arguments,
    })
}

/// Finds an installed `node_modules/<package>` by walking up from `start`,
/// matching Node's own upward `node_modules` lookup.
fn find_local_package_dir(package: &str, start: &Path) -> Option<PathBuf> {
    let mut directory = start.canonicalize().ok()?;
    loop {
        let candidate = directory.join("node_modules").join(package);
        if candidate.join("package.json").is_file() {
            return Some(candidate);
        }
        directory = directory.parent()?.to_path_buf();
    }
}

/// Installs `spec` into a per-spec cache directory with the project's own
/// `npm`, returning the installed package directory. Kept project-local-
/// only by construction: `--prefix` points at the cache, never at the
/// caller's directory.
fn install_into_cache(spec: &str) -> Result<PathBuf, String> {
    let cache = x_cache_root()?.join(spec.replace(['/', '@'], "_"));
    let package = thaw_registry::package_name(spec);
    let package_dir = cache.join("node_modules").join(package);
    if package_dir.join("package.json").is_file() {
        return Ok(package_dir);
    }
    std::fs::create_dir_all(&cache)
        .map_err(|error| format!("failed to create `{}`: {error}", cache.display()))?;
    let status = Command::new("npm")
        .args(["install", "--prefix"])
        .arg(&cache)
        .args(["--no-save", "--no-package-lock", "--no-audit", "--no-fund"])
        .arg(spec)
        .status()
        .map_err(|error| format!("failed to run npm install: {error}"))?;
    if !status.success() {
        return Err(format!("npm install `{spec}` failed with {status}"));
    }
    if !package_dir.join("package.json").is_file() {
        return Err(format!(
            "`{spec}` did not install a package at `{}`",
            package_dir.display()
        ));
    }
    Ok(package_dir)
}

/// `$XDG_CACHE_HOME/thaw/x` (or `$HOME/.cache/thaw/x`, or the temp
/// directory as a last resort).
fn x_cache_root() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    Ok(base.join("thaw").join("x"))
}

/// Resolves the executable a package's `bin` entry maps to, matching
/// `npx`'s own precedence: a bin key named after the package (its
/// unscoped basename) wins; a bare string `bin` is that package's own
/// command; a single-entry object is unambiguous; anything else needs an
/// explicit command (not implemented yet, so it is rejected clearly).
fn resolve_package_bin(package_dir: &Path, package: &str) -> Result<(String, PathBuf), String> {
    let manifest = std::fs::read_to_string(package_dir.join("package.json"))
        .map_err(|error| format!("failed to read `{}`: {error}", package_dir.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|error| format!("invalid package.json for `{package}`: {error}"))?;
    let command = package.rsplit('/').next().unwrap_or(package);
    let bin = manifest.get("bin");
    match bin {
        Some(serde_json::Value::String(path)) => Ok((command.to_string(), bin_path(package_dir, path))),
        Some(serde_json::Value::Object(entries)) => {
            if let Some(serde_json::Value::String(path)) = entries.get(command) {
                return Ok((command.to_string(), bin_path(package_dir, path)));
            }
            if entries.len() == 1 {
                if let Some((name, serde_json::Value::String(path))) = entries.iter().next() {
                    return Ok((name.clone(), bin_path(package_dir, path)));
                }
            }
            let mut names = entries.keys().cloned().collect::<Vec<_>>();
            names.sort();
            Err(format!(
                "`{package}` exposes multiple commands ({}) and none is named `{command}`; \
                 running a specific command is not supported yet",
                names.join(", ")
            ))
        }
        Some(_) => Err(format!("`{package}` has an unsupported `bin` entry")),
        None => Err(format!("`{package}` does not declare a `bin` command")),
    }
}

fn bin_path(package_dir: &Path, declared: &str) -> PathBuf {
    package_dir.join(declared.trim_start_matches("./"))
}

/// Builds the command that runs `path`. An executable target is spawned
/// directly (its shebang chooses the runtime); otherwise the shebang is
/// read and its interpreter is invoked explicitly, so a bin whose archive
/// dropped the executable bit still runs. A target with no shebang at all
/// is an error rather than a guess.
fn bin_command(path: &Path) -> Result<Command, String> {
    if is_executable(path) {
        return Ok(Command::new(path));
    }
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let shebang = source
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("#!"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| {
            format!(
                "`{}` is not executable and has no shebang interpreter",
                path.display()
            )
        })?;
    let mut parts = shebang.split_whitespace();
    let interpreter = parts
        .next()
        .ok_or_else(|| format!("`{}` has an empty shebang", path.display()))?;
    let mut command = Command::new(interpreter);
    command.args(parts);
    command.arg(path);
    Ok(command)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    false
}
