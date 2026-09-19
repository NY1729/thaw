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
/// package spec from the command's own arguments. `-p`/`--package <spec>`
/// resolves that package and runs the command named by the first
/// positional argument (`thaw x -p prisma prisma generate` style),
/// matching `npx`; a package exposing several bins without one matching
/// its name is rejected with the list of available commands.
fn run_x(args: &[String]) -> Result<i32, String> {
    let invocation = parse_x_args(args)?;
    let cwd = std::env::current_dir()
        .map_err(|error| format!("failed to read the current directory: {error}"))?;
    let specs = invocation.specs();
    let mut bins: Vec<(String, PathBuf)> = Vec::new();
    for spec in &specs {
        let package = thaw_registry::package_name(spec).to_string();
        let package_dir = match find_local_package_dir(&package, &cwd) {
            Some(directory) => directory,
            None => install_into_cache(spec)?,
        };
        bins.extend(package_bins(&package_dir, &package)?);
    }
    let bin = match &invocation.command {
        Some(command) => select_bin(&bins, command, &specs)?,
        None => default_bin(&bins, thaw_registry::package_name(&specs[0]))?,
    };
    bin_command(&bin)?
        .args(&invocation.arguments)
        .status()
        .map(|status| status.code().unwrap_or(1))
        .map_err(|error| format!("failed to run `{}`: {error}", bin.display()))
}

/// Parsed form of a `thaw x` invocation.
struct XInvocation {
    /// Package specs from `-p`/`--package`, in order (empty when the plain
    /// `thaw x <package>` form was used).
    packages: Vec<String>,
    /// Positional package specifier, for the plain form.
    spec: Option<String>,
    /// Explicit command to run, required when `-p` was given.
    command: Option<String>,
    /// Everything after the spec/command, forwarded verbatim to the bin.
    arguments: Vec<String>,
}

impl XInvocation {
    fn specs(&self) -> Vec<String> {
        match &self.spec {
            Some(spec) => vec![spec.clone()],
            None => self.packages.clone(),
        }
    }
}

fn parse_x_args(args: &[String]) -> Result<XInvocation, String> {
    let mut packages = Vec::new();
    let mut positionals: Vec<String> = Vec::new();
    let mut options_done = false;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        if !options_done {
            match argument {
                // `npx` compatibility: `thaw x` never prompts or writes to
                // the project, so `-y`/`--yes` only has to be accepted.
                "-y" | "--yes" => {
                    index += 1;
                    continue;
                }
                "-p" | "--package" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or("--package requires a package argument")?;
                    packages.push(value.clone());
                    index += 1;
                    continue;
                }
                other if other.starts_with("--package=") => {
                    packages.push(other["--package=".len()..].to_string());
                    index += 1;
                    continue;
                }
                "--" => {
                    options_done = true;
                    index += 1;
                    continue;
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option `{other}`"));
                }
                _ => {}
            }
        }
        // The first positional ends option parsing; everything from here
        // is forwarded verbatim.
        positionals.extend(args[index..].iter().cloned());
        break;
    }

    if packages.is_empty() {
        let mut positionals = positionals.into_iter();
        let spec = positionals
            .next()
            .ok_or("usage: thaw x <package>[@<version>] [--] [arguments...] (missing package name)")?;
        return Ok(XInvocation {
            packages,
            spec: Some(spec),
            command: None,
            arguments: drop_separator(positionals.collect()),
        });
    }
    let mut positionals = positionals.into_iter();
    let command = positionals.next().ok_or(
        "usage: thaw x -p <package>[@<version>] [--] <command> [arguments...] (missing command)",
    )?;
    Ok(XInvocation {
        packages,
        spec: None,
        command: Some(command),
        arguments: drop_separator(positionals.collect()),
    })
}

/// Drops the single `--` that separates a `thaw x` spec/command from the
/// arguments to forward (`thaw x pkg -- --flag` runs with `--flag`), while
/// preserving any later `--` (`thaw x pkg a -- b` forwards all three).
fn drop_separator(mut arguments: Vec<String>) -> Vec<String> {
    if arguments.first().map(String::as_str) == Some("--") {
        arguments.remove(0);
    }
    arguments
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

/// Every command a package's `bin` entry declares, as `(name, path)`. A
/// bare string `bin` is the package's own command (its unscoped
/// basename); an object maps each command name to its file. A non-string
/// value is ignored; an unsupported `bin` shape is an error; no `bin` at
/// all is an empty list (the caller decides whether that is fatal).
fn package_bins(package_dir: &Path, package: &str) -> Result<Vec<(String, PathBuf)>, String> {
    let manifest = std::fs::read_to_string(package_dir.join("package.json"))
        .map_err(|error| format!("failed to read `{}`: {error}", package_dir.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|error| format!("invalid package.json for `{package}`: {error}"))?;
    Ok(match manifest.get("bin") {
        Some(serde_json::Value::String(path)) => vec![(
            package_basename(package).to_string(),
            bin_path(package_dir, path),
        )],
        Some(serde_json::Value::Object(entries)) => entries
            .iter()
            .filter_map(|(name, value)| match value {
                serde_json::Value::String(path) => {
                    Some((name.clone(), bin_path(package_dir, path)))
                }
                _ => None,
            })
            .collect(),
        Some(_) => return Err(format!("`{package}` has an unsupported `bin` entry")),
        None => Vec::new(),
    })
}

/// The command `thaw x <package>` runs by default: the one named after the
/// package's own basename, or, failing that, a package's sole command
/// (matching `npx`'s single-bin convenience). Multiple unmatched commands
/// are rejected with the list and a `-p` hint.
fn default_bin(bins: &[(String, PathBuf)], package: &str) -> Result<PathBuf, String> {
    let command = package_basename(package);
    if let Some((_, path)) = bins.iter().find(|(name, _)| name == command) {
        return Ok(path.clone());
    }
    match bins {
        [] => Err(format!("`{package}` does not declare a `bin` command")),
        [(_, path)] => Ok(path.clone()),
        _ => Err(format!(
            "`{package}` exposes multiple commands ({}) and none is named `{command}`; \
             choose one with `-p {package} <command>`",
            command_names(bins)
        )),
    }
}

/// The bin an explicit `-p`/`--package` invocation asks for by name.
fn select_bin(
    bins: &[(String, PathBuf)],
    command: &str,
    packages: &[String],
) -> Result<PathBuf, String> {
    bins.iter()
        .find(|(name, _)| name == command)
        .map(|(_, path)| path.clone())
        .ok_or_else(|| {
            let available = if bins.is_empty() {
                "no commands".to_string()
            } else {
                format!("available: {}", command_names(bins))
            };
            format!(
                "no command `{command}` found in {} ({available})",
                packages.join(", ")
            )
        })
}

fn command_names(bins: &[(String, PathBuf)]) -> String {
    let mut names = bins
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names.join(", ")
}

fn package_basename(package: &str) -> &str {
    package.rsplit('/').next().unwrap_or(package)
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
