use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::process::Child;
use std::time::Duration;

fn run_dev(args: &[String]) -> Result<(), String> {
    let (input, configured_vite) = dev_project_paths(args, Path::new("."))?;
    let mut build_args = args.to_vec();
    if let Some(directory) = configured_vite.filter(|_| {
        !build_args
            .iter()
            .any(|argument| argument == "--vite" || argument == "--assets")
    }) {
        build_args.extend(["--vite".into(), directory.display().to_string()]);
    }
    if !build_args.iter().any(|arg| arg == "--static" || arg == "--external-native") {
        build_args.push("--external-native".into());
    }
    let output = build_output(&build_args).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("thaw-dev-{}", std::process::id()))
    });
    if build_output(&build_args).is_none() {
        build_args.extend(["-o".into(), output.display().to_string()]);
    }
    let mut roots = vec![input.parent().unwrap_or(Path::new(".")).to_path_buf()];
    let vite_directory = build_args
        .iter()
        .position(|arg| arg == "--vite")
        .and_then(|index| build_args.get(index + 1))
        .map(PathBuf::from);
    if let Some(directory) = &vite_directory {
        roots.push(directory.clone());
    }
    roots.sort();
    roots.dedup();

    let mut fingerprint = source_fingerprint(&roots)?;
    let mut vite_fingerprint = vite_directory
        .as_ref()
        .map(|directory| source_fingerprint(std::slice::from_ref(directory)))
        .transpose()?;
    let mut child = rebuild_and_start(&build_args, &output, None);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let next = source_fingerprint(&roots)?;
        if next == fingerprint {
            continue;
        }
        fingerprint = next;
        let next_vite_fingerprint = vite_directory
            .as_ref()
            .map(|directory| source_fingerprint(std::slice::from_ref(directory)))
            .transpose()?;
        let rebuild_args = if vite_fingerprint == next_vite_fingerprint {
            vite_directory
                .as_ref()
                .filter(|directory| directory.join("dist").is_dir())
                .map(|directory| reuse_vite_assets(&build_args, directory))
                .unwrap_or_else(|| build_args.clone())
        } else {
            build_args.clone()
        };
        vite_fingerprint = next_vite_fingerprint;
        eprintln!("change detected; rebuilding...");
        child = rebuild_and_start(&rebuild_args, &output, child);
    }
}

fn dev_project_paths(args: &[String], directory: &Path) -> Result<(PathBuf, Option<PathBuf>), String> {
    if let Some(input) = args.first().filter(|argument| !argument.starts_with('-')) {
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
        Ok(()) => match Command::new(output).spawn() {
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

fn source_fingerprint(roots: &[PathBuf]) -> Result<u64, String> {
    fn visit(path: &Path, hasher: &mut DefaultHasher) -> Result<(), String> {
        if path.is_dir() {
            if matches!(
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
                visit(&entry.path(), hasher)?;
            }
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ts" | "tsx" | "js" | "jsx" | "css" | "html" | "json")
        ) {
            path.hash(hasher);
            let metadata = std::fs::metadata(path)
                .map_err(|error| format!("failed to watch `{}`: {error}", path.display()))?;
            metadata.len().hash(hasher);
            metadata.modified().ok().hash(hasher);
        }
        Ok(())
    }

    let mut hasher = DefaultHasher::new();
    for root in roots {
        visit(root, &mut hasher)?;
    }
    Ok(hasher.finish())
}
