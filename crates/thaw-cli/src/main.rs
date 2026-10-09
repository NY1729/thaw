use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use inkwell::context::Context;
use thaw_llvm::HirCompiler;

mod module_graph;
include!("compat.rs");
include!("node_compat.rs");
include!("completeness.rs");

const USAGE: &str = "usage: thaw <script.ts> [arguments...]\n       thaw prepare\n       thaw install [directory]\n       thaw add <package>... [--prefix <directory>]\n       thaw run <file.ts> [arguments...]\n       thaw run <package-script> [--prefix <directory>]\n       thaw x <package>[@<version>] [--] [arguments...]\n       thaw dev <input.ts|project> [build options]\n       thaw build <input.ts|project> [-o <output>] [--static] [--external-native] [--no-install] [--icu4c] [--assets <directory> | --vite <directory>] [--link <path>]... [--bridge <path.d.ts>]... [--ffi-metadata <path.json>]... [--registry <dir>] [--use <package>]...\n       thaw inspect <executable>\n       thaw compat [manifest.json]\n       thaw node-compat [manifest.json]\n       thaw completeness [--json] [--with-node]\n       thaw registry add <package>[@<version>] [--registry <dir>] [--from-node-modules <dir>]\n       thaw --help\n       thaw --version";

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("--version" | "-V") => println!("thaw {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h" | "help") => println!("{USAGE}"),
        Some(_) if matches!(args.get(2).map(String::as_str), Some("--help" | "-h")) => {
            println!("{USAGE}")
        }
        Some("build") => {
            if let Err(err) = run_build(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("install") => {
            if let Err(err) = run_install(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("prepare") => {
            if let Err(err) = run_prepare(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("add") => {
            if let Err(err) = run_add(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("run") => match run_script(&args[2..]) {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        },
        Some("x") => match run_x(&args[2..]) {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        },
        Some("dev") => {
            if let Err(err) = run_dev(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("registry") => {
            if let Err(err) = run_registry(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("inspect") => {
            if let Err(err) = run_inspect(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("compat") => {
            if let Err(err) = run_compat(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("node-compat") => {
            if let Err(err) = run_node_compat(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some("completeness") => {
            if let Err(err) = run_completeness(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        Some(input) if is_script_path(input) => match run_file(input, &args[2..]) {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        },
        Some(command) => {
            eprintln!("error: unknown command or script `{command}`\n\n{USAGE}");
            std::process::exit(1);
        }
        None => {
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}

fn is_script_path(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|value| value.to_str()),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
    )
}

fn exit_status_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

fn run_file(input: &str, args: &[String]) -> Result<i32, String> {
    run_file_in(input, args, None, None)
}

/// Compiles `input` and runs the resulting binary with `args`. When
/// `directory` is given the binary starts there -- a project script's own
/// working directory, matching `npm run`/`bun run` -- and `registry`, when
/// given, is that project's own `thaw_modules` (a project script's imports
/// must resolve against the project, not the invoking shell's cwd).
fn run_file_in(
    input: &str,
    args: &[String],
    directory: Option<&Path>,
    registry: Option<&Path>,
) -> Result<i32, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let output = std::env::temp_dir().join(format!("thaw-run-{}-{nonce}", std::process::id()));
    run_file_at(input, args, &output, directory, registry)
}

fn run_file_at(
    input: &str,
    args: &[String],
    output: &Path,
    directory: Option<&Path>,
    registry: Option<&Path>,
) -> Result<i32, String> {
    let build_args = run_build_args(input, output, registry);
    let result = run_build(&build_args).and_then(|_| {
        let mut command = Command::new(output);
        command.args(args);
        if let Some(directory) = directory {
            command.current_dir(directory);
        }
        command
            .status()
            .map(exit_status_code)
            .map_err(|error| format!("failed to run `{}`: {error}", output.display()))
    });
    let _ = std::fs::remove_file(output);
    result
}

/// The `thaw build` arguments for a `thaw run`/`thaw <file>` invocation: the
/// entry, its output, and -- for a project script -- that project's own
/// registry.
fn run_build_args(input: &str, output: &Path, registry: Option<&Path>) -> Vec<String> {
    let mut args = vec![input.to_string(), "-o".into(), output.display().to_string()];
    if let Some(registry) = registry {
        args.push("--registry".into());
        args.push(registry.display().to_string());
    }
    args
}

fn run_install(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err("usage: thaw install [directory]".into());
    }
    let directory = Path::new(args.first().map(String::as_str).unwrap_or("."));
    if !directory.join("package.json").is_file() {
        return Err(format!(
            "`{}` does not contain package.json",
            directory.display()
        ));
    }
    let status = npm_install_command(directory)
        .status()
        .map_err(|error| format!("failed to run npm install: {error}"))?;
    if !status.success() {
        return Err(format!("npm install failed with {status}"));
    }
    Ok(())
}

fn npm_install_command(directory: &Path) -> Command {
    let mut command = Command::new("npm");
    command.args(["install", "--prefix"]).arg(directory);
    command
}

fn run_add(args: &[String]) -> Result<(), String> {
    let prefix = args.iter().position(|arg| arg == "--prefix");
    let (packages, directory) = match prefix {
        Some(index) if index + 2 == args.len() => (&args[..index], Path::new(&args[index + 1])),
        Some(_) => return Err("usage: thaw add <package>... [--prefix <directory>]".into()),
        None => (args, Path::new(".")),
    };
    if packages.is_empty() {
        return Err("usage: thaw add <package>... [--prefix <directory>]".into());
    }
    if !directory.join("package.json").is_file() {
        return Err(format!(
            "`{}` does not contain package.json",
            directory.display()
        ));
    }
    let status = npm_add_command(packages, directory)
        .status()
        .map_err(|error| format!("failed to run npm install: {error}"))?;
    if !status.success() {
        return Err(format!("npm install failed with {status}"));
    }
    Ok(())
}

fn npm_add_command(packages: &[String], directory: &Path) -> Command {
    let mut command = Command::new("npm");
    command
        .args(["install", "--prefix"])
        .arg(directory)
        .args(packages);
    command
}

const RUN_USAGE: &str = "usage: thaw run <file.ts> [arguments...] | thaw run <package-script> [--prefix <directory>] [--] [arguments...]";

fn run_script(args: &[String]) -> Result<i32, String> {
    // `thaw run` with no arguments lists the project's scripts, like
    // `bun run`, rather than erroring -- but only when there is actually a
    // `package.json` to read.
    if args.is_empty() {
        if Path::new("package.json").is_file() {
            return list_scripts(Path::new("."));
        }
        return Err(RUN_USAGE.into());
    }
    let script = &args[0];
    if is_script_path(script) {
        return run_file(script, &args[1..]);
    }
    // `[--prefix <dir>] [--] [arguments...]` -- a `--prefix` anywhere before
    // the arguments picks the project directory; the first non-option token
    // ends it, and every token from there on is forwarded to the script
    // (matching `npm run <script> -- ...`/`bun run <script> ...`).
    let (directory, extra_args) = parse_script_tail(&args[1..])?;
    let directory = directory.as_path();
    if !directory.join("package.json").is_file() {
        return Err(format!(
            "`{}` does not contain package.json",
            directory.display()
        ));
    }
    // A script whose command is just a supported source file is compiled
    // and run by thaw -- the point of `thaw run` -- from the project
    // directory, with the script's own declared arguments plus any the
    // caller added.
    if let Some((file, mut script_args)) = package_script_file(directory, script) {
        script_args.extend(extra_args);
        let input = directory.join(&file);
        let input = input
            .to_str()
            .ok_or_else(|| format!("`{}` is not valid UTF-8", input.display()))?;
        let registry = directory.join("thaw_modules");
        return run_file_in(input, &script_args, Some(directory), Some(&registry));
    }
    // Anything wider (a shell pipeline, `tsx watch`, `vite build`, ...)
    // keeps running through npm, which is what actually provides that
    // shell and `node_modules/.bin` on PATH.
    let status = npm_run_command(script, directory, &extra_args)
        .status()
        .map_err(|error| format!("failed to run npm script `{script}`: {error}"))?;
    Ok(exit_status_code(status))
}

/// The project's `scripts`, `(name, command)` sorted by name.
fn package_script_listing(directory: &Path) -> Result<Vec<(String, String)>, String> {
    let manifest = std::fs::read_to_string(directory.join("package.json"))
        .map_err(|error| format!("failed to read package.json: {error}"))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|error| format!("invalid package.json: {error}"))?;
    let Some(scripts) = manifest
        .get("scripts")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(Vec::new());
    };
    let mut listing = scripts
        .iter()
        .map(|(name, command)| {
            (
                name.clone(),
                command.as_str().unwrap_or_default().to_string(),
            )
        })
        .collect::<Vec<_>>();
    listing.sort();
    Ok(listing)
}

fn list_scripts(directory: &Path) -> Result<i32, String> {
    let listing = package_script_listing(directory)?;
    if listing.is_empty() {
        return Err(format!(
            "`{}` has no scripts",
            directory.join("package.json").display()
        ));
    }
    for (name, command) in listing {
        println!("{name}: {command}");
    }
    Ok(0)
}

/// Whether `script`'s command is a single supported source file, and the
/// arguments to forward to it if so. Returns `None` for a shell command
/// (`vite build`, `tsx watch src/server.ts`, ...), which `run_script` then
/// hands to npm unchanged.
///
/// Deliberately only a *simple* command: every whitespace-separated token
/// must be plain (no quotes, escapes, redirection, substitution, ...). A
/// quoted argument like `"src/main.ts \"hello world\""` would otherwise be
/// split into `"hello` and `world"`, which is not what npm's shell would
/// pass -- so anything with shell syntax is left to `npm run`, which is
/// what actually interprets it.
fn package_script_file(directory: &Path, script: &str) -> Option<(String, Vec<String>)> {
    let manifest = std::fs::read_to_string(directory.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    let command = manifest.get("scripts")?.get(script)?.as_str()?;
    let mut tokens = command.split_whitespace();
    let file = tokens.next()?;
    if !is_script_path(file) || !command.split_whitespace().all(is_simple_shell_token) {
        return None;
    }
    Some((file.to_string(), tokens.map(str::to_string).collect()))
}

/// A token a naive `split_whitespace` is guaranteed to reproduce exactly:
/// no quotes, escapes, globs, redirection, or other shell metacharacters.
fn is_simple_shell_token(token: &str) -> bool {
    !token.is_empty()
        && token.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '_' | '-' | '.' | '/' | ':' | '@' | '=' | '+' | ','
                )
        })
}

/// Parses the `[--prefix <dir>] [--] [arguments...]` tail of `thaw run
/// <script>`, returning the project directory and the arguments to forward.
fn parse_script_tail(args: &[String]) -> Result<(PathBuf, Vec<String>), String> {
    let mut directory = PathBuf::from(".");
    let mut extra_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--prefix" => {
                index += 1;
                directory = PathBuf::from(args.get(index).ok_or(RUN_USAGE)?);
                index += 1;
            }
            "--" => {
                extra_args.extend(args[index + 1..].iter().cloned());
                break;
            }
            _ => {
                extra_args.extend(args[index..].iter().cloned());
                break;
            }
        }
    }
    Ok((directory, extra_args))
}

fn npm_run_command(script: &str, directory: &Path, arguments: &[String]) -> Command {
    let mut command = Command::new("npm");
    command.args(["run", script, "--prefix"]).arg(directory);
    if !arguments.is_empty() {
        command.arg("--").args(arguments);
    }
    command
}

include!("dev.rs");

const ARTIFACT_MARKER: &str = "THAW_ARTIFACT_V1:";
const ARTIFACT_SECTION: &[u8] = b".thaw.artifact";

#[derive(Clone, Copy)]
enum ElfEndian {
    Little,
    Big,
}

fn elf_endian(bytes: &[u8]) -> Result<ElfEndian, String> {
    if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" || bytes[4] != 2 {
        return Err("not an ELF64 executable".into());
    }
    match bytes[5] {
        1 => Ok(ElfEndian::Little),
        2 => Ok(ElfEndian::Big),
        _ => Err("ELF executable has unsupported byte order".into()),
    }
}

fn elf_range(bytes: &[u8], offset: u64, length: u64) -> Result<&[u8], String> {
    let start = usize::try_from(offset).map_err(|_| "ELF offset exceeds addressable memory")?;
    let length = usize::try_from(length).map_err(|_| "ELF length exceeds addressable memory")?;
    let end = start.checked_add(length).ok_or("ELF range overflows")?;
    bytes
        .get(start..end)
        .ok_or_else(|| "ELF range extends beyond file".into())
}

impl ElfEndian {
    fn u16(self, bytes: &[u8]) -> Result<u16, String> {
        let raw: [u8; 2] = bytes
            .get(..2)
            .ok_or("truncated ELF integer")?
            .try_into()
            .map_err(|_| "truncated ELF integer")?;
        Ok(match self {
            Self::Little => u16::from_le_bytes(raw),
            Self::Big => u16::from_be_bytes(raw),
        })
    }

    fn u32(self, bytes: &[u8]) -> Result<u32, String> {
        let raw: [u8; 4] = bytes
            .get(..4)
            .ok_or("truncated ELF integer")?
            .try_into()
            .map_err(|_| "truncated ELF integer")?;
        Ok(match self {
            Self::Little => u32::from_le_bytes(raw),
            Self::Big => u32::from_be_bytes(raw),
        })
    }

    fn u64(self, bytes: &[u8]) -> Result<u64, String> {
        let raw: [u8; 8] = bytes
            .get(..8)
            .ok_or("truncated ELF integer")?
            .try_into()
            .map_err(|_| "truncated ELF integer")?;
        Ok(match self {
            Self::Little => u64::from_le_bytes(raw),
            Self::Big => u64::from_be_bytes(raw),
        })
    }
}

fn elf_section<'a>(bytes: &'a [u8], name: &[u8]) -> Result<&'a [u8], String> {
    let endian = elf_endian(bytes)?;
    let header = &bytes[..64];
    let table_offset = endian.u64(&header[40..])?;
    let entry_size = u64::from(endian.u16(&header[58..])?);
    let count = u64::from(endian.u16(&header[60..])?);
    let names_index = u64::from(endian.u16(&header[62..])?);
    if count == 0 || names_index == 0xffff {
        return Err("ELF extended section numbering is unsupported".into());
    }
    if entry_size < 64 || names_index == 0 || names_index >= count {
        return Err("ELF section table is invalid".into());
    }
    let table_size = count
        .checked_mul(entry_size)
        .ok_or("ELF section table overflows")?;
    elf_range(bytes, table_offset, table_size)?;
    let entry = |index: u64| -> Result<&[u8], String> {
        let offset = index
            .checked_mul(entry_size)
            .and_then(|step| table_offset.checked_add(step))
            .ok_or("ELF section offset overflows")?;
        elf_range(bytes, offset, 64)
    };
    let names_header = entry(names_index)?;
    if endian.u32(&names_header[4..])? != 3 {
        return Err("ELF section names table has an invalid type".into());
    }
    let names = elf_range(
        bytes,
        endian.u64(&names_header[24..])?,
        endian.u64(&names_header[32..])?,
    )?;
    let mut found = None;
    for index in 1..count {
        let section = entry(index)?;
        let name_offset = usize::try_from(endian.u32(section)?)
            .map_err(|_| "ELF section name offset overflows")?;
        let suffix = names
            .get(name_offset..)
            .ok_or("ELF section name is out of range")?;
        let end = suffix
            .iter()
            .position(|byte| *byte == 0)
            .ok_or("ELF section name is not null terminated")?;
        if &suffix[..end] != name {
            continue;
        }
        if found.is_some() {
            return Err("executable contains duplicate Thaw artifact sections".into());
        }
        if endian.u32(&section[4..])? != 1 {
            return Err("Thaw artifact section is not PROGBITS".into());
        }
        found = Some(elf_range(
            bytes,
            endian.u64(&section[24..])?,
            endian.u64(&section[32..])?,
        )?);
    }
    found.ok_or_else(|| "executable does not contain a Thaw artifact section".into())
}

fn run_inspect(args: &[String]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("usage: thaw inspect <executable>".into());
    }
    let path = Path::new(&args[0]);
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let endian = elf_endian(&bytes).map_err(|error| format!("`{}`: {error}", path.display()))?;
    let architecture = match endian.u16(&bytes[18..])? {
        0x3e => "x86_64",
        0xb7 => "aarch64",
        _ => "unknown",
    };
    let linkage = if elf_has_program_interpreter(path)? {
        "dynamic-system"
    } else {
        "fully-static"
    };
    let manifest = artifact_manifest_from_bytes(&bytes)?;
    println!("file: {}", path.display());
    println!("format: ELF64");
    println!("architecture: {architecture}");
    println!("linkage: {linkage}");
    println!(
        "packages: {}",
        manifest["packages"]
            .as_array()
            .map(|packages| packages
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    );
    println!(
        "quickjs: {}",
        manifest["quickjs"].as_bool().unwrap_or(false)
    );
    if let Some(reasons) = manifest["quickjs_reasons"].as_array() {
        for reason in reasons {
            let kind = reason["kind"].as_str().unwrap_or("unknown");
            let package = reason["package"]
                .as_str()
                .map(|value| format!(" package={value}"))
                .unwrap_or_default();
            let operation = reason["function"]
                .as_str()
                .or_else(|| reason["operation"].as_str())
                .map(|value| format!(" operation={value}"))
                .unwrap_or_default();
            let detail = reason["detail"]
                .as_str()
                .map(|value| format!(" reason={value}"))
                .unwrap_or_default();
            let location = match (
                reason["source"].as_str(),
                reason["line"].as_u64(),
                reason["column"].as_u64(),
            ) {
                (Some(source), Some(line), Some(column)) if line > 0 => {
                    format!(" at {source}:{line}:{column}")
                }
                _ => String::new(),
            };
            println!("quickjs-reason: {kind}{package}{operation}{location}{detail}");
        }
    }
    println!("napi: {}", manifest["napi"].as_bool().unwrap_or(false));
    Ok(())
}

fn artifact_manifest_from_bytes(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let section = elf_section(bytes, ARTIFACT_SECTION)?;
    let payload = section
        .strip_prefix(ARTIFACT_MARKER.as_bytes())
        .ok_or("Thaw artifact metadata has an unsupported version")?;
    let json = payload
        .strip_suffix(&[0])
        .ok_or("Thaw artifact metadata is not null terminated")?;
    if json.is_empty() || json.contains(&0) {
        return Err("Thaw artifact metadata has invalid contents".into());
    }
    serde_json::from_slice(json).map_err(|error| format!("invalid Thaw artifact metadata: {error}"))
}

fn run_registry(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("add") => run_registry_add(&args[1..]),
        _ => Err("usage: thaw registry add <package>[@<version>] [--registry <dir>] [--from-node-modules <dir>]".to_string()),
    }
}

/// The "automatic" half of the registry story (docs/design/registry.md
/// section 6): fetches a real npm package and drops it into the local
/// registry directory in the layout `thaw build --use` expects, so a
/// package name is all a user needs -- no hand-copying `package.d.ts`/
/// `bundle.js` themselves.
fn run_registry_add(args: &[String]) -> Result<(), String> {
    let mut package: Option<String> = None;
    let mut registry_dir = PathBuf::from("thaw_modules");
    let mut installed_dir: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--registry" => {
                i += 1;
                let value = args.get(i).ok_or("--registry requires a path argument")?;
                registry_dir = PathBuf::from(value);
            }
            "--from-node-modules" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or("--from-node-modules requires a path argument")?;
                installed_dir = Some(PathBuf::from(value));
            }
            other => {
                if package.is_some() {
                    return Err(format!("unexpected extra argument `{other}`"));
                }
                package = Some(other.to_string());
            }
        }
        i += 1;
    }

    let package =
        package.ok_or("missing package name (usage: thaw registry add <package>[@<version>])")?;
    // `add` accepts an optional `@<version>` suffix (same syntax as `npm
    // install`) but the registry's own directory is always named after
    // the bare package name, independent of which version was requested
    // -- `thaw_registry::package_name` does the same split thaw-registry
    // itself uses internally, purely for this function's own display
    // purposes.
    let name = thaw_registry::package_name(&package);

    let added = if let Some(node_modules) = installed_dir {
        println!("adding installed `{name}`...");
        thaw_registry::add_installed(&registry_dir, &node_modules, name)?
    } else {
        println!("fetching `{package}`...");
        thaw_registry::add(&registry_dir, &package)?
    };
    let bundle_note = if added.bundled_file_count > 1 {
        format!(" (bundled {} files)", added.bundled_file_count)
    } else {
        String::new()
    };
    let deps_note = if added.dependency_versions.len() > 1 {
        let mut deps: Vec<String> = added
            .dependency_versions
            .iter()
            .filter(|(dep_name, _)| dep_name.as_str() != name)
            .map(|(dep_name, version)| format!("{dep_name}@{version}"))
            .collect();
        deps.sort();
        format!("\n  deps:  {}", deps.join(", "))
    } else {
        String::new()
    };
    let native_note = if let Some(native) = &added.native_addon {
        format!(
            "\n  native: {} ({}/{} {}, sha256 {})",
            native.source, native.platform, native.arch, native.libc, native.sha256
        )
    } else if let Some(diagnostic) = &added.native_diagnostic {
        format!("\n  native: skipped ({diagnostic}); using JavaScript fallback")
    } else {
        String::new()
    };
    println!(
        "added `{name}@{}` to `{}`\n  types: {}\n  main:  {}{bundle_note}{deps_note}{native_note}",
        added.resolved_version,
        registry_dir.join(name).display(),
        added.dts_relative_path,
        added.js_relative_path
    );
    Ok(())
}

fn build_option_value(args: &[String], index: usize) -> Result<Option<&str>, String> {
    let message = match args[index].as_str() {
        "-o" | "--output" => "-o requires a path argument",
        "--link" => "--link requires a path argument",
        "--bridge" => "--bridge requires a path argument",
        "--ffi-metadata" => "--ffi-metadata requires a path argument",
        "--registry" => "--registry requires a path argument",
        "--use" => "--use requires a package name argument",
        "--assets" => "--assets requires a directory argument",
        "--vite" => "--vite requires a directory argument",
        _ => return Ok(None),
    };
    args.get(index + 1)
        .map(|value| Some(value.as_str()))
        .ok_or_else(|| message.to_string())
}

fn run_build(args: &[String]) -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut extra_links: Vec<PathBuf> = Vec::new();
    let mut bridge_dts: Vec<PathBuf> = Vec::new();
    let mut ffi_metadata: Vec<PathBuf> = Vec::new();
    let mut registry_dir = PathBuf::from("thaw_modules");
    let mut use_packages: Vec<String> = Vec::new();
    let mut assets: Option<PathBuf> = None;
    let mut vite: Option<PathBuf> = None;
    let mut static_link = false;
    let mut embed_native_addons = true;
    let mut install_missing = true;
    let mut icu4c = false;
    let mut registry_was_explicit = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                output = Some(PathBuf::from(value));
            }
            "--link" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                extra_links.push(PathBuf::from(value));
            }
            "--bridge" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                bridge_dts.push(PathBuf::from(value));
            }
            "--ffi-metadata" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                ffi_metadata.push(PathBuf::from(value));
            }
            "--registry" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                registry_dir = PathBuf::from(value);
                registry_was_explicit = true;
            }
            "--use" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                use_packages.push(value.to_string());
            }
            "--assets" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                assets = Some(PathBuf::from(value));
            }
            "--vite" => {
                let value = build_option_value(args, i)?.unwrap();
                i += 1;
                vite = Some(PathBuf::from(value));
            }
            "--static" => static_link = true,
            "--external-native" => embed_native_addons = false,
            "--no-install" => install_missing = false,
            "--icu4c" => icu4c = true,
            other => {
                if input.is_some() {
                    return Err(format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }

    let project_directory = input
        .as_ref()
        .filter(|path| path.is_dir())
        .cloned()
        .or_else(|| input.is_none().then(|| PathBuf::from(".")));
    if let Some(directory) = &project_directory {
        let manifest_path = directory.join("package.json");
        let manifest = std::fs::read_to_string(&manifest_path)
            .map_err(|_| format!("`{}` does not contain package.json", directory.display()))?;
        let (configured_input, configured_vite, configured_output) =
            project_build_defaults(&manifest)?;
        input = project_input_path(directory, configured_input);
        if output.is_none() {
            output = configured_output
                .map(|path| directory.join(path))
                .or_else(|| Some(directory.join("app")));
        }
        if assets.is_none() && vite.is_none() {
            vite = configured_vite.map(|path| directory.join(path));
        }
        if !registry_was_explicit {
            registry_dir = directory.join("thaw_modules");
        }
    }
    let generated_input = if input.is_none() && vite.is_some() {
        Some(write_static_asset_server_entry()?)
    } else {
        None
    };
    let input = input
        .or_else(|| generated_input.clone())
        .ok_or("missing input file; set `thaw.entry` in package.json or add server.ts")?;
    let output = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default();
        PathBuf::from(stem)
    });
    if assets.is_some() && vite.is_some() {
        return Err("--assets and --vite cannot be used together".into());
    }
    if let Some(directory) = vite {
        assets = Some(build_vite_project(&directory)?);
    }

    let result = build_with_native_mode(
        &input,
        &output,
        &extra_links,
        &bridge_dts,
        &ffi_metadata,
        &registry_dir,
        &use_packages,
        static_link,
        assets.as_deref(),
        embed_native_addons,
        install_missing,
        icu4c,
    );
    if let Some(path) = generated_input {
        let _ = std::fs::remove_file(path);
    }
    result
}

#[allow(clippy::type_complexity)]
fn project_build_defaults(
    manifest: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>, Option<PathBuf>), String> {
    let manifest: serde_json::Value =
        serde_json::from_str(manifest).map_err(|error| format!("invalid package.json: {error}"))?;
    let thaw = manifest.get("thaw").and_then(serde_json::Value::as_object);
    let path = |name| {
        thaw.and_then(|config| config.get(name))
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
    };
    let entry = path("entry")
        .or_else(|| {
            ["source", "module", "main"]
                .into_iter()
                .find_map(|name| manifest.get(name)?.as_str().map(PathBuf::from))
        })
        .or_else(|| {
            ["start", "dev"].into_iter().find_map(|name| {
                manifest["scripts"][name]
                    .as_str()?
                    .split_whitespace()
                    .find(|word| {
                        matches!(
                            Path::new(word)
                                .extension()
                                .and_then(|extension| extension.to_str()),
                            Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
                        )
                    })
                    .map(PathBuf::from)
            })
        });
    let vite = path("vite").or_else(|| {
        manifest["scripts"]["build"]
            .as_str()
            .is_some_and(|script| script.split_whitespace().any(|word| word == "vite"))
            .then(|| PathBuf::from("."))
    });
    Ok((entry, vite, path("output")))
}

fn project_input_path(directory: &Path, configured: Option<PathBuf>) -> Option<PathBuf> {
    configured
        .filter(|path| directory.join(path).is_file())
        .map(|path| directory.join(path))
        .or_else(|| {
            ["server.ts", "src/server.ts", "index.ts", "src/index.ts"]
                .into_iter()
                .map(|path| directory.join(path))
                .find(|path| path.is_file())
        })
}

fn write_static_asset_server_entry() -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("thaw-vite-entry-{}.ts", std::process::id()));
    std::fs::write(
        &path,
        r#"import { createServer } from "node:http";
function main(): void {
  const port: number = Number(process.env.PORT) || 3000;
  const server = createServer((request: { method: string; url: string; statusCode: number; body: () => string }, response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (body: string) => boolean; write: (body: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }): boolean => {
    if (request.method === "GET" && thawServeAsset(response, request.url)) { return true; }
    response.statusCode = 404;
    return response.end("Not Found");
  });
  server.listen(port, (): void => { console.log(`http://127.0.0.1:${port}`); });
}
"#,
    )
    .map_err(|error| format!("failed to create Vite server entry: {error}"))?;
    Ok(path)
}

fn build_vite_project(directory: &Path) -> Result<PathBuf, String> {
    if !directory.join("package.json").is_file() {
        return Err(format!(
            "--vite expects a project containing package.json, got `{}`",
            directory.display()
        ));
    }
    let output = Command::new("npm")
        .args(["run", "build", "--prefix"])
        .arg(directory)
        .output()
        .map_err(|error| format!("failed to run the Vite build: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Vite build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let dist = directory.join("dist");
    if !dist.is_dir() {
        return Err(format!(
            "Vite build did not create `{}`; use --assets for a custom outDir",
            dist.display()
        ));
    }
    Ok(dist)
}

/// Reads each `.d.ts` in `bridge_dts`, classifies its functions (thaw-bridge,
/// docs/design/bridge.md sections 3-4), and generates the corresponding
/// callable surface (section 6: an ambient `declare function` per fast-path
/// function; section 7: a `callDynamic` wrapper per fallback function) --
/// concatenated ahead of the user's own source. Thaw has no real module/
/// import system yet, so "prepend the generated text" is the whole
/// integration; there's nothing to separately compile or link at this step
/// (fast-path symbols still need `--link`, fallback functions still need
/// `loadScript` called with the package's actual JS source, per
/// `generate_shim`'s doc comment).
fn generate_bridge_shims(bridge_dts: &[PathBuf]) -> Result<String, String> {
    let mut shim = String::new();
    for path in bridge_dts {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read `{}`: {e}", path.display()))?;
        let functions = thaw_bridge::parse_dts_named(
            &source,
            thaw_parser::common::FileName::Real(path.clone()),
        )
        .map_err(|e| format!("failed to parse `{}`: {e}", path.display()))?;
        // `true`: the manual `--bridge` path trusts the classification
        // as-is, since the user is already responsible for supplying a
        // matching `--link`ed library themselves for any FastPath
        // function it declares (unlike `--use`, see
        // `generate_registry_shims`, where thaw-registry knows whether a
        // `native.a` actually exists).
        shim.push_str(&thaw_bridge::generate_shim(
            &functions,
            true,
            &[],
            &std::collections::HashSet::new(),
        ));
    }
    Ok(shim)
}

include!("registry_integration/shims.rs");
include!("registry_integration/jit_validation.rs");
include!("registry_integration/dynamic_declarations.rs");
include!("registry_integration/shim_support.rs");
include!("registry_integration/shim_generation.rs");
include!("registry_integration/class_methods.rs");
include!("registry_integration/calls.rs");

include!("ffi_metadata.rs");

include!("build.rs");

include!("x.rs");

#[cfg(test)]
mod tests;
