fn prepare_async_modules(modules: &mut [BundledModule]) -> Result<(), String> {
    use std::collections::BTreeSet;

    for module in modules.iter_mut() {
        module.async_module = module.has_top_level_await;
    }
    loop {
        let async_keys: BTreeSet<_> = modules
            .iter()
            .filter(|module| module.async_module)
            .map(|module| module.key.clone())
            .collect();
        let mut changed = false;
        for module in modules.iter_mut().filter(|module| module.has_esm) {
            if module.async_module {
                continue;
            }
            if module.static_esm_specs.iter().any(|specifier| {
                module
                    .requires
                    .iter()
                    .any(|(source, target)| source == specifier && async_keys.contains(target))
            }) {
                module.async_module = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    fn visit(
        key: &str,
        modules: &[BundledModule],
        visiting: &mut Vec<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        if let Some(index) = visiting.iter().position(|item| item == key) {
            let mut cycle = visiting[index..].to_vec();
            cycle.push(key.to_string());
            return Err(format!(
                "top-level await module cycle is not supported: {}",
                cycle.join(" -> ")
            ));
        }
        if !visited.insert(key.to_string()) {
            return Ok(());
        }
        let Some(module) = modules.iter().find(|module| module.key == key) else {
            return Ok(());
        };
        visiting.push(key.to_string());
        for (specifier, target) in &module.requires {
            if module.static_esm_specs.contains(specifier)
                && modules
                    .iter()
                    .any(|candidate| candidate.key == *target && candidate.async_module)
            {
                visit(target, modules, visiting, visited)?;
            }
        }
        visiting.pop();
        Ok(())
    }

    let mut visited = BTreeSet::new();
    for module in modules.iter().filter(|module| module.async_module) {
        visit(&module.key, modules, &mut Vec::new(), &mut visited)?;
    }

    for module in modules.iter_mut() {
        let source = rewrite_esm_to_commonjs_mode(&module.source, module.async_module)
            .unwrap_or_else(|| module.source.clone());
        module.source = source;
    }
    Ok(())
}
