use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::process::Child;
use std::time::Duration;

fn run_dev(args: &[String]) -> Result<(), String> {
    let (mut input, configured_vite) = dev_project_paths(args, Path::new("."))?;
    let project_root = match dev_input_arg(args)? {
        Some(path) if Path::new(path).is_dir() => Some(PathBuf::from(path)),
        None => Some(PathBuf::from(".")),
        _ => None,
    };
    let output = build_output(args).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("thaw-dev-{}", std::process::id()))
    });
    let build_args = dev_build_args(args, configured_vite, &output);
    let mut roots = dev_watch_roots(&input, project_root.as_deref(), &[]);
    let mut vite_directory = dev_vite_directory(&build_args);
    let mut assets_directory = dev_assets_directory(&build_args);
    let mut fingerprint = source_fingerprint(&roots)?;
    let mut vite_fingerprint = vite_directory
        .as_ref()
        .map(|directory| vite_source_fingerprint(directory))
        .transpose()?;
    let mut assets_fingerprint = assets_directory
        .as_ref()
        .map(|directory| explicit_assets_fingerprint(directory))
        .transpose()?;
    let mut child = rebuild_and_start(&build_args, &output, None);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let next = source_fingerprint(&roots)?;
        let current_assets_fingerprint = assets_directory
            .as_ref()
            .map(|directory| explicit_assets_fingerprint(directory))
            .transpose()?;
        let (next_input, configured_vite) = match dev_project_paths(args, Path::new(".")) {
            Ok(paths) => paths,
            Err(error) => {
                eprintln!("error: {error}");
                roots = dev_watch_roots(&input, project_root.as_deref(), &roots);
                fingerprint = source_fingerprint(&roots)?;
                continue;
            }
        };
        let next_build_args = dev_build_args(args, configured_vite, &output);
        let next_vite_directory = dev_vite_directory(&next_build_args);
        let next_assets_directory = dev_assets_directory(&next_build_args);
        let next_vite_fingerprint = next_vite_directory
            .as_ref()
            .map(|directory| vite_source_fingerprint(directory))
            .transpose()?;
        let next_assets_fingerprint = if next_assets_directory == assets_directory {
            current_assets_fingerprint
        } else {
            next_assets_directory.as_ref()
                .map(|directory| explicit_assets_fingerprint(directory))
                .transpose()?
        };
        if next == fingerprint && next_vite_fingerprint == vite_fingerprint
            && next_assets_fingerprint == assets_fingerprint {
            continue;
        }
        let rebuild_args = if vite_directory == next_vite_directory
            && vite_fingerprint == next_vite_fingerprint {
            next_vite_directory
                .as_ref()
                .filter(|directory| directory.join("dist").is_dir())
                .map(|directory| reuse_vite_assets(&next_build_args, directory))
                .unwrap_or_else(|| next_build_args.clone())
        } else {
            next_build_args.clone()
        };
        input = next_input;
        vite_directory = next_vite_directory;
        assets_directory = next_assets_directory;
        vite_fingerprint = next_vite_fingerprint;
        assets_fingerprint = next_assets_fingerprint;
        eprintln!("change detected; rebuilding...");
        child = rebuild_and_start(&rebuild_args, &output, child);
        roots = dev_watch_roots(&input, project_root.as_deref(), &roots);
        fingerprint = source_fingerprint(&roots)?;
    }
}

fn dev_build_args(args: &[String], configured_vite: Option<PathBuf>, output: &Path) -> Vec<String> {
    let mut build_args = args.to_vec();
    if let Some(directory) = configured_vite.filter(|_| {
        !build_args.iter().any(|argument| argument == "--vite" || argument == "--assets")
    }) {
        build_args.extend(["--vite".into(), directory.display().to_string()]);
    }
    if !build_args.iter().any(|arg| arg == "--static" || arg == "--external-native") {
        build_args.push("--external-native".into());
    }
    if build_output(&build_args).is_none() {
        build_args.extend(["-o".into(), output.display().to_string()]);
    }
    build_args
}

fn dev_vite_directory(args: &[String]) -> Option<PathBuf> {
    args.iter().position(|arg| arg == "--vite")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from)
}

fn dev_assets_directory(args: &[String]) -> Option<PathBuf> {
    args.iter().position(|arg| arg == "--assets")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from)
}

fn dev_watch_roots(input: &Path, project_root: Option<&Path>, previous: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![input.parent().unwrap_or(Path::new(".")).to_path_buf()];
    if let Some(project_root) = project_root {
        let manifest_path = project_root.join("package.json");
        if let Ok(manifest) = std::fs::read_to_string(&manifest_path) {
            if let Ok((Some(entry), _, _)) = project_build_defaults(&manifest) {
                roots.push(project_root.join(entry));
            }
        }
        roots.push(manifest_path);
    }
    match module_graph::source_paths(input) {
        Ok((paths, complete)) => {
            roots.extend(paths);
            if !complete {
                roots.extend(previous.iter().filter(|path| !path.is_dir()).cloned());
            }
        }
        // Keep the last resolved graph while an edited import is invalid or
        // temporarily missing; restoring a dependency must wake the watcher.
        Err(_) => roots.extend(previous.iter().filter(|path| !path.is_dir()).cloned()),
    }
    roots.sort();
    roots.dedup();
    roots
}

fn dev_project_paths(args: &[String], directory: &Path) -> Result<(PathBuf, Option<PathBuf>), String> {
    if let Some(input) = dev_input_arg(args)? {
        let project = Path::new(input);
        if !project.is_dir() {
            return Ok((PathBuf::from(input), None));
        }
        return dev_project_paths(&[], project).map(|(entry, vite)| {
            (
                project.join(entry),
                vite.map(|directory| project.join(directory)),
            )
        });
    }
    let manifest = std::fs::read_to_string(directory.join("package.json")).map_err(|_| {
        "missing input file and package.json has no project configuration".to_string()
    })?;
    let configured = project_build_defaults(&manifest)?;
    let input = project_input_path(directory, configured.0)
        .and_then(|path| path.strip_prefix(directory).ok().map(PathBuf::from))
        .or_else(|| configured.1.as_ref().map(|_| PathBuf::from("package.json")))
        .ok_or("missing input file; set `thaw.entry` in package.json or add server.ts")?;
    Ok((input, configured.1))
}

fn dev_input_arg(args: &[String]) -> Result<Option<&str>, String> {
    let mut index = 0;
    while index < args.len() {
        if build_option_value(args, index)?.is_some() {
            index += 2;
            continue;
        }
        match args[index].as_str() {
            "--static" | "--external-native" | "--no-install" | "--icu4c" => index += 1,
            input => return Ok(Some(input)),
        }
    }
    Ok(None)
}

fn reuse_vite_assets(args: &[String], directory: &Path) -> Vec<String> {
    let mut args = args.to_vec();
    if let Some(index) = args.iter().position(|arg| arg == "--vite") {
        args[index] = "--assets".into();
        args[index + 1] = directory.join("dist").display().to_string();
    }
    args
}

fn build_output(args: &[String]) -> Option<PathBuf> {
    args.windows(2)
        .find(|pair| pair[0] == "-o" || pair[0] == "--output")
        .map(|pair| PathBuf::from(&pair[1]))
}

fn rebuild_and_start(args: &[String], output: &Path, child: Option<Child>) -> Option<Child> {
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    match run_build(args) {
        Ok(()) => match dev_executable_path(output)
            .and_then(|program| Command::new(program).spawn().map_err(|error| error.to_string())) {
            Ok(child) => Some(child),
            Err(error) => {
                eprintln!("error: failed to start `{}`: {error}", output.display());
                None
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            None
        }
    }
}

fn dev_executable_path(output: &Path) -> Result<PathBuf, String> {
    std::env::current_dir()
        .map(|directory| directory.join(output))
        .map_err(|error| format!("failed to resolve `{}`: {error}", output.display()))
}

fn source_fingerprint(roots: &[PathBuf]) -> Result<u64, String> {
    fingerprint_paths(roots, false, false)
}

fn vite_source_fingerprint(root: &Path) -> Result<u64, String> {
    fingerprint_paths(&[root.to_path_buf()], true, false)
}

fn explicit_assets_fingerprint(root: &Path) -> Result<u64, String> {
    fingerprint_paths(&[root.to_path_buf()], true, true)
}

fn fingerprint_paths(roots: &[PathBuf], include_assets: bool, include_all_directories: bool) -> Result<u64, String> {
    fn visit(path: &Path, hasher: &mut DefaultHasher, include_assets: bool, include_all_directories: bool, is_root: bool) -> Result<(), String> {
        // The asset generator uses DirEntry::file_type and omits symlinked
        // children. Match that ownership boundary, including directory loops.
        if include_all_directories && !is_root
            && path.symlink_metadata().is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Ok(());
        }
        if path.is_dir() {
            if !is_root && !include_all_directories && matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("node_modules" | "dist" | "thaw_modules" | ".git")
            ) {
                return Ok(());
            }
            let mut entries = std::fs::read_dir(path)
                .map_err(|error| format!("failed to watch `{}`: {error}", path.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("failed to watch `{}`: {error}", path.display()))?;
            entries.sort_by_key(|entry| entry.path());
            for entry in entries {
                visit(&entry.path(), hasher, include_assets, include_all_directories, false)?;
            }
        } else if include_assets || matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "css" | "html" | "json")
        ) {
            path.hash(hasher);
            match std::fs::metadata(path) {
                Ok(metadata) => {
                    true.hash(hasher);
                    metadata.len().hash(hasher);
                    metadata.modified().ok().hash(hasher);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    false.hash(hasher);
                }
                Err(error) => return Err(format!("failed to watch `{}`: {error}", path.display())),
            }
        }
        Ok(())
    }

    let mut hasher = DefaultHasher::new();
    for root in roots {
        visit(root, &mut hasher, include_assets, include_all_directories, true)?;
    }
    Ok(hasher.finish())
}
