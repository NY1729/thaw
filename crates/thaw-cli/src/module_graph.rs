use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use swc_ecma_visit::{Visit, VisitMut, VisitMutWith, VisitWith};
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

pub(crate) fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~') {
            url.push(char::from(*byte));
        } else {
            use std::fmt::Write;
            write!(url, "%{byte:02X}").unwrap();
        }
    }
    url
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(Path::new("/")),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if normalized != Path::new("/") {
                    normalized.pop();
                }
            }
            std::path::Component::Normal(component) => normalized.push(component),
        }
    }
    normalized
}

fn resolve_import_meta(
    module_path: &Path,
    specifier: &str,
    external_resolutions: &HashMap<String, String>,
) -> Option<String> {
    if specifier.starts_with("file://") {
        return Some(specifier.to_string());
    }
    let suffix_at = specifier
        .char_indices()
        .find_map(|(index, character)| matches!(character, '?' | '#').then_some(index));
    let (path, suffix) = suffix_at
        .map(|index| specifier.split_at(index))
        .unwrap_or((specifier, ""));
    if !path.starts_with('.') && !path.starts_with('/') {
        return external_resolutions
            .get(path)
            .map(|resolution| format!("{resolution}{suffix}"));
    }
    let target = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        module_path
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .join(path)
    };
    Some(format!(
        "{}{suffix}",
        file_url(&normalize_absolute_path(&target))
    ))
}

fn constant_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(thaw_parser::ast::Lit::Str(value)) => {
            Some(value.value.to_string_lossy().into_owned())
        }
        Expr::Tpl(template) if template.quasis.len() == template.exprs.len() + 1 => {
            let mut value = String::new();
            for (index, quasi) in template.quasis.iter().enumerate() {
                value.push_str(
                    &quasi
                        .cooked
                        .as_ref()
                        .map(|part| part.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string()),
                );
                if let Some(expr) = template.exprs.get(index) {
                    value.push_str(&constant_string(expr)?);
                }
            }
            Some(value)
        }
        Expr::Paren(parenthesized) => constant_string(&parenthesized.expr),
        Expr::Bin(binary) if binary.op == thaw_parser::ast::BinaryOp::Add => Some(format!(
            "{}{}",
            constant_string(&binary.left)?,
            constant_string(&binary.right)?
        )),
        Expr::TsAs(assertion) => constant_string(&assertion.expr),
        Expr::TsTypeAssertion(assertion) => constant_string(&assertion.expr),
        _ => None,
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
    let module = thaw_hir::normalize_top_level_destructuring(&module)
        .map_err(|error| format!("failed to normalize `{}`: {error}", path.display()))?;
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
    import_meta_url: &'a str,
    import_meta_main: bool,
    module_path: &'a Path,
    external_resolutions: &'a HashMap<String, String>,
    shadowed: HashSet<String>,
    function_depth: usize,
}

impl RenameReferences<'_> {
    fn rename_ident(&self, ident: &mut thaw_parser::ast::Ident) {
        if self.shadowed.contains(ident.sym.as_ref()) {
            return;
        }
        if let Some(replacement) = self.names.get(ident.sym.as_ref()) {
            ident.sym = replacement.clone().into();
        }
    }
}

#[derive(Default)]
struct LocalBindingCollector {
    names: HashSet<String>,
}

struct PatternBindingCollector<'a>(&'a mut HashSet<String>);

impl Visit for PatternBindingCollector<'_> {
    fn visit_binding_ident(&mut self, binding: &thaw_parser::ast::BindingIdent) {
        self.0.insert(binding.id.sym.to_string());
    }
}

impl Visit for LocalBindingCollector {
    fn visit_var_declarator(&mut self, declaration: &thaw_parser::ast::VarDeclarator) {
        declaration
            .name
            .visit_with(&mut PatternBindingCollector(&mut self.names));
    }

    fn visit_fn_decl(&mut self, declaration: &thaw_parser::ast::FnDecl) {
        self.names.insert(declaration.ident.sym.to_string());
    }

    fn visit_class_decl(&mut self, declaration: &thaw_parser::ast::ClassDecl) {
        self.names.insert(declaration.ident.sym.to_string());
    }

    fn visit_catch_clause(&mut self, clause: &thaw_parser::ast::CatchClause) {
        if let Some(parameter) = &clause.param {
            parameter.visit_with(&mut PatternBindingCollector(&mut self.names));
        }
        clause.body.visit_with(self);
    }

    fn visit_function(&mut self, _function: &thaw_parser::ast::Function) {}

    fn visit_arrow_expr(&mut self, _arrow: &thaw_parser::ast::ArrowExpr) {}
}

impl VisitMut for RenameReferences<'_> {
    fn visit_mut_function(&mut self, function: &mut thaw_parser::ast::Function) {
        let mut collector = LocalBindingCollector::default();
        for parameter in &function.params {
            parameter
                .pat
                .visit_with(&mut PatternBindingCollector(&mut collector.names));
        }
        if let Some(body) = &function.body {
            body.visit_with(&mut collector);
        }
        let saved = self.shadowed.clone();
        self.shadowed.extend(collector.names);
        self.function_depth += 1;
        function.visit_mut_children_with(self);
        self.function_depth -= 1;
        self.shadowed = saved;
    }

    fn visit_mut_arrow_expr(&mut self, arrow: &mut thaw_parser::ast::ArrowExpr) {
        let mut collector = LocalBindingCollector::default();
        for parameter in &arrow.params {
            parameter.visit_with(&mut PatternBindingCollector(&mut collector.names));
        }
        if let thaw_parser::ast::ArrowFunctionBody::FunctionBody(body) = &*arrow.body {
            body.visit_with(&mut collector);
        }
        let saved = self.shadowed.clone();
        self.shadowed.extend(collector.names);
        self.function_depth += 1;
        arrow.visit_mut_children_with(self);
        self.function_depth -= 1;
        self.shadowed = saved;
    }

    fn visit_mut_expr(&mut self, expr: &mut Expr) {
        expr.visit_mut_children_with(self);
        if let Expr::Call(call) = expr {
            let is_resolve = matches!(&call.callee, Callee::Expr(callee)
                if matches!(callee.as_ref(), Expr::Member(member)
                    if matches!(member.obj.as_ref(), Expr::MetaProp(meta)
                        if meta.kind == thaw_parser::ast::MetaPropKind::ImportMeta)
                    && matches!(&member.prop, thaw_parser::ast::MemberProp::Ident(property)
                        if property.sym == *"resolve")));
            if is_resolve && call.args.len() == 1 && call.args[0].spread.is_none() {
                if let Some(specifier) = constant_string(&call.args[0].expr) {
                    if let Some(resolved) =
                        resolve_import_meta(self.module_path, &specifier, self.external_resolutions)
                    {
                        *expr = Expr::Lit(thaw_parser::ast::Lit::Str(thaw_parser::ast::Str {
                            span: call.span,
                            value: resolved.into(),
                            raw: None,
                        }));
                        return;
                    }
                }
            }
        }
        if let Expr::Ident(ident) = expr {
            self.rename_ident(ident);
            return;
        }
        let Expr::Member(member) = expr else {
            return;
        };
        if let (Expr::Ident(namespace), thaw_parser::ast::MemberProp::Ident(property)) =
            (member.obj.as_ref(), &member.prop)
        {
            if !self.shadowed.contains(namespace.sym.as_ref()) {
                if let Some(target) = self
                    .namespaces
                    .get(namespace.sym.as_ref())
                    .and_then(|exports| exports.get(property.sym.as_ref()))
                {
                    *expr = Expr::Ident(thaw_parser::ast::Ident::new_no_ctxt(
                        target.clone().into(),
                        member.span,
                    ));
                    return;
                }
            }
        }
        let Expr::MetaProp(meta) = member.obj.as_ref() else {
            return;
        };
        if meta.kind != thaw_parser::ast::MetaPropKind::ImportMeta {
            return;
        }
        let thaw_parser::ast::MemberProp::Ident(property) = &member.prop else {
            return;
        };
        if property.sym == *"main" {
            *expr = Expr::Lit(thaw_parser::ast::Lit::Bool(thaw_parser::ast::Bool {
                span: member.span,
                value: self.import_meta_main,
            }));
            return;
        }
        let value = match property.sym.as_ref() {
            "url" => self.import_meta_url.to_string(),
            "filename" => self.module_path.to_string_lossy().into_owned(),
            "dirname" => self
                .module_path
                .parent()
                .unwrap_or_else(|| Path::new("/"))
                .to_string_lossy()
                .into_owned(),
            _ => return,
        };
        *expr = Expr::Lit(thaw_parser::ast::Lit::Str(thaw_parser::ast::Str {
            span: member.span,
            value: value.into(),
            raw: None,
        }));
    }

    fn visit_mut_call_expr(&mut self, call: &mut thaw_parser::ast::CallExpr) {
        call.visit_mut_children_with(self);
        if let Callee::Expr(callee) = &mut call.callee {
            match &mut **callee {
                Expr::Ident(ident) => {
                    // Direct imported/module calls keep the bundler's established
                    // callee resolution even when a local value has the same text
                    // name. Non-callee identifier reads remain lexically scoped.
                    if let Some(replacement) = self.names.get(ident.sym.as_ref()) {
                        ident.sym = replacement.clone().into();
                    }
                }
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

    fn visit_mut_ts_property_signature(
        &mut self,
        property: &mut thaw_parser::ast::TsPropertySignature,
    ) {
        if property.computed {
            property.key.visit_mut_with(self);
        }
        property.type_ann.visit_mut_with(self);
    }

    fn visit_mut_fn_decl(&mut self, function: &mut thaw_parser::ast::FnDecl) {
        function.function.visit_mut_with(self);
        if self.function_depth == 0 {
            if let Some(replacement) = self.names.get(function.ident.sym.as_ref()) {
                function.ident.sym = replacement.clone().into();
            }
        }
    }

    fn visit_mut_class_decl(&mut self, declaration: &mut thaw_parser::ast::ClassDecl) {
        declaration.class.visit_mut_with(self);
        if self.function_depth == 0 {
            if let Some(replacement) = self.names.get(declaration.ident.sym.as_ref()) {
                declaration.ident.sym = replacement.clone().into();
            }
        }
    }

    fn visit_mut_class_expr(&mut self, expression: &mut thaw_parser::ast::ClassExpr) {
        expression.class.visit_mut_with(self);
        if self.function_depth == 0 {
            if let Some(ident) = &mut expression.ident {
                if let Some(replacement) = self.names.get(ident.sym.as_ref()) {
                    ident.sym = replacement.clone().into();
                }
            }
        }
    }

    fn visit_mut_var_declarator(&mut self, declaration: &mut thaw_parser::ast::VarDeclarator) {
        declaration.visit_mut_children_with(self);
        if self.function_depth == 0 {
            if let thaw_parser::ast::Pat::Ident(binding) = &mut declaration.name {
                if let Some(replacement) = self.names.get(binding.id.sym.as_ref()) {
                    binding.id.sym = replacement.clone().into();
                }
            }
        }
    }

    fn visit_mut_assign_expr(&mut self, assignment: &mut thaw_parser::ast::AssignExpr) {
        assignment.visit_mut_children_with(self);
        if let thaw_parser::ast::AssignTarget::Simple(
            thaw_parser::ast::SimpleAssignTarget::Ident(binding),
        ) = &mut assignment.left
        {
            self.rename_ident(&mut binding.id);
        }
    }

    fn visit_mut_prop(&mut self, property: &mut thaw_parser::ast::Prop) {
        if let thaw_parser::ast::Prop::Shorthand(ident) = property {
            let original = ident.clone();
            self.rename_ident(ident);
            if ident.sym != original.sym {
                *property = thaw_parser::ast::Prop::KeyValue(thaw_parser::ast::KeyValueProp {
                    key: thaw_parser::ast::PropName::Ident(thaw_parser::ast::IdentName::new(
                        original.sym,
                        original.span,
                    )),
                    value: Box::new(thaw_parser::ast::Expr::Ident(ident.clone())),
                });
            }
        } else {
            property.visit_mut_children_with(self);
        }
    }
}

fn declaration_names(declaration: &Decl) -> Vec<String> {
    match declaration {
        Decl::Fn(declaration) if declaration.function.body.is_some() => {
            vec![declaration.ident.sym.to_string()]
        }
        Decl::Class(declaration) => vec![declaration.ident.sym.to_string()],
        Decl::TsInterface(declaration) => vec![declaration.id.sym.to_string()],
        Decl::Var(declaration) => declaration
            .decls
            .iter()
            .filter_map(|declarator| match &declarator.name {
                thaw_parser::ast::Pat::Ident(binding) => Some(binding.id.sym.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn declared_names(module: &Module) -> Vec<String> {
    module
        .body
        .iter()
        .flat_map(|item| match item {
            ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(declaration)) => {
                declaration_names(declaration)
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                declaration_names(&export.decl)
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
                thaw_parser::ast::DefaultDecl::Fn(function) => function
                    .ident
                    .as_ref()
                    .map(|ident| vec![ident.sym.to_string()])
                    .unwrap_or_default(),
                thaw_parser::ast::DefaultDecl::Class(class) => class
                    .ident
                    .as_ref()
                    .map(|ident| vec![ident.sym.to_string()])
                    .unwrap_or_default(),
                thaw_parser::ast::DefaultDecl::TsInterfaceDecl(interface) => {
                    vec![interface.id.sym.to_string()]
                }
            },
            _ => Vec::new(),
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
    external_namespace_aliases: &HashMap<String, HashSet<String>>,
    external_resolutions: &HashMap<String, String>,
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
    let mut ambiguous_exports: Vec<HashSet<String>> = vec![HashSet::new(); modules.len()];
    let mut bundled_items = Vec::new();
    for index in 0..modules.len() {
        let is_entry = index == entry_index;
        let mut names = HashMap::new();
        let mut namespaces = HashMap::new();
        let import_meta_url = file_url(&modules[index].path);
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
                    let is_external_default = !modules[index].dependencies.contains_key(specifier)
                        && matches!(imported, ImportSpecifier::Default(_));
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
                    // The external-package equivalent of the local-module
                    // check just above: `requested` doesn't name a real
                    // function/class/interface at all here, it's this
                    // package's own re-export of a self-referential
                    // namespace alias for its whole export table (real
                    // example: zod's own `export { z, z as default };`,
                    // where `z` is bound purely by an `import * as z`
                    // elsewhere in zod's own `.d.ts`) -- see
                    // `thaw_bridge::self_referential_namespace_aliases`'s
                    // own doc comment for why this can't just be another
                    // entry in `dependency_exports` the way an ordinary
                    // function/class name is. `import { z } from "zod"`
                    // then resolves exactly like `import * as z from
                    // "zod"` already does: the whole package's own export
                    // table, not a single symbol.
                    if !modules[index].dependencies.contains_key(specifier)
                        && external_namespace_aliases
                            .get(specifier)
                            .is_some_and(|aliases| aliases.contains(&requested))
                    {
                        namespaces.insert(local, dependency_exports.clone());
                        continue;
                    }
                    let target = dependency_exports.get(&requested).ok_or_else(|| {
                        if modules[index]
                            .dependencies
                            .get(specifier)
                            .is_some_and(|dependency| {
                                ambiguous_exports[*dependency].contains(&requested)
                            })
                        {
                            return format!(
                                "{}: `{specifier}` has an ambiguous star export named `{requested}`",
                                module_location(&modules[index], specifier)
                            );
                        }
                        format!(
                            "{}: `{specifier}` has no export named `{requested}`",
                            module_location(&modules[index], specifier)
                        )
                    })?;
                    names.insert(local.clone(), target.clone());
                    if is_external_default {
                        namespaces.insert(local, dependency_exports.clone());
                        namespaces.insert(target.clone(), dependency_exports.clone());
                    }
                }
            }
        }

        let mut public = HashMap::new();
        let mut public_namespaces = HashMap::new();
        let mut explicit_exports = HashSet::new();
        let mut ambiguous = HashSet::new();
        let mut items = Vec::new();
        for item in modules[index].module.body.clone() {
            match item {
                ModuleItem::Stmt(mut statement) => {
                    statement.visit_mut_with(&mut RenameReferences {
                        names: &names,
                        namespaces: &namespaces,
                        import_meta_url: &import_meta_url,
                        import_meta_main: is_entry,
                        module_path: &modules[index].path,
                        external_resolutions,
                        shadowed: HashSet::new(),
                        function_depth: 0,
                    });
                    items.push(ModuleItem::Stmt(statement));
                }
                ModuleItem::ModuleDecl(ModuleDecl::Import(_)) => {}
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(mut export)) => {
                    let originals = declaration_names(&export.decl);
                    export.decl.visit_mut_with(&mut RenameReferences {
                        names: &names,
                        namespaces: &namespaces,
                        import_meta_url: &import_meta_url,
                        import_meta_main: is_entry,
                        module_path: &modules[index].path,
                        external_resolutions,
                        shadowed: HashSet::new(),
                        function_depth: 0,
                    });
                    for original in originals {
                        public.insert(original.clone(), names[&original].clone());
                        explicit_exports.insert(original.clone());
                        ambiguous.remove(&original);
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
                                } else if let Some(exports_table) = source_exports {
                                    // The re-export equivalent of the
                                    // named-import check in this same
                                    // function's `Import` handling above --
                                    // `export { z } from "zod";` re-
                                    // exporting a self-referential
                                    // namespace alias (see
                                    // `thaw_bridge::self_referential_
                                    // namespace_aliases`'s own doc
                                    // comment), not a real function/class
                                    // name `exports_table` would have.
                                    let external_specifier = export
                                        .src
                                        .as_ref()
                                        .and_then(|source| source.value.as_str());
                                    if let Some(specifier) = external_specifier {
                                        if !modules[index].dependencies.contains_key(specifier)
                                            && external_namespace_aliases
                                                .get(specifier)
                                                .is_some_and(|aliases| aliases.contains(&original))
                                        {
                                            public_namespaces
                                                .insert(exported, exports_table.clone());
                                            continue;
                                        }
                                    }
                                }
                                let target = source_exports
                                    .and_then(|source| source.get(&original))
                                    .or_else(|| names.get(&original))
                                    .ok_or_else(|| {
                                        if let Some(source) = &export.src {
                                            if let Some(specifier) = source.value.as_str() {
                                                if modules[index]
                                                    .dependencies
                                                    .get(specifier)
                                                    .is_some_and(|dependency| {
                                                        ambiguous_exports[*dependency]
                                                            .contains(&original)
                                                    })
                                                {
                                                    return format!(
                                                        "cannot re-export ambiguous star export `{original}` from `{specifier}`"
                                                    );
                                                }
                                            }
                                        }
                                        format!("cannot export unknown name `{original}`")
                                    })?;
                                explicit_exports.insert(exported.clone());
                                ambiguous.remove(&exported);
                                public.insert(exported, target.clone());
                            }
                            thaw_parser::ast::ExportSpecifier::Namespace(namespace) => {
                                let exported = export_name(&namespace.name)?;
                                let source = source_exports.ok_or_else(|| {
                                    "namespace exports require a source module".to_string()
                                })?;
                                explicit_exports.insert(exported.clone());
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
                                explicit_exports.insert(exported.clone());
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
                                import_meta_url: &import_meta_url,
                                import_meta_main: is_entry,
                                module_path: &modules[index].path,
                                external_resolutions,
                                shadowed: HashSet::new(),
                                function_depth: 0,
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
                            explicit_exports.insert("default".to_string());
                            items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Fn(
                                thaw_parser::ast::FnDecl {
                                    ident,
                                    declare: false,
                                    function: function.function.clone(),
                                },
                            ))));
                        }
                        thaw_parser::ast::DefaultDecl::Class(class) => {
                            class.class.visit_mut_with(&mut RenameReferences {
                                names: &names,
                                namespaces: &namespaces,
                                import_meta_url: &import_meta_url,
                                import_meta_main: is_entry,
                                module_path: &modules[index].path,
                                external_resolutions,
                                shadowed: HashSet::new(),
                                function_depth: 0,
                            });
                            let mut ident = class.ident.take().unwrap_or_else(|| {
                                thaw_parser::ast::Ident::new_no_ctxt(
                                    format!("__thawmod{index}_default").into(),
                                    class.class.span,
                                )
                            });
                            if let Some(replacement) = names.get(ident.sym.as_ref()) {
                                ident.sym = replacement.clone().into();
                            }
                            public.insert("default".to_string(), ident.sym.to_string());
                            explicit_exports.insert("default".to_string());
                            items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(
                                Decl::Class(thaw_parser::ast::ClassDecl {
                                    ident,
                                    declare: false,
                                    class: class.class.clone(),
                                }),
                            )));
                        }
                        thaw_parser::ast::DefaultDecl::TsInterfaceDecl(interface) => {
                            let original = interface.id.sym.to_string();
                            interface.visit_mut_with(&mut RenameReferences {
                                names: &names,
                                namespaces: &namespaces,
                                import_meta_url: &import_meta_url,
                                import_meta_main: is_entry,
                                module_path: &modules[index].path,
                                external_resolutions,
                                shadowed: HashSet::new(),
                                function_depth: 0,
                            });
                            public.insert("default".to_string(), interface.id.sym.to_string());
                            explicit_exports.insert("default".to_string());
                            items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(
                                Decl::TsInterface(interface.clone()),
                            )));
                            let _ = original;
                        }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(mut export)) => {
                    if let Expr::Ident(ident) = export.expr.as_ref() {
                        let original = ident.sym.to_string();
                        if let Some(target) = names.get(&original) {
                            public.insert("default".to_string(), target.clone());
                            explicit_exports.insert("default".to_string());
                            continue;
                        }
                    }

                    export.expr.visit_mut_with(&mut RenameReferences {
                        names: &names,
                        namespaces: &namespaces,
                        import_meta_url: &import_meta_url,
                        import_meta_main: is_entry,
                        module_path: &modules[index].path,
                        external_resolutions,
                        shadowed: HashSet::new(),
                        function_depth: 0,
                    });
                    let symbol = format!("__thawmod{index}_default_value");
                    let ident =
                        thaw_parser::ast::Ident::new_no_ctxt(symbol.clone().into(), export.span);
                    items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(Decl::Var(
                        Box::new(thaw_parser::ast::VarDecl {
                            span: export.span,
                            ctxt: Default::default(),
                            kind: thaw_parser::ast::VarDeclKind::Const,
                            declare: false,
                            decls: vec![thaw_parser::ast::VarDeclarator {
                                span: export.span,
                                name: thaw_parser::ast::Pat::Ident(ident.into()),
                                init: Some(export.expr),
                                definite: false,
                            }],
                        }),
                    ))));
                    public.insert("default".to_string(), symbol);
                    explicit_exports.insert("default".to_string());
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) => {
                    let specifier = export.src.value.as_str().ok_or("invalid export-all")?;
                    let dependency_ambiguous = modules[index]
                        .dependencies
                        .get(specifier)
                        .map(|dependency| &ambiguous_exports[*dependency]);
                    let dependency_exports = modules[index]
                        .dependencies
                        .get(specifier)
                        .map(|dependency| &exports[*dependency])
                        .or_else(|| external_exports.get(specifier))
                        .ok_or_else(|| format!("unresolved export-all `{specifier}`"))?;
                    for (name, target) in dependency_exports {
                        if name == "default" || explicit_exports.contains(name) {
                            continue;
                        }
                        if let Some(existing) = public.get(name) {
                            if existing != target {
                                public.remove(name);
                                ambiguous.insert(name.clone());
                            }
                        } else if !ambiguous.contains(name) {
                            public.insert(name.clone(), target.clone());
                        }
                    }
                    if let Some(dependency_ambiguous) = dependency_ambiguous {
                        for name in dependency_ambiguous {
                            if !explicit_exports.contains(name) {
                                public.remove(name);
                                ambiguous.insert(name.clone());
                            }
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
        ambiguous_exports[index] = ambiguous;
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
