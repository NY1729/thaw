use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use swc_ecma_visit::{VisitMut, VisitMutWith};
use thaw_parser::ast::{
    Callee, Decl, Expr, ImportSpecifier, Module, ModuleDecl, ModuleExportName, ModuleItem,
    TsEntityName, TsInterfaceDecl, TsTypeRef,
};

#[derive(Debug)]
struct LoadedModule {
    path: PathBuf,
    source: String,
    module: Module,
    dependencies: HashMap<String, usize>,
}

fn export_name(name: &ModuleExportName) -> Result<String, String> {
    match name {
        ModuleExportName::Ident(ident) => Ok(ident.sym.to_string()),
        ModuleExportName::Str(value) => value
            .value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "module export names must be valid UTF-8".to_string()),
    }
}

fn resolve_relative(from: &Path, specifier: &str) -> Result<PathBuf, String> {
    if !specifier.starts_with('.') {
        return Err(format!(
            "user modules only support relative imports, not `{specifier}`; use `--use` for registry packages"
        ));
    }
    let base = from.parent().unwrap_or_else(|| Path::new("."));
    let raw = base.join(specifier);
    let candidates = if raw.extension().is_some() {
        vec![raw]
    } else {
        vec![raw.with_extension("ts"), raw.join("index.ts")]
    };
    for candidate in &candidates {
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|error| format!("failed to resolve `{}`: {error}", candidate.display()));
        }
    }
    Err(format!(
        "cannot resolve `{specifier}` from `{}` (tried {})",
        from.display(),
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn dependency_specifiers(module: &Module) -> Result<Vec<String>, String> {
    let mut specs = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(decl) = item else {
            continue;
        };
        let source = match decl {
            ModuleDecl::Import(import) => Some(&import.src),
            ModuleDecl::ExportNamed(export) => export.src.as_ref(),
            ModuleDecl::ExportAll(export) => Some(&export.src),
            _ => None,
        };
        if let Some(source) = source {
            let spec = source
                .value
                .as_str()
                .ok_or_else(|| "module specifiers must be valid UTF-8".to_string())?;
            if !specs.iter().any(|existing| existing == spec) {
                specs.push(spec.to_string());
            }
        }
    }
    Ok(specs)
}

fn load_module(
    path: PathBuf,
    source_override: Option<&str>,
    modules: &mut Vec<LoadedModule>,
    loaded: &mut HashMap<PathBuf, usize>,
    visiting: &mut Vec<PathBuf>,
) -> Result<usize, String> {
    if let Some(index) = loaded.get(&path) {
        return Ok(*index);
    }
    if let Some(start) = visiting.iter().position(|candidate| candidate == &path) {
        let mut cycle = visiting[start..]
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        cycle.push(path.display().to_string());
        return Err(format!("cyclic user-module import: {}", cycle.join(" -> ")));
    }
    visiting.push(path.clone());
    let source = match source_override {
        Some(source) => source.to_string(),
        None => std::fs::read_to_string(&path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?,
    };
    let module = thaw_parser::parse_typescript(&source)
        .map_err(|error| format!("failed to parse `{}`: {error}", path.display()))?;
    let mut dependencies = HashMap::new();
    for specifier in dependency_specifiers(&module)? {
        if !specifier.starts_with('.') {
            continue;
        }
        let dependency_path = resolve_relative(&path, &specifier)
            .map_err(|error| format!("{}: {error}", source_location(&path, &source, &specifier)))?;
        let dependency = load_module(dependency_path, None, modules, loaded, visiting)?;
        dependencies.insert(specifier, dependency);
    }
    visiting.pop();
    let index = modules.len();
    modules.push(LoadedModule {
        path: path.clone(),
        source,
        module,
        dependencies,
    });
    loaded.insert(path, index);
    Ok(index)
}

fn module_location(module: &LoadedModule, specifier: &str) -> String {
    source_location(&module.path, &module.source, specifier)
}

fn source_location(path: &Path, source: &str, specifier: &str) -> String {
    let quoted = [format!("\"{specifier}\""), format!("'{specifier}'")];
    let offset = quoted
        .iter()
        .find_map(|needle| source.find(needle))
        .unwrap_or(0);
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map(|(_, tail)| tail.chars().count() + 1)
        .unwrap_or_else(|| before.chars().count() + 1);
    format!("{}:{line}:{column}", path.display())
}

struct RenameReferences<'a> {
    names: &'a HashMap<String, String>,
    namespaces: &'a HashMap<String, HashMap<String, String>>,
}

impl RenameReferences<'_> {
    fn rename_ident(&self, ident: &mut thaw_parser::ast::Ident) {
        if let Some(replacement) = self.names.get(ident.sym.as_ref()) {
            ident.sym = replacement.clone().into();
        }
    }
}

impl VisitMut for RenameReferences<'_> {
    fn visit_mut_call_expr(&mut self, call: &mut thaw_parser::ast::CallExpr) {
        call.visit_mut_children_with(self);
        if let Callee::Expr(callee) = &mut call.callee {
            match &mut **callee {
                Expr::Ident(ident) => self.rename_ident(ident),
                Expr::Member(member) => {
                    if let (Expr::Ident(namespace), thaw_parser::ast::MemberProp::Ident(property)) =
                        (&*member.obj, &member.prop)
                    {
                        if let Some(target) = self
                            .namespaces
                            .get(namespace.sym.as_ref())
                            .and_then(|exports| exports.get(property.sym.as_ref()))
                        {
                            **callee = Expr::Ident(thaw_parser::ast::Ident::new_no_ctxt(
                                target.clone().into(),
                                member.span,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn visit_mut_ts_type_ref(&mut self, reference: &mut TsTypeRef) {
        reference.visit_mut_children_with(self);
        if let TsEntityName::Ident(ident) = &mut reference.type_name {
            self.rename_ident(ident);
        }
    }

    fn visit_mut_ts_interface_decl(&mut self, interface: &mut TsInterfaceDecl) {
        interface.visit_mut_children_with(self);
        self.rename_ident(&mut interface.id);
        for base in &mut interface.extends {
            if let Expr::Ident(ident) = &mut *base.expr {
                self.rename_ident(ident);
            }
        }
    }

    fn visit_mut_fn_decl(&mut self, function: &mut thaw_parser::ast::FnDecl) {
        function.function.visit_mut_with(self);
        self.rename_ident(&mut function.ident);
    }
}

fn declared_names(module: &Module) -> Vec<String> {
    module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(decl)))
                if decl.function.body.is_some() =>
            {
                Some(decl.ident.sym.to_string())
            }
            ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::TsInterface(decl))) => {
                Some(decl.id.sym.to_string())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
                Decl::Fn(decl) if decl.function.body.is_some() => Some(decl.ident.sym.to_string()),
                Decl::TsInterface(decl) => Some(decl.id.sym.to_string()),
                _ => None,
            },
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
                thaw_parser::ast::DefaultDecl::Fn(function) => {
                    function.ident.as_ref().map(|ident| ident.sym.to_string())
                }
                thaw_parser::ast::DefaultDecl::TsInterfaceDecl(interface) => {
                    Some(interface.id.sym.to_string())
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

pub fn external_specifiers(
    entry: &Path,
    entry_source: &str,
) -> Result<Vec<(String, String)>, String> {
    let entry = entry
        .canonicalize()
        .map_err(|error| format!("failed to resolve `{}`: {error}", entry.display()))?;
    let mut modules = Vec::new();
    load_module(
        entry,
        Some(entry_source),
        &mut modules,
        &mut HashMap::new(),
        &mut Vec::new(),
    )?;
    let mut result = Vec::new();
    for module in modules {
        for specifier in dependency_specifiers(&module.module)? {
            if !specifier.starts_with('.')
                && !result.iter().any(|(existing, _)| existing == &specifier)
            {
                result.push((specifier.clone(), module_location(&module, &specifier)));
            }
        }
    }
    Ok(result)
}

pub fn bundle(
    entry: &Path,
    entry_source: &str,
    external_exports: &HashMap<String, HashMap<String, String>>,
) -> Result<Module, String> {
    let entry = entry
        .canonicalize()
        .map_err(|error| format!("failed to resolve `{}`: {error}", entry.display()))?;
    let mut modules = Vec::new();
    let mut loaded = HashMap::new();
    let entry_index = load_module(
        entry,
        Some(entry_source),
        &mut modules,
        &mut loaded,
        &mut Vec::new(),
    )?;

    let mut exports: Vec<HashMap<String, String>> = vec![HashMap::new(); modules.len()];
    let mut namespace_exports: Vec<HashMap<String, HashMap<String, String>>> =
        vec![HashMap::new(); modules.len()];
    let mut bundled_items = Vec::new();
    for index in 0..modules.len() {
        let is_entry = index == entry_index;
        let mut names = HashMap::new();
        let mut namespaces = HashMap::new();
        for name in declared_names(&modules[index].module) {
            let replacement = if is_entry && matches!(name.as_str(), "main" | "handler") {
                name.clone()
            } else {
                // Do not contain HIR's reserved `__thaw_` specialization
                // delimiter: specialized names split on that exact marker.
                format!("__thawmod{index}_{name}")
            };
            names.insert(name, replacement);
        }

        for item in &modules[index].module.body {
            if let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item {
                let specifier = import.src.value.as_str().ok_or_else(|| {
                    format!("invalid import in `{}`", modules[index].path.display())
                })?;
                let dependency_exports = modules[index]
                    .dependencies
                    .get(specifier)
                    .map(|dependency| &exports[*dependency])
                    .or_else(|| external_exports.get(specifier))
                    .ok_or_else(|| {
                        format!(
                            "{}: unresolved module `{specifier}`",
                            module_location(&modules[index], specifier)
                        )
                    })?;
                for imported in &import.specifiers {
                    let (local, requested) = match imported {
                        ImportSpecifier::Named(named) => (
                            named.local.sym.to_string(),
                            named
                                .imported
                                .as_ref()
                                .map(export_name)
                                .transpose()?
                                .unwrap_or_else(|| named.local.sym.to_string()),
                        ),
                        ImportSpecifier::Default(default) => {
                            (default.local.sym.to_string(), "default".to_string())
                        }
                        ImportSpecifier::Namespace(namespace) => {
                            namespaces.insert(
                                namespace.local.sym.to_string(),
                                dependency_exports.clone(),
                            );
                            continue;
                        }
                    };
                    if let Some(namespace) = modules[index]
                        .dependencies
                        .get(specifier)
                        .and_then(|dependency| namespace_exports[*dependency].get(&requested))
                    {
                        namespaces.insert(local, namespace.clone());
                        continue;
                    }
                    let target = dependency_exports.get(&requested).ok_or_else(|| {
                        format!(
                            "{}: `{specifier}` has no export named `{requested}`",
                            module_location(&modules[index], specifier)
                        )
                    })?;
                    names.insert(local, target.clone());
                }
            }
        }

        let mut public = HashMap::new();
        let mut public_namespaces = HashMap::new();
        let mut items = Vec::new();
        for item in modules[index].module.body.clone() {
            match item {
                ModuleItem::Stmt(mut statement) => {
                    statement.visit_mut_with(&mut RenameReferences {
                        names: &names,
                        namespaces: &namespaces,
                    });
                    items.push(ModuleItem::Stmt(statement));
                }
                ModuleItem::ModuleDecl(ModuleDecl::Import(_)) => {}
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(mut export)) => {
                    let original = match &export.decl {
                        Decl::Fn(decl) => Some(decl.ident.sym.to_string()),
                        Decl::TsInterface(decl) => Some(decl.id.sym.to_string()),
                        _ => None,
                    };
                    export.decl.visit_mut_with(&mut RenameReferences {
                        names: &names,
                        namespaces: &namespaces,
                    });
                    if let Some(original) = original {
                        public.insert(original.clone(), names[&original].clone());
                    }
                    items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(export.decl)));
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => {
                    let source_exports = if let Some(source) = &export.src {
                        let specifier = source.value.as_str().ok_or("invalid re-export")?;
                        Some(
                            modules[index]
                                .dependencies
                                .get(specifier)
                                .map(|dependency| &exports[*dependency])
                                .or_else(|| external_exports.get(specifier))
                                .ok_or_else(|| format!("unresolved re-export `{specifier}`"))?,
                        )
                    } else {
                        None
                    };
                    for specifier in export.specifiers {
                        match specifier {
                            thaw_parser::ast::ExportSpecifier::Named(named) => {
                                let original = export_name(&named.orig)?;
                                let exported = named
                                    .exported
                                    .as_ref()
                                    .map(export_name)
                                    .transpose()?
                                    .unwrap_or_else(|| original.clone());
                                if source_exports.is_none() {
                                    if let Some(namespace) = namespaces.get(&original) {
                                        public_namespaces.insert(exported, namespace.clone());
                                        continue;
                                    }
                                }
                                let target = source_exports
                                    .and_then(|source| source.get(&original))
                                    .or_else(|| names.get(&original))
                                    .ok_or_else(|| {
                                        format!("cannot export unknown name `{original}`")
                                    })?;
                                public.insert(exported, target.clone());
                            }
                            thaw_parser::ast::ExportSpecifier::Namespace(namespace) => {
                                let exported = export_name(&namespace.name)?;
                                let source = source_exports.ok_or_else(|| {
                                    "namespace exports require a source module".to_string()
                                })?;
                                public_namespaces.insert(exported, source.clone());
                            }
                            thaw_parser::ast::ExportSpecifier::Default(default) => {
                                let exported = default.exported.sym.to_string();
                                let source = source_exports.ok_or_else(|| {
                                    "default re-exports require a source module".to_string()
                                })?;
                                let target = source.get("default").ok_or_else(|| {
                                    "re-export source has no default export".to_string()
                                })?;
                                public.insert(exported, target.clone());
                            }
                        }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(mut export)) => {
                    match &mut export.decl {
                        thaw_parser::ast::DefaultDecl::Fn(function) => {
                            function.function.visit_mut_with(&mut RenameReferences {
                                names: &names,
                                namespaces: &namespaces,
                            });
                            let mut ident = function.ident.take().unwrap_or_else(|| {
                                thaw_parser::ast::Ident::new_no_ctxt(
                                    format!("__thawmod{index}_default").into(),
                                    function.function.span,
                                )
                            });
                            if let Some(replacement) = names.get(ident.sym.as_ref()) {
                                ident.sym = replacement.clone().into();
                            }
                            public.insert("default".to_string(), ident.sym.to_string());
                            items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(
                                thaw_parser::ast::FnDecl {
                                    ident,
                                    declare: false,
                                    function: function.function.clone(),
                                },
                            ))));
                        }
                        thaw_parser::ast::DefaultDecl::TsInterfaceDecl(interface) => {
                            let original = interface.id.sym.to_string();
                            interface.visit_mut_with(&mut RenameReferences {
                                names: &names,
                                namespaces: &namespaces,
                            });
                            public.insert("default".to_string(), interface.id.sym.to_string());
                            items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(
                                Decl::TsInterface(interface.clone()),
                            )));
                            let _ = original;
                        }
                        _ => return Err("default class exports are not supported yet".to_string()),
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(export)) => {
                    let Expr::Ident(ident) = export.expr.as_ref() else {
                        return Err(
                            "default export expressions must reference a top-level declaration"
                                .to_string(),
                        );
                    };
                    let original = ident.sym.to_string();
                    let target = names.get(&original).ok_or_else(|| {
                        format!("cannot default-export unknown name `{original}`")
                    })?;
                    public.insert("default".to_string(), target.clone());
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) => {
                    let specifier = export.src.value.as_str().ok_or("invalid export-all")?;
                    let dependency_exports = modules[index]
                        .dependencies
                        .get(specifier)
                        .map(|dependency| &exports[*dependency])
                        .or_else(|| external_exports.get(specifier))
                        .ok_or_else(|| format!("unresolved export-all `{specifier}`"))?;
                    for (name, target) in dependency_exports {
                        if name != "default"
                            && public.insert(name.clone(), target.clone()).is_some()
                        {
                            return Err(format!("duplicate re-export `{name}`"));
                        }
                    }
                }
                ModuleItem::ModuleDecl(_) => {
                    return Err("unsupported TypeScript module declaration".to_string())
                }
            }
        }
        exports[index] = public;
        namespace_exports[index] = public_namespaces;
        bundled_items.extend(items);
    }

    let mut seen = HashSet::new();
    bundled_items.retain(|item| {
        let name = match item {
            ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(decl))) => {
                Some(decl.ident.sym.to_string())
            }
            ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::TsInterface(decl))) => {
                Some(decl.id.sym.to_string())
            }
            _ => None,
        };
        name.map(|name| seen.insert(name)).unwrap_or(true)
    });
    Ok(Module {
        span: modules[entry_index].module.span,
        body: bundled_items,
        shebang: None,
    })
}
