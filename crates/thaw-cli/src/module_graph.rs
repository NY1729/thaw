use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use swc_ecma_visit::{Visit, VisitMut, VisitMutWith, VisitWith};
use thaw_parser::ast::{
    Callee, Decl, Expr, ImportSpecifier, MemberProp, Module, ModuleDecl, ModuleExportName,
    ModuleItem, Pat, TsEntityName, TsInterfaceDecl, TsTypeRef, VarDeclarator,
};

type SourceTransform<'a> = dyn Fn(&str, &Path, bool) -> Result<String, String> + 'a;
pub(crate) const EXTERNAL_MODULE_OFFSET: usize = 1_000_000;

#[derive(Debug)]
struct LoadedModule {
    path: PathBuf,
    source: String,
    module: Module,
    dependencies: HashMap<String, usize>,
    runtime_dependencies: HashSet<usize>,
    static_dependencies: HashSet<usize>,
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
    let candidates = relative_candidates(from, specifier);
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

fn relative_candidates(from: &Path, specifier: &str) -> Vec<PathBuf> {
    let raw = normalize_absolute_path(
        &from.parent().unwrap_or_else(|| Path::new(".")).join(specifier)
    );
    if raw.extension().is_some() {
        vec![raw]
    } else {
        vec![raw.with_extension("ts"), raw.join("index.ts")]
    }
}

fn dependency_specifiers(module: &Module) -> Result<Vec<(String, bool, bool)>, String> {
    let mut specs = Vec::new();
    let mut runtime = HashSet::new();
    let mut static_runtime = HashSet::new();
    let mut create_require_functions = HashSet::new();
    let mut module_namespaces = HashSet::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(decl) = item else {
            continue;
        };
        let source = match decl {
            ModuleDecl::Import(import) => Some((&import.src, !import.type_only
                && (import.specifiers.is_empty() || import.specifiers.iter().any(|specifier| {
                    !matches!(specifier, ImportSpecifier::Named(named) if named.is_type_only)
                })))),
            ModuleDecl::ExportNamed(export) => export.src.as_ref().map(|source| (source,
                !export.type_only && (export.specifiers.is_empty()
                    || export.specifiers.iter().any(|specifier| {
                        !matches!(specifier, thaw_parser::ast::ExportSpecifier::Named(named) if named.is_type_only)
                    })))),
            ModuleDecl::ExportAll(export) => Some((&export.src, !export.type_only)),
            _ => None,
        };
        if let Some((source, is_runtime)) = source {
            let spec = source
                .value
                .as_str()
                .ok_or_else(|| "module specifiers must be valid UTF-8".to_string())?;
            if !specs.iter().any(|existing| existing == spec) {
                specs.push(spec.to_string());
            }
            if is_runtime {
                runtime.insert(spec.to_string());
                static_runtime.insert(spec.to_string());
            }
            if matches!(spec, "module" | "node:module") {
                if let ModuleDecl::Import(import) = decl {
                    for specifier in &import.specifiers {
                        match specifier {
                            ImportSpecifier::Named(named)
                                if export_name(&named.imported.clone().unwrap_or_else(|| {
                                    ModuleExportName::Ident(named.local.clone())
                                }))? == "createRequire" =>
                            {
                                create_require_functions.insert(named.local.sym.to_string());
                            }
                            ImportSpecifier::Namespace(namespace) => {
                                module_namespaces.insert(namespace.local.sym.to_string());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    struct CreatedRequires<'a> {
        functions: &'a HashSet<String>,
        namespaces: &'a HashSet<String>,
        aliases: HashSet<String>,
    }
    impl Visit for CreatedRequires<'_> {
        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            let (Pat::Ident(binding), Some(Expr::Call(call))) =
                (&declaration.name, declaration.init.as_deref())
            else {
                declaration.visit_children_with(self);
                return;
            };
            let created = match &call.callee {
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(function) => self.functions.contains(function.sym.as_str()),
                    Expr::Member(member) => matches!(
                        (member.obj.as_ref(), &member.prop),
                        (Expr::Ident(namespace), MemberProp::Ident(method))
                            if self.namespaces.contains(namespace.sym.as_str())
                                && method.sym == *"createRequire"
                    ),
                    _ => false,
                },
                _ => false,
            };
            if created {
                self.aliases.insert(binding.id.sym.to_string());
            }
            declaration.visit_children_with(self);
        }
    }
    let mut created = CreatedRequires {
        functions: &create_require_functions,
        namespaces: &module_namespaces,
        aliases: HashSet::new(),
    };
    module.visit_with(&mut created);

    struct RequiredSpecs<'a> {
        aliases: &'a HashSet<String>,
        specs: Vec<String>,
    }
    impl Visit for RequiredSpecs<'_> {
        fn visit_call_expr(&mut self, call: &thaw_parser::ast::CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if matches!(callee.as_ref(), Expr::Ident(name) if self.aliases.contains(name.sym.as_str()))
                {
                    if let Some(specifier) = call.args.first().and_then(|argument| {
                        argument
                            .spread
                            .is_none()
                            .then(|| constant_string(&argument.expr))
                            .flatten()
                    }) {
                        self.specs.push(specifier);
                    }
                }
            }
            call.visit_children_with(self);
        }
    }
    let mut required = RequiredSpecs {
        aliases: &created.aliases,
        specs: Vec::new(),
    };
    module.visit_with(&mut required);
    for specifier in required.specs {
        runtime.insert(specifier.clone());
        static_runtime.insert(specifier.clone());
        if !specs.contains(&specifier) {
            specs.push(specifier);
        }
    }

    // `import("literal")` -- only a literal string specifier is
    // supported (a computed one, e.g. a template literal with
    // substitutions, would need real runtime module resolution this
    // ahead-of-time compiler doesn't have); other shapes are simply
    // left alone here, same as an unresolvable relative `require`
    // would be, and surface as a real error later when the call itself
    // is rewritten (see `RenameReferences::visit_mut_expr`).
    struct DynamicImportSpecs {
        specs: Vec<String>,
    }
    impl Visit for DynamicImportSpecs {
        fn visit_call_expr(&mut self, call: &thaw_parser::ast::CallExpr) {
            if matches!(call.callee, Callee::Import(_)) {
                if let Some(specifier) = call.args.first().and_then(|argument| {
                    argument
                        .spread
                        .is_none()
                        .then(|| constant_string(&argument.expr))
                        .flatten()
                }) {
                    if !self.specs.contains(&specifier) {
                        self.specs.push(specifier);
                    }
                }
            }
            call.visit_children_with(self);
        }
    }
    let mut dynamic_imports = DynamicImportSpecs { specs: Vec::new() };
    module.visit_with(&mut dynamic_imports);
    for specifier in dynamic_imports.specs {
        runtime.insert(specifier.clone());
        if !specs.contains(&specifier) {
            specs.push(specifier);
        }
    }
    Ok(specs.into_iter().map(|specifier| {
        let is_runtime = runtime.contains(&specifier);
        let is_static = static_runtime.contains(&specifier);
        (specifier, is_runtime, is_static)
    }).collect())
}

fn load_module(
    path: PathBuf,
    source_override: Option<&str>,
    transform: Option<&SourceTransform<'_>>,
    modules: &mut Vec<LoadedModule>,
    loaded: &mut HashMap<PathBuf, usize>,
    watched: &mut HashSet<PathBuf>,
) -> Result<usize, String> {
    watched.insert(path.clone());
    if let Some(index) = loaded.get(&path) {
        return Ok(*index);
    }
    let source = match source_override {
        Some(source) => source.to_string(),
        None => std::fs::read_to_string(&path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?,
    };
    let source = match transform {
        Some(transform) => transform(&source, &path, source_override.is_some())
            .map_err(|error| format!("failed to transform `{}`: {error}", path.display()))?,
        None => source,
    };
    let source_name = if source_override.is_some() || transform.is_some() {
        thaw_parser::common::FileName::Custom(format!("{} (in-memory module source)", path.display()).into())
    } else {
        thaw_parser::common::FileName::Real(path.clone())
    };
    let module = thaw_parser::parse_typescript_with_source_map_named(&source, source_name)
        .map(|(module, _)| module)
        .map_err(|error| format!("failed to parse `{}`: {error}", path.display()))?;
    let module = thaw_hir::normalize_top_level_destructuring(&module)
        .map_err(|error| format!("failed to normalize `{}`: {error}", path.display()))?;
    let specifiers = dependency_specifiers(&module)?;
    // Reserve identity before descending so a dynamic or type-only back edge
    // can refer to this module. Static cycles are checked separately against
    // the static dependency graph before emission.
    let index = modules.len();
    modules.push(LoadedModule {
        path: path.clone(), source: source.clone(), module,
        dependencies: HashMap::new(), runtime_dependencies: HashSet::new(),
        static_dependencies: HashSet::new(),
    });
    loaded.insert(path.clone(), index);
    let mut dependencies = HashMap::new();
    let mut runtime_dependencies = HashSet::new();
    let mut static_dependencies = HashSet::new();
    for (specifier, is_runtime, is_static) in specifiers {
        if !specifier.starts_with('.') {
            continue;
        }
        let dependency_path = resolve_relative(&path, &specifier).map_err(|error| {
            watched.extend(relative_candidates(&path, &specifier));
            format!("{}: {error}", source_location(&path, &source, &specifier))
        })?;
        let dependency = load_module(dependency_path, None, transform, modules, loaded, watched)?;
        if is_runtime { runtime_dependencies.insert(dependency); }
        if is_static { static_dependencies.insert(dependency); }
        dependencies.insert(specifier, dependency);
    }
    modules[index].dependencies = dependencies;
    modules[index].runtime_dependencies = runtime_dependencies;
    modules[index].static_dependencies = static_dependencies;
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
    /// `local namespace alias -> { nested namespace name -> { member name
    /// -> real flattened target } }` -- lets a *two*-level member chain
    /// (`z.coerce.number(...)`) rewrite straight to the flattened function
    /// it names, the way `namespaces` already does for one level
    /// (`z.number(...)`). See `thaw_bridge::nested_namespace_members`'s
    /// own doc comment; real example: zod's `z.coerce`, `z.core`, `z.iso`.
    nested_namespaces: &'a HashMap<String, HashMap<String, HashMap<String, String>>>,
    /// Literal import specifier -> guarded initializer and resolved exports.
    /// Both local modules and registry packages have compiler-owned entries.
    /// Computed specifiers still reach HIR's unsupported-import error.
    dynamic_imports: &'a HashMap<String, usize>,
    /// Local names bound from an *external* (npm/registry) import, which
    /// have no declaration in the compiled program under that name -- see
    /// `visit_mut_expr`'s own `InstanceOf` handling.
    external_bindings: &'a HashSet<String>,
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
    fn visit_block_stmt(&mut self, _block: &thaw_parser::ast::BlockStmt) {}

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

    fn visit_catch_clause(&mut self, _clause: &thaw_parser::ast::CatchClause) {}

    fn visit_function(&mut self, _function: &thaw_parser::ast::Function) {}

    fn visit_arrow_expr(&mut self, _arrow: &thaw_parser::ast::ArrowExpr) {}

    fn visit_for_stmt(&mut self, _statement: &thaw_parser::ast::ForStmt) {}
    fn visit_for_in_stmt(&mut self, _statement: &thaw_parser::ast::ForInStmt) {}
    fn visit_for_of_stmt(&mut self, _statement: &thaw_parser::ast::ForOfStmt) {}
    fn visit_switch_stmt(&mut self, _statement: &thaw_parser::ast::SwitchStmt) {}
}

#[derive(Default)]
struct HoistedBindingCollector(LocalBindingCollector);

impl Visit for HoistedBindingCollector {
    fn visit_var_decl(&mut self, declaration: &thaw_parser::ast::VarDecl) {
        if declaration.kind == thaw_parser::ast::VarDeclKind::Var {
            for binding in &declaration.decls {
                binding
                    .name
                    .visit_with(&mut PatternBindingCollector(&mut self.0.names));
            }
        }
    }
    fn visit_function(&mut self, _function: &thaw_parser::ast::Function) {}
    fn visit_arrow_expr(&mut self, _arrow: &thaw_parser::ast::ArrowExpr) {}
}

impl VisitMut for RenameReferences<'_> {
    fn visit_mut_switch_stmt(&mut self, statement: &mut thaw_parser::ast::SwitchStmt) {
        statement.discriminant.visit_mut_with(self);
        let saved = self.shadowed.clone();
        let mut collector = LocalBindingCollector::default();
        for case in &statement.cases {
            for statement in &case.cons {
                statement.visit_with(&mut collector);
            }
        }
        self.shadowed.extend(collector.names);
        statement.cases.visit_mut_with(self);
        self.shadowed = saved;
    }
    fn visit_mut_for_stmt(&mut self, statement: &mut thaw_parser::ast::ForStmt) {
        let saved = self.shadowed.clone();
        if let Some(thaw_parser::ast::VarDeclOrExpr::VarDecl(declaration)) = &statement.init {
            declaration.visit_with(&mut PatternBindingCollector(&mut self.shadowed));
        }
        statement.visit_mut_children_with(self);
        self.shadowed = saved;
    }

    fn visit_mut_for_in_stmt(&mut self, statement: &mut thaw_parser::ast::ForInStmt) {
        let saved = self.shadowed.clone();
        statement
            .left
            .visit_with(&mut PatternBindingCollector(&mut self.shadowed));
        statement.visit_mut_children_with(self);
        self.shadowed = saved;
    }

    fn visit_mut_for_of_stmt(&mut self, statement: &mut thaw_parser::ast::ForOfStmt) {
        let saved = self.shadowed.clone();
        statement
            .left
            .visit_with(&mut PatternBindingCollector(&mut self.shadowed));
        statement.visit_mut_children_with(self);
        self.shadowed = saved;
    }
    fn visit_mut_block_stmt(&mut self, block: &mut thaw_parser::ast::BlockStmt) {
        let mut collector = LocalBindingCollector::default();
        for statement in &block.stmts {
            statement.visit_with(&mut collector);
        }
        let saved = self.shadowed.clone();
        self.shadowed.extend(collector.names);
        block.visit_mut_children_with(self);
        self.shadowed = saved;
    }

    fn visit_mut_catch_clause(&mut self, clause: &mut thaw_parser::ast::CatchClause) {
        let saved = self.shadowed.clone();
        if let Some(parameter) = &clause.param {
            parameter.visit_with(&mut PatternBindingCollector(&mut self.shadowed));
        }
        clause.visit_mut_children_with(self);
        self.shadowed = saved;
    }

    fn visit_mut_function(&mut self, function: &mut thaw_parser::ast::Function) {
        let mut collector = LocalBindingCollector::default();
        for parameter in &function.params {
            parameter
                .pat
                .visit_with(&mut PatternBindingCollector(&mut collector.names));
        }
        let mut hoisted = HoistedBindingCollector::default();
        function.body.visit_with(&mut hoisted);
        collector.names.extend(hoisted.0.names);
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
        let mut hoisted = HoistedBindingCollector::default();
        arrow.body.visit_with(&mut hoisted);
        collector.names.extend(hoisted.0.names);
        let saved = self.shadowed.clone();
        self.shadowed.extend(collector.names);
        self.function_depth += 1;
        arrow.visit_mut_children_with(self);
        self.function_depth -= 1;
        self.shadowed = saved;
    }

    fn visit_mut_expr(&mut self, expr: &mut Expr) {
        // Resolve a complete external namespace path before a child member
        // can be rewritten on its own. The export table keeps qualified
        // keys such as `Outer.Inner.make` distinct from another `make`.
        if let Expr::Member(member) = &*expr {
            let mut path = Vec::new();
            let mut current: &Expr = expr;
            while let Expr::Member(segment) = current {
                let thaw_parser::ast::MemberProp::Ident(property) = &segment.prop else { break; };
                path.push(property.sym.as_ref());
                current = segment.obj.as_ref();
            }
            if let Expr::Ident(root) = current {
                if !self.shadowed.contains(root.sym.as_ref()) && path.len() > 1 {
                    path.reverse();
                    if let Some(target) = self.namespaces.get(root.sym.as_ref())
                        .and_then(|exports| exports.get(&path.join(".")))
                    {
                        let span = member.span;
                        let target = target.clone();
                        *expr = Expr::Ident(thaw_parser::ast::Ident::new_no_ctxt(
                            target.into(), span,
                        ));
                        return;
                    }
                }
            }
        }
        // `a instanceof B`'s right operand is a bare class-identifier
        // *type* reference, not an ordinary value read. A *local* class
        // (declared in this module, or imported from a relative module in
        // this same program) is renamed like any other declaration
        // reference, so thaw-hir's own `instanceof` lowering can find that
        // class's constructor signature under its real, rewritten name
        // (real bug: `class User {} ... u instanceof User` failed "not a
        // known class" because the operand was left bare while the class
        // itself was renamed to `__thawmod0_User`). An *external*
        // (npm/registry) imported name has no such declaration in the
        // program; it stays exactly as written so the shim's own bare
        // ambient stub still matches (real trigger: a real npm-exported
        // `class YAMLException extends Error {}`, imported as a value
        // elsewhere in the same file -- see `[[project_npm_interop_gaps_
        // 19]]`). Visits the left operand normally either way. A
        // non-identifier right operand (a member expression) is still
        // left completely untouched, as before.
        if let Expr::Bin(binary) = expr {
            if binary.op == thaw_parser::ast::BinaryOp::InstanceOf {
                binary.left.visit_mut_with(self);
                let rename_right = matches!(
                    binary.right.as_ref(),
                    Expr::Ident(ident) if !self.external_bindings.contains(ident.sym.as_ref())
                );
                if rename_right {
                    binary.right.visit_mut_with(self);
                }
                return;
            }
        }
        // Resolve a three-level nested-namespace chain (`ns.sub.member`)
        // *before* visiting children. The inner `ns.sub` is often itself
        // resolvable as a single-level namespace member (real trigger:
        // winston's `winston.format.json`/`winston.transports.Console`,
        // where `winston` is the default-imported `export = winston`
        // namespace over its whole export table and both `format` and
        // `transports` are themselves exported namespace aliases), so the
        // child pass below would rewrite `ns.sub` first and leave
        // `.member` dangling on a symbol that no longer names the
        // namespace -- producing e.g. "call to unknown function
        // `__thaw_typed_js_....format.json`". The identical lookup is kept
        // after the child pass too, for a chain whose middle segment is
        // *only* a nested namespace (zod's `z.coerce.number`, where
        // `z.coerce` has no single-level target to begin with).
        if let Expr::Member(member) = expr {
            if let thaw_parser::ast::MemberProp::Ident(property) = &member.prop {
                if let Expr::Member(inner) = member.obj.as_ref() {
                    if let (Expr::Ident(namespace), thaw_parser::ast::MemberProp::Ident(sub)) =
                        (inner.obj.as_ref(), &inner.prop)
                    {
                        if !self.shadowed.contains(namespace.sym.as_ref()) {
                            if let Some(target) = self
                                .nested_namespaces
                                .get(namespace.sym.as_ref())
                                .and_then(|namespaces| namespaces.get(sub.sym.as_ref()))
                                .and_then(|members| members.get(property.sym.as_ref()))
                            {
                                *expr = Expr::Ident(thaw_parser::ast::Ident::new_no_ctxt(
                                    target.clone().into(),
                                    member.span,
                                ));
                                return;
                            }
                        }
                    }
                }
            }
        }
        expr.visit_mut_children_with(self);
        if let Expr::Call(call) = expr {
            if matches!(call.callee, Callee::Import(_)) {
                if let Some(specifier) = call.args.first().and_then(|argument| {
                    argument
                        .spread
                        .is_none()
                        .then(|| constant_string(&argument.expr))
                        .flatten()
                }) {
                    if let Some(dependency) = self.dynamic_imports.get(&specifier) {
                        // A Promise continuation executes the dependency's
                        // guarded initializer, then returns the stable live
                        // namespace. The generated identifiers come from
                        // compiler-owned module indices.
                        let snippet = format!(
                            "Promise.resolve().then(() => {{ __thaw_lazy_module_{dependency}(); return __thaw_namespace_{dependency}; }})"
                        );
                        if let Ok((mut parsed, _)) = thaw_parser::parse_typescript_with_source_map_named(
                            &snippet, thaw_parser::common::FileName::Custom("generated dynamic import initializer.ts".into()),
                        ) {
                            if let Some(ModuleItem::Stmt(thaw_parser::ast::Stmt::Expr(statement))) =
                                parsed.body.pop()
                            {
                                *expr = *statement.expr;
                                return;
                            }
                        }
                    }
                }
            }
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
        // A *two*-level member chain (`z.coerce.number(...)`) -- `z.coerce`
        // isn't itself a real bound value, so this must match the whole
        // chain at once rather than resolving `z.coerce` to something and
        // then reading `.number` off it (see `nested_namespaces`'s own doc
        // comment). Checked before the single-level case just below, which
        // wouldn't match here anyway (`member.obj` is a `Member`, not an
        // `Ident`).
        if let thaw_parser::ast::MemberProp::Ident(property) = &member.prop {
            if let Expr::Member(inner) = member.obj.as_ref() {
                if let (Expr::Ident(namespace), thaw_parser::ast::MemberProp::Ident(sub)) =
                    (inner.obj.as_ref(), &inner.prop)
                {
                    if !self.shadowed.contains(namespace.sym.as_ref()) {
                        if let Some(target) = self
                            .nested_namespaces
                            .get(namespace.sym.as_ref())
                            .and_then(|namespaces| namespaces.get(sub.sym.as_ref()))
                            .and_then(|members| members.get(property.sym.as_ref()))
                        {
                            *expr = Expr::Ident(thaw_parser::ast::Ident::new_no_ctxt(
                                target.clone().into(),
                                member.span,
                            ));
                            return;
                        }
                    }
                }
            }
        }
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

    fn visit_mut_ts_type_alias_decl(&mut self, alias: &mut thaw_parser::ast::TsTypeAliasDecl) {
        alias.visit_mut_children_with(self);
        if self.function_depth == 0 {
            self.rename_ident(&mut alias.id);
        }
    }

    fn visit_mut_ts_enum_decl(&mut self, declaration: &mut thaw_parser::ast::TsEnumDecl) {
        declaration.visit_mut_children_with(self);
        if self.function_depth == 0 {
            self.rename_ident(&mut declaration.id);
        }
    }

    fn visit_mut_ts_module_decl(&mut self, declaration: &mut thaw_parser::ast::TsModuleDecl) {
        declaration.visit_mut_children_with(self);
        if self.function_depth == 0 {
            if let thaw_parser::ast::TsModuleName::Ident(name) = &mut declaration.id {
                self.rename_ident(name);
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
        Decl::TsTypeAlias(declaration) => vec![declaration.id.sym.to_string()],
        Decl::TsEnum(declaration) => vec![declaration.id.sym.to_string()],
        Decl::TsModule(declaration) => match &declaration.id {
            thaw_parser::ast::TsModuleName::Ident(name) => vec![name.sym.to_string()],
            thaw_parser::ast::TsModuleName::Str(_) => Vec::new(),
        },
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

fn reachable_modules(modules: &[LoadedModule], entry: usize, static_only: bool) -> HashSet<usize> {
    let mut reachable = HashSet::new();
    let mut pending = vec![entry];
    while let Some(index) = pending.pop() {
        if reachable.insert(index) {
            pending.extend(if static_only {
                &modules[index].static_dependencies
            } else {
                &modules[index].runtime_dependencies
            }.iter().copied());
        }
    }
    reachable
}

fn module_emission_order(modules: &[LoadedModule]) -> Result<Vec<usize>, String> {
    fn visit(
        index: usize,
        modules: &[LoadedModule],
        states: &mut [u8],
        stack: &mut Vec<usize>,
        order: &mut Vec<usize>,
    ) -> Result<(), String> {
        if states[index] == 2 { return Ok(()); }
        if states[index] == 1 {
            let start = stack.iter().position(|item| *item == index).unwrap_or(0);
            let cycle = stack[start..].iter().copied().chain(std::iter::once(index))
                .map(|item| modules[item].path.display().to_string())
                .collect::<Vec<_>>().join(" -> ");
            return Err(format!("cyclic static user-module import: {cycle}"));
        }
        states[index] = 1;
        stack.push(index);
        let mut dependencies = modules[index].static_dependencies.iter().copied().collect::<Vec<_>>();
        dependencies.sort_unstable();
        for dependency in dependencies { visit(dependency, modules, states, stack, order)?; }
        stack.pop();
        states[index] = 2;
        order.push(index);
        Ok(())
    }
    let mut states = vec![0; modules.len()];
    let mut order = Vec::with_capacity(modules.len());
    let mut stack = Vec::new();
    for index in 0..modules.len() {
        visit(index, modules, &mut states, &mut stack, &mut order)?;
    }
    Ok(order)
}

pub fn runtime_features(
    entry: &Path,
    source: &str,
) -> Result<std::collections::BTreeSet<&'static str>, String> {
    let mut modules = Vec::new();
    load_module(
        entry.canonicalize().map_err(|error| error.to_string())?,
        Some(source),
        None,
        &mut modules,
        &mut HashMap::new(),
        &mut HashSet::new(),
    )?;
    Ok(modules
        .iter()
        // Type-only modules retain their declarations and function bodies
        // for type resolution, so code generation may still need their
        // runtime symbols even though their initializers are not executed.
        .flat_map(|module| thaw_bridge::required_runtime_features(&module.source))
        .collect())
}

/// The same resolved user-module paths consumed by the build, for `dev`'s
/// watch set. Re-resolving after a rebuild picks up newly imported modules.
pub fn source_paths(entry: &Path) -> Result<(Vec<PathBuf>, bool), String> {
    let entry = entry.canonicalize()
        .map_err(|error| format!("failed to resolve `{}`: {error}", entry.display()))?;
    let mut modules = Vec::new();
    let mut watched = HashSet::new();
    // A missing newly imported module is itself a watch target. The build
    // still reports the resolver error; `dev` keeps listening for its creation.
    let complete = load_module(entry, None, None, &mut modules, &mut HashMap::new(), &mut watched)
        .and_then(|_| module_emission_order(&modules).map(|_| ())).is_ok();
    let mut paths = watched.into_iter().collect::<Vec<_>>();
    paths.sort();
    Ok((paths, complete))
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
        None,
        &mut modules,
        &mut HashMap::new(),
        &mut HashSet::new(),
    )?;
    let mut result = Vec::new();
    for index in module_emission_order(&modules)? {
        let module = &modules[index];
        for (specifier, _, _) in dependency_specifiers(&module.module)? {
            if !specifier.starts_with('.')
                && !result.iter().any(|(existing, _)| existing == &specifier)
            {
                result.push((specifier.clone(), module_location(&module, &specifier)));
            }
        }
    }
    Ok(result)
}

/// External packages needed at runtime. The complete specifier list above is
/// still used to resolve `.d.ts` exports for type-only imports.
pub fn external_runtime_specifiers(
    entry: &Path,
    entry_source: &str,
) -> Result<HashSet<String>, String> {
    let mut modules = Vec::new();
    load_module(
        entry.canonicalize().map_err(|error| error.to_string())?,
        Some(entry_source),
        None,
        &mut modules,
        &mut HashMap::new(),
        &mut HashSet::new(),
    )?;
    let mut runtime = HashSet::new();
    for module in &modules {
        for (specifier, is_runtime, _) in dependency_specifiers(&module.module)? {
            if is_runtime && !specifier.starts_with('.') {
                runtime.insert(specifier);
            }
        }
    }
    Ok(runtime)
}

pub fn external_static_specifiers(
    entry: &Path,
    entry_source: &str,
) -> Result<HashSet<String>, String> {
    let mut modules = Vec::new();
    let entry_index = load_module(
        entry.canonicalize().map_err(|error| error.to_string())?,
        Some(entry_source),
        None,
        &mut modules,
        &mut HashMap::new(),
        &mut HashSet::new(),
    )?;
    let reachable = reachable_modules(&modules, entry_index, true);
    let mut static_packages = HashSet::new();
    for (index, module) in modules.iter().enumerate() {
        if !reachable.contains(&index) { continue; }
        for (specifier, _, is_static) in dependency_specifiers(&module.module)? {
            if is_static && !specifier.starts_with('.') {
                static_packages.insert(specifier);
            }
        }
    }
    Ok(static_packages)
}

pub fn external_dynamic_specifiers(
    entry: &Path,
    entry_source: &str,
) -> Result<HashSet<String>, String> {
    struct DynamicImports { specs: HashSet<String> }
    impl Visit for DynamicImports {
        fn visit_call_expr(&mut self, call: &thaw_parser::ast::CallExpr) {
            if matches!(call.callee, Callee::Import(_)) {
                if let Some(specifier) = call.args.first()
                    .filter(|argument| argument.spread.is_none())
                    .and_then(|argument| constant_string(&argument.expr)) {
                    if !specifier.starts_with('.') { self.specs.insert(specifier); }
                }
            }
            call.visit_children_with(self);
        }
    }
    let mut modules = Vec::new();
    load_module(
        entry.canonicalize().map_err(|error| error.to_string())?,
        Some(entry_source), None, &mut modules, &mut HashMap::new(),
        &mut HashSet::new(),
    )?;
    let mut found = DynamicImports { specs: HashSet::new() };
    for module in &modules { module.module.visit_with(&mut found); }
    Ok(found.specs)
}

/// `nested`'s members name a package's own flattened `.d.ts` function
/// (e.g. `__thaw_ns_coerce_number`, straight from `thaw_bridge::
/// nested_namespace_members`) -- not necessarily the *real* generated
/// wrapper function that name ultimately dispatches through
/// (`dependency_exports`, built the same way `namespaces` resolves an
/// ordinary single-level member, applies a Fallback classification's own
/// package-prefixed rename, e.g. `zod___thaw_ns_coerce_number`). Re-keys
/// every member through `dependency_exports` the same way an ordinary
/// named import already does, falling back to the unresolved name only if
/// `dependency_exports` somehow doesn't have it (shouldn't happen for a
/// real registry package, but leaves a plain rename as harmless rather
/// than silently dropping the member).
fn resolve_nested_namespaces(
    nested: &HashMap<String, HashMap<String, String>>,
    dependency_exports: &HashMap<String, String>,
) -> HashMap<String, HashMap<String, String>> {
    nested
        .iter()
        .map(|(namespace, members)| {
            let resolved = members
                .iter()
                .map(|(member, target)| {
                    // `target` is the member's bare name, which normally
                    // matches a top-level flattened symbol (zod's own
                    // `coerce.number` -> the hoisted `number` function). A
                    // member declared *only* inside a nested namespace has no
                    // top-level declaration though (crypto-js's
                    // `enc.Hex`/`format.Hex` encoder/format consts), so fall
                    // back to the namespace-qualified `namespace.member`
                    // path the shim's value binding is keyed under --
                    // otherwise every such member resolved to the same bare
                    // name and collided.
                    let resolved_target =
                        dependency_exports.get(target).cloned().unwrap_or_else(|| {
                            let qualified = format!("{namespace}.{member}");
                            dependency_exports
                                .get(&qualified)
                                .cloned()
                                .unwrap_or(qualified)
                        });
                    (member.clone(), resolved_target)
                })
                .collect();
            (namespace.clone(), resolved)
        })
        .collect()
}

#[cfg(test)]
pub fn bundle(
    entry: &Path,
    entry_source: &str,
    external_exports: &HashMap<String, HashMap<String, String>>,
    external_namespace_aliases: &HashMap<String, HashSet<String>>,
    external_nested_namespaces: &HashMap<String, HashMap<String, HashMap<String, String>>>,
    external_resolutions: &HashMap<String, String>,
) -> Result<Module, String> {
    bundle_with_source_transform(
        entry,
        entry_source,
        external_exports,
        external_namespace_aliases,
        external_nested_namespaces,
        external_resolutions,
        &HashMap::new(),
        &HashMap::new(),
        &|source| Ok(source.to_string()),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn bundle_with_source_transform(
    entry: &Path,
    entry_source: &str,
    external_exports: &HashMap<String, HashMap<String, String>>,
    external_namespace_aliases: &HashMap<String, HashSet<String>>,
    external_nested_namespaces: &HashMap<String, HashMap<String, HashMap<String, String>>>,
    external_resolutions: &HashMap<String, String>,
    // `package -> the symbol its own `export = X;` assignment names`, so a
    // bare `import * as X from "pkg"` can bind `X` to that value (real
    // esModuleInterop: `import * as Koa from "koa"` -> `new Koa()`).
    external_export_assignments: &HashMap<String, String>,
    external_module_indices: &HashMap<String, usize>,
    transform: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Module, String> {
    let adapter = |source: &str, _path: &Path, _is_override: bool| transform(source);
    bundle_with_named_source_transform(
        entry, entry_source, external_exports, external_namespace_aliases,
        external_nested_namespaces, external_resolutions, external_export_assignments,
        external_module_indices, &adapter,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn bundle_with_named_source_transform(
    entry: &Path,
    entry_source: &str,
    external_exports: &HashMap<String, HashMap<String, String>>,
    external_namespace_aliases: &HashMap<String, HashSet<String>>,
    external_nested_namespaces: &HashMap<String, HashMap<String, HashMap<String, String>>>,
    external_resolutions: &HashMap<String, String>,
    external_export_assignments: &HashMap<String, String>,
    external_module_indices: &HashMap<String, usize>,
    transform: &dyn Fn(&str, &Path, bool) -> Result<String, String>,
) -> Result<Module, String> {
    let entry = entry
        .canonicalize()
        .map_err(|error| format!("failed to resolve `{}`: {error}", entry.display()))?;
    let mut modules = Vec::new();
    let mut loaded = HashMap::new();
    let entry_index = load_module(
        entry,
        Some(entry_source),
        Some(transform),
        &mut modules,
        &mut loaded,
        &mut HashSet::new(),
    )?;
    let eager_reachable = reachable_modules(&modules, entry_index, true);
    let emission_order = module_emission_order(&modules)?;
    if modules.len() >= EXTERNAL_MODULE_OFFSET {
        return Err("too many local modules for the generated module index space".into());
    }

    let mut exports: Vec<HashMap<String, String>> = vec![HashMap::new(); modules.len()];
    // Direct declarations have stable qualified symbols even when a dynamic
    // or type-only edge points back to an ancestor. Seed these before any
    // module's imports are resolved; sourced re-exports are finalized in the
    // static dependency order below.
    for (index, module) in modules.iter().enumerate() {
        let own = declared_names(&module.module).into_iter()
            .map(|name| {
                let symbol = if index == entry_index && matches!(name.as_str(), "main" | "handler") {
                    name.clone()
                } else { format!("__thawmod{index}_{name}") };
                (name, symbol)
            }).collect::<HashMap<_, _>>();
        for item in &module.module.body {
            match item {
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                    for name in declaration_names(&export.decl) {
                        if let Some(symbol) = own.get(&name) { exports[index].insert(name, symbol.clone()); }
                    }
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => {
                    let name = match &export.decl {
                        thaw_parser::ast::DefaultDecl::Fn(function) => function.ident.as_ref().map(|id| id.sym.as_ref()),
                        thaw_parser::ast::DefaultDecl::Class(class) => class.ident.as_ref().map(|id| id.sym.as_ref()),
                        thaw_parser::ast::DefaultDecl::TsInterfaceDecl(interface) => Some(interface.id.sym.as_ref()),
                    };
                    let symbol = name.and_then(|name| own.get(name)).cloned()
                        .unwrap_or_else(|| format!("__thawmod{index}_default"));
                    exports[index].insert("default".to_string(), symbol);
                }
                ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(export)) => {
                    let symbol = match export.expr.as_ref() {
                        Expr::Ident(name) => own.get(name.sym.as_ref()).cloned(),
                        _ => None,
                    }.unwrap_or_else(|| format!("__thawmod{index}_default_value"));
                    exports[index].insert("default".to_string(), symbol);
                }
                _ => {}
            }
        }
    }
    let directly_exported_names = exports.iter()
        .map(|module| module.keys().cloned().collect::<HashSet<_>>())
        .collect::<Vec<_>>();
    for _ in 0..modules.len() {
        let mut changed = false;
        for (index, module) in modules.iter().enumerate() {
            for item in &module.module.body {
                match item {
                    ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) => {
                        let Some(source) = &export.src else { continue; };
                        let Some(specifier) = source.value.as_str() else { continue; };
                        let source_exports = module.dependencies.get(specifier)
                            .map(|dependency| &exports[*dependency])
                            .or_else(|| external_exports.get(specifier));
                        let Some(source_exports) = source_exports.cloned() else { continue; };
                        for member in &export.specifiers {
                            if let thaw_parser::ast::ExportSpecifier::Named(member) = member {
                                let original = export_name(&member.orig)?;
                                let public = member.exported.as_ref().map(export_name).transpose()?
                                    .unwrap_or_else(|| original.clone());
                                if let Some(target) = source_exports.get(&original) {
                                    changed |= exports[index].insert(public, target.clone()).as_ref() != Some(target);
                                }
                            }
                        }
                    }
                    ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export)) => {
                        let Some(specifier) = export.src.value.as_str() else { continue; };
                        let source_exports = module.dependencies.get(specifier)
                            .map(|dependency| &exports[*dependency])
                            .or_else(|| external_exports.get(specifier));
                        let Some(source_exports) = source_exports.cloned() else { continue; };
                        for (name, target) in source_exports {
                            if name != "default" && !directly_exported_names[index].contains(&name) {
                                changed |= exports[index].insert(name, target.clone()).as_ref() != Some(&target);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if !changed { break; }
    }
    let mut namespace_exports: Vec<HashMap<String, HashMap<String, String>>> =
        vec![HashMap::new(); modules.len()];
    let mut ambiguous_exports: Vec<HashSet<String>> = vec![HashSet::new(); modules.len()];
    let mut type_only_export_names: Vec<HashSet<String>> = vec![HashSet::new(); modules.len()];
    let mut bundled_items = Vec::new();
    let mut type_only_targets = HashSet::new();
    let mut module_ranges = Vec::new();
    for index in emission_order {
        let is_entry = index == entry_index;
        let mut names = HashMap::new();
        let mut namespaces = HashMap::new();
        let mut nested_namespaces = HashMap::new();
        // Local binding names that come from an *external* (npm/registry)
        // import, as opposed to a local declaration or a relative-module
        // import. Only these are deliberately left unrenamed on an
        // `instanceof` right operand -- see `visit_mut_expr`'s own
        // `InstanceOf` handling.
        let mut external_bindings: HashSet<String> = HashSet::new();
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
            let declaration = match item {
                ModuleItem::Stmt(thaw_parser::ast::Stmt::Decl(declaration)) => Some(declaration),
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => Some(&export.decl),
                _ => None,
            };
            let name = match declaration {
                Some(Decl::TsInterface(interface)) => Some(interface.id.sym.as_ref()),
                Some(Decl::TsTypeAlias(alias)) => Some(alias.id.sym.as_ref()),
                _ => None,
            };
            if let Some(target) = name.and_then(|name| names.get(name)) {
                type_only_targets.insert(target.clone());
            }
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
                            let local = namespace.local.sym.to_string();
                            // A bare `import * as X from "pkg"` of an
                            // `export = X` package (real example: koa's own
                            // `import * as Koa from "koa"; new Koa()`)
                            // binds `X` to the exported *value itself*,
                            // matching real esModuleInterop. Binding it as
                            // a plain namespace object instead left
                            // `new X()` with nothing to construct. The
                            // namespace table is kept for member access
                            // (`X.staticThing`), keyed under both the local
                            // name and the renamed value symbol.
                            if let Some(value) = external_export_assignments.get(specifier) {
                                names.insert(local.clone(), value.clone());
                                namespaces.insert(value.clone(), dependency_exports.clone());
                            }
                            namespaces.insert(local.clone(), dependency_exports.clone());
                            if let Some(nested) = external_nested_namespaces.get(specifier) {
                                nested_namespaces.insert(
                                    local,
                                    resolve_nested_namespaces(nested, dependency_exports),
                                );
                            }
                            continue;
                        }
                    };
                    // A named import whose requested name is itself a
                    // synthetic nested-namespace alias from an external
                    // package (`thaw_bridge::nested_namespace_members`,
                    // e.g. winston's own `export import transports =
                    // Transports;`, later used as `transports.Console`)
                    // rather than a real top-level function/class -- bind
                    // it as a namespace over just that alias's own member
                    // table (not the whole package's `dependency_exports`,
                    // unlike a namespace import) so a later dotted access
                    // like `transports.Console` resolves the same way any
                    // other namespace-imported dotted access already does.
                    if !modules[index].dependencies.contains_key(specifier)
                        && !dependency_exports.contains_key(&requested)
                    {
                        if let Some(members) = external_nested_namespaces
                            .get(specifier)
                            .and_then(|nested| nested.get(&requested))
                        {
                            let resolved = members
                                .iter()
                                .map(|(member, target)| {
                                    let resolved_target = dependency_exports
                                        .get(target)
                                        .cloned()
                                        .unwrap_or_else(|| target.clone());
                                    (member.clone(), resolved_target)
                                })
                                .collect();
                            namespaces.insert(local, resolved);
                            continue;
                        }
                    }
                    if let Some(namespace) = modules[index]
                        .dependencies
                        .get(specifier)
                        .and_then(|dependency| namespace_exports[*dependency].get(&requested))
                    {
                        namespaces.insert(local, namespace.clone());
                        continue;
                    }
                    // Registry packages are loaded through CommonJS's
                    // `module.exports`. With synthetic-default-import
                    // semantics, a package that only declares named
                    // exports exposes that whole object as its default.
                    if is_external_default && !dependency_exports.contains_key("default") {
                        if let Some(nested) = external_nested_namespaces.get(specifier) {
                            nested_namespaces.insert(
                                local.clone(),
                                resolve_nested_namespaces(nested, dependency_exports),
                            );
                        }
                        namespaces.insert(local, dependency_exports.clone());
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
                        // A concrete export for `requested` (e.g. a
                        // synthetic `default` value for a typeless package
                        // -- `mustache`) wins over binding the whole
                        // (possibly empty) export table as a namespace.
                        && !dependency_exports.contains_key(&requested)
                        && external_namespace_aliases
                            .get(specifier)
                            .is_some_and(|aliases| aliases.contains(&requested))
                    {
                        if let Some(nested) = external_nested_namespaces.get(specifier) {
                            nested_namespaces.insert(
                                local.clone(),
                                resolve_nested_namespaces(nested, dependency_exports),
                            );
                        }
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
                    if !modules[index].dependencies.contains_key(specifier) {
                        external_bindings.insert(local.clone());
                    }
                    // A named export that is *also* a namespace merged
                    // onto its own binding (real example: marked's
                    // exported `marked` function, merged with
                    // `declare namespace marked { var parse; let use;
                    // }`) must resolve `.member` accesses through the
                    // package's own export table as well as being
                    // directly callable -- unlike a pure namespace
                    // import, it stays in `names` above. The member
                    // table is keyed by the renamed `target` too,
                    // because `RenameReferences` renames the member's
                    // object identifier before resolving the access.
                    if let Some(members) = external_nested_namespaces
                        .get(specifier)
                        .and_then(|nested| nested.get(&requested))
                    {
                        let resolved = members
                            .iter()
                            .map(|(member, member_target)| {
                                let resolved_target = dependency_exports
                                    .get(member_target)
                                    .cloned()
                                    .unwrap_or_else(|| member_target.clone());
                                (member.clone(), resolved_target)
                            })
                            .collect::<HashMap<_, _>>();
                        namespaces.insert(target.clone(), resolved.clone());
                        namespaces.entry(local.clone()).or_insert(resolved);
                    }
                    if is_external_default {
                        if let Some(nested) = external_nested_namespaces.get(specifier) {
                            let resolved = resolve_nested_namespaces(nested, dependency_exports);
                            nested_namespaces.insert(local.clone(), resolved.clone());
                            nested_namespaces.insert(target.clone(), resolved);
                        }
                        namespaces.insert(local, dependency_exports.clone());
                        namespaces.insert(target.clone(), dependency_exports.clone());
                    }
                }
            }
        }

        // Local paths use resolved module indices; registry packages use the
        // separately generated initializer indices supplied by the build.
        // Leave import() calls intact through identifier/export resolution.
        // A cycle may point to a module whose re-export table is finalized
        // later; the postpass below rewrites calls after all tables exist.
        let dynamic_import_exports: HashMap<String, usize> = HashMap::new();

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
                        nested_namespaces: &nested_namespaces,
                        dynamic_imports: &dynamic_import_exports,
                        external_bindings: &external_bindings,
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
                        nested_namespaces: &nested_namespaces,
                        dynamic_imports: &dynamic_import_exports,
                        external_bindings: &external_bindings,
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
                    let export_type_only = export.type_only;
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
                                let named_type_only = named.is_type_only;
                                let original = export_name(&named.orig)?;
                                let exported = named
                                    .exported
                                    .as_ref()
                                    .map(export_name)
                                    .transpose()?
                                    .unwrap_or_else(|| original.clone());
                                if export_type_only || named_type_only {
                                    type_only_export_names[index].insert(exported.clone());
                                }
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
                                    .map(|source| source.get(&original))
                                    .unwrap_or_else(|| names.get(&original))
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
                                nested_namespaces: &nested_namespaces,
                                dynamic_imports: &dynamic_import_exports,
                                external_bindings: &external_bindings,
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
                                nested_namespaces: &nested_namespaces,
                                dynamic_imports: &dynamic_import_exports,
                                external_bindings: &external_bindings,
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
                                nested_namespaces: &nested_namespaces,
                                dynamic_imports: &dynamic_import_exports,
                                external_bindings: &external_bindings,
                                import_meta_url: &import_meta_url,
                                import_meta_main: is_entry,
                                module_path: &modules[index].path,
                                external_resolutions,
                                shadowed: HashSet::new(),
                                function_depth: 0,
                            });
                            public.insert("default".to_string(), interface.id.sym.to_string());
                            type_only_export_names[index].insert("default".to_string());
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
                        nested_namespaces: &nested_namespaces,
                        dynamic_imports: &dynamic_import_exports,
                        external_bindings: &external_bindings,
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
                            if export.type_only {
                                type_only_export_names[index].insert(name.clone());
                            }
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
        // The marker survives HIR normalization and identifies the owner of
        // each source-ordered initialization step. A module reached only for
        // types retains its declarations without startup effects; if a
        // compiled function later imports it as a value, the guard can still
        // initialize it. Dynamic-only groups run on first import.
        let mut static_dependencies = modules[index]
            .static_dependencies
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for (specifier, _, is_static) in dependency_specifiers(&modules[index].module)? {
            if is_static {
                if let Some(external) = external_module_indices.get(&specifier) {
                    static_dependencies.push(*external);
                }
            }
        }
        static_dependencies.sort_unstable();
        static_dependencies.dedup();
        let marker = format!(
            "__thaw_internal_module:{}:{}:{}:{}",
            index,
            usize::from(eager_reachable.contains(&index)),
            1,
            static_dependencies.iter().map(usize::to_string).collect::<Vec<_>>().join(",")
        );
        bundled_items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Expr(
            thaw_parser::ast::ExprStmt {
                span: Default::default(),
                expr: Box::new(Expr::Lit(thaw_parser::ast::Lit::Str(thaw_parser::ast::Str {
                    span: Default::default(),
                    value: marker.into(),
                    raw: None,
                }))),
            },
        )));
        let start = bundled_items.len();
        bundled_items.extend(items);
        module_ranges.push((index, start, bundled_items.len()));
    }

    for (index, start, end) in module_ranges {
        let mut targets = modules[index].dependencies.clone();
        for (specifier, external) in external_module_indices {
            if external_exports.contains_key(specifier) {
                targets.insert(specifier.clone(), *external);
            }
        }
        let empty_names = HashMap::new();
        let empty_namespaces = HashMap::new();
        let empty_nested = HashMap::new();
        let empty_external = HashSet::new();
        let mut rewriter = RenameReferences {
            names: &empty_names,
            namespaces: &empty_namespaces,
            nested_namespaces: &empty_nested,
            dynamic_imports: &targets,
            external_bindings: &empty_external,
            import_meta_url: "",
            import_meta_main: index == entry_index,
            module_path: &modules[index].path,
            external_resolutions,
            shadowed: HashSet::new(),
            function_depth: 0,
        };
        for item in &mut bundled_items[start..end] {
            item.visit_mut_with(&mut rewriter);
        }
    }

    // Allocate namespace objects once during startup. Without their own
    // boundary these declarations would become part of the last (possibly
    // dynamic-only) module's initializer, leaving another import's namespace
    // uninitialized. Promise continuations run after startup initialization.
    let namespace_group = modules.len();
    bundled_items.extend(thaw_parser::parse_typescript_with_source_map_named(
        &format!("function __thaw_lazy_module_{namespace_group}(): void {{}}"),
        thaw_parser::common::FileName::Custom("generated namespace initializer.ts".into()),
    )?.0.body);
    bundled_items.push(ModuleItem::Stmt(thaw_parser::ast::Stmt::Expr(
        thaw_parser::ast::ExprStmt {
            span: Default::default(),
            expr: Box::new(Expr::Lit(thaw_parser::ast::Lit::Str(thaw_parser::ast::Str {
                span: Default::default(),
                value: format!("__thaw_internal_module:{namespace_group}:1:1:").into(),
                raw: None,
            }))),
        },
    )));
    // Each property reads the export's current binding, preserving live
    // namespace values across repeated imports.
    for index in 0..modules.len() {
        let mut getters = exports[index]
            .iter()
            .filter(|(name, target)| !type_only_export_names[index].contains(*name)
                && !type_only_targets.contains(*target) && !target.starts_with("__thaw_type_"))
            .map(|(name, target)| {
                let key = serde_json::to_string(name).map_err(|error| error.to_string())?;
                Ok(format!("get {key}() {{ return {target}; }}"))
            })
            .collect::<Result<Vec<_>, String>>()?;
        for (name, members) in &namespace_exports[index] {
            let key = serde_json::to_string(name).map_err(|error| error.to_string())?;
            let mut member_getters = members.iter()
                .filter(|(_, target)| !type_only_targets.contains(*target) && !target.starts_with("__thaw_type_"))
                .map(|(member, target)| {
                    let member = serde_json::to_string(member).map_err(|error| error.to_string())?;
                    Ok(format!("get {member}() {{ return {target}; }}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            member_getters.sort();
            getters.push(format!("{key}: {{{}}}", member_getters.join(",")));
        }
        getters.sort();
        let snippet = format!(
            "function __thaw_lazy_module_{index}(): void {{}}\nconst __thaw_namespace_{index} = {{{}}};",
            getters.join(",")
        );
        let generated = thaw_parser::parse_typescript_with_source_map_named(
            &snippet, thaw_parser::common::FileName::Custom("generated namespace object.ts".into()),
        )?.0;
        bundled_items.extend(generated.body);
    }
    for (specifier, index) in external_module_indices {
        let Some(export_map) = external_exports.get(specifier) else { continue; };
        let mut getters = export_map.iter()
            .filter(|(_, target)| !target.starts_with("__thaw_type_"))
            .map(|(name, target)| {
                let key = serde_json::to_string(name).map_err(|error| error.to_string())?;
                Ok(format!("get {key}() {{ return {target}; }}"))
            })
            .collect::<Result<Vec<_>, String>>()?;
        getters.sort();
        let generated = thaw_parser::parse_typescript_with_source_map_named(
            &format!("const __thaw_namespace_{index} = {{{}}};", getters.join(",")),
            thaw_parser::common::FileName::Custom("generated external namespace object.ts".into()),
        )?.0;
        bundled_items.extend(generated.body);
    }

    Ok(Module {
        span: modules[entry_index].module.span,
        body: bundled_items,
        shebang: None,
    })
}
