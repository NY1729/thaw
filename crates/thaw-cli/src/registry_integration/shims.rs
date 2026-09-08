/// One `--use`d package, resolved and classified, but with no shim text
/// generated yet -- collision resolution (see `generate_registry_shims`)
/// needs to see every package's declared names *before* deciding how any
/// individual one should be rendered, so this is deliberately kept as an
/// intermediate step rather than folded into one pass.
struct ResolvedPackage {
    name: String,
    commonjs_export_name: Option<String>,
    called_commonjs_namespace_properties: std::collections::HashSet<String>,
    functions: Vec<thaw_bridge::DtsFunction>,
    values: Vec<thaw_bridge::DtsValue>,
    classes: Vec<thaw_bridge::DtsClass>,
    classifications: Vec<(String, thaw_bridge::Classification)>,
    native_lib: Option<PathBuf>,
    native_addon: Option<PathBuf>,
    native_dependencies: Vec<PathBuf>,
    bundle_js: Option<String>,
    /// `(factory function name) -> (this package's own class name)` for
    /// every function whose declared return type names one of `classes`
    /// -- real example: dayjs's `declare function dayjs(...):
    /// dayjs.Dayjs`, linking factory function `dayjs` to class `Dayjs`.
    /// Lets a Fallback factory call be tracked as producing a class
    /// instance the same way `new ClassName(...)` already is (see
    /// `class_methods.rs`'s `constructed_class`), even though nothing
    /// about the call itself says `new`.
    factory_class_returns: std::collections::HashMap<String, String>,
    /// Every name this package's own `.d.ts` re-exports as a self-
    /// referential namespace alias for its own already-flattened export
    /// table -- see `thaw_bridge::self_referential_namespace_aliases`'s
    /// own doc comment. Real example: zod's own `z` (and `default`),
    /// letting `import { z } from "zod"` resolve the same way `import *
    /// as z from "zod"` already does, rather than failing with "`zod`
    /// has no export named `z`" (neither is a function/class/interface
    /// at all -- both are bound purely by an `import *`).
    namespace_self_aliases: std::collections::HashSet<String>,
    type_only_exports: std::collections::HashSet<String>,
    /// Every `NAME -> { member name -> real flattened function name }`
    /// nested-namespace table this package's own `.d.ts` declares -- see
    /// `thaw_bridge::nested_namespace_members`'s own doc comment. Real
    /// example: zod's `coerce` (and `core`, `iso`), letting `z.coerce.
    /// number(...)` resolve straight to the flattened function it
    /// actually names, one level down from `z` itself.
    nested_namespaces: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

enum JitExport {
    Value(String),
    Null,
    Undefined,
    Object(Vec<(String, JitExport)>),
    Dictionary(Vec<JitDictionaryEntry>),
    Tuple(Vec<JitExport>),
    Conditional(String, Box<JitExport>, Box<JitExport>),
    WithLocals(Vec<JitLocal>, Box<JitExport>),
}

enum JitDictionaryEntry {
    Static(String, JitExport),
    Computed(JitExport, JitExport),
    Spread(JitExport),
}

struct JitLocal {
    ty: thaw_hir::HirType,
    operation: String,
}

fn jit_tagged_union(elements: &[thaw_hir::HirType]) -> bool {
    jit_argument_tagged_union(elements)
}

fn jit_argument_tagged_union(elements: &[thaw_hir::HirType]) -> bool {
    (2..=9).contains(&elements.len())
        && elements.iter().all(|element| jit_union_member_code(element).is_some())
        && elements
            .iter()
            .enumerate()
            .all(|(index, element)| !elements[..index].contains(element))
        && elements
            .iter()
            .filter(|element| matches!(element, thaw_hir::HirType::Object(_)))
            .count()
            <= 1
        && elements
            .iter()
            .filter(|element| matches!(element, thaw_hir::HirType::Tuple(_)))
            .count()
            <= 1
        && (elements.contains(&thaw_hir::HirType::Str)
            || elements.iter().any(|element| {
                matches!(
                    element,
                    thaw_hir::HirType::Array(_)
                        | thaw_hir::HirType::Dictionary(_)
                        | thaw_hir::HirType::Object(_)
                        | thaw_hir::HirType::Tuple(_)
                )
            }))
}

fn jit_union_member_code(ty: &thaw_hir::HirType) -> Option<char> {
    match ty {
        thaw_hir::HirType::F64 => Some('n'),
        thaw_hir::HirType::Bool => Some('b'),
        thaw_hir::HirType::Str => Some('s'),
        thaw_hir::HirType::Array(element) => match element.as_ref() {
            thaw_hir::HirType::F64 => Some('N'),
            thaw_hir::HirType::Bool => Some('B'),
            thaw_hir::HirType::Str => Some('S'),
            _ => None,
        },
        thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
            thaw_hir::HirType::F64 => Some('D'),
            thaw_hir::HirType::Bool => Some('E'),
            thaw_hir::HirType::Str => Some('F'),
            _ => None,
        },
        thaw_hir::HirType::Object(_) => Some('O'),
        thaw_hir::HirType::Tuple(elements) => {
            if !elements.is_empty() && elements.iter().all(|element| *element == thaw_hir::HirType::F64) {
                Some('X')
            } else if !elements.is_empty()
                && elements.iter().all(|element| *element == thaw_hir::HirType::Bool)
            {
                Some('Y')
            } else if !elements.is_empty()
                && elements.iter().all(|element| *element == thaw_hir::HirType::Str)
            {
                Some('Z')
            } else {
                Some('T')
            }
        }
        _ => None,
    }
}

fn is_unary_math_method(operation: &str) -> bool {
    matches!(
        operation,
        "abs"
            | "acos"
            | "acosh"
            | "asin"
            | "asinh"
            | "atan"
            | "atanh"
            | "cbrt"
            | "ceil"
            | "clz32"
            | "cos"
            | "cosh"
            | "exp"
            | "expm1"
            | "floor"
            | "fround"
            | "log"
            | "log1p"
            | "log2"
            | "log10"
            | "round"
            | "sign"
            | "sin"
            | "sinh"
            | "sqrt"
            | "tan"
            | "tanh"
            | "trunc"
    )
}

include!("jit/aggregates.rs");
include!("jit/returns.rs");
include!("jit/expressions.rs");
include!("jit/conditions.rs");
include!("jit/control_flow.rs");
include!("jit/loop_control.rs");
include!("jit/loop_expressions.rs");
include!("jit/loop_analysis.rs");
include!("jit/loop_bodies.rs");
include!("jit/loop_aliases.rs");
include!("jit/callables.rs");

fn jit_export(
    source: &str,
    export_name: &str,
    allow_default: bool,
    function: &thaw_bridge::DtsFunction,
) -> Option<JitExport> {
    use thaw_parser::ast::{
        ArrowExpr, AssignExpr, AssignOp, AssignTarget, BinaryOp, CallExpr, Callee, Decl, Expr,
        ExprOrSpread, ForHead, Function, Ident, Lit, MemberProp, ModuleItem, OptChainBase, Pat,
        ObjectPatProp, Prop, PropName, PropOrSpread, SimpleAssignTarget, Stmt, UnaryOp, UpdateOp,
        VarDeclKind, VarDeclOrExpr,
    };

    jit_aggregates!();
    jit_returns!();
    jit_expressions!();
    jit_conditions!();
    jit_control_flow!();
    jit_loop_control!();
    jit_loop_expressions!();
    jit_loop_analysis!();
    jit_loop_bodies!();
    jit_loop_aliases!();
    jit_callables!();

    if function.generic.is_some()
        || function.rest_param.is_some()
        || !function
            .params
            .iter()
            .all(|(_, ty)| {
                let thaw_bridge::DtsType::Native(ty) = ty else {
                    return false;
                };
                let ty = match ty {
                    thaw_hir::HirType::Optional(payload) => payload.as_ref(),
                    ty => ty,
                };
                jit_parameter_slots(ty).is_some()
            })
        || function
            .params
            .iter()
            .try_fold(0usize, |slots, (_, ty)| match ty {
                thaw_bridge::DtsType::Native(ty) => {
                    jit_parameter_slots(ty).map(|count| slots + count)
                }
                thaw_bridge::DtsType::Unsupported(_) => None,
            })
            .is_none_or(|slots| slots > 16)
        || !match &function.ret {
            thaw_bridge::DtsType::Native(
                thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str,
            ) => true,
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(elements)) => {
                jit_tagged_union(elements)
            }
            thaw_bridge::DtsType::Native(
                thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload),
            ) => {
                jit_result_supported(payload)
                    && matches!(
                        payload.as_ref(),
                        thaw_hir::HirType::F64
                            | thaw_hir::HirType::Bool
                            | thaw_hir::HirType::Str
                            | thaw_hir::HirType::Union(_)
                            | thaw_hir::HirType::Array(_)
                            | thaw_hir::HirType::Dictionary(_)
                            | thaw_hir::HirType::Object(_)
                            | thaw_hir::HirType::Tuple(_)
                    )
            }
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element)) => {
                jit_array_result_element_supported(element)
            }
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Dictionary(element)) => matches!(
                element.as_ref(),
                thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str
            ),
            thaw_bridge::DtsType::Native(
                ty @ (thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)),
            ) => jit_result_supported(ty),
            _ => false,
        }
    {
        return None;
    }

    let module = thaw_parser::parse_javascript(source).ok()?;
    let mut module_functions = std::collections::HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))) = item {
            if module_functions
                .insert(
                    declaration.ident.sym.to_string(),
                    NumericCallable::Function(declaration.function.as_ref()),
                )
                .is_some()
            {
                return None;
            }
        }
    }
    if module_functions.contains_key("String")
        || module_functions.contains_key("Number")
        || module_functions.contains_key("Boolean")
    {
        return None;
    }
    let mut style = None;
    let mut callable = None;
    let mut module_locals = std::collections::HashMap::new();
    let mut module_mutable = std::collections::HashSet::new();
    let mut module_callable_mutable = std::collections::HashSet::new();
    let mut module_callable_tables = std::collections::HashMap::new();
    let mut module_callable_table_mutable = std::collections::HashSet::new();
    let no_parameters = std::collections::HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(statement) = item else {
            return None;
        };
        if matches!(statement, Stmt::Decl(Decl::Fn(_))) {
            continue;
        }
        if let Stmt::Decl(Decl::Var(declaration)) = statement {
            for declarator in &declaration.decls {
                let Pat::Ident(name) = &declarator.name else {
                    return None;
                };
                if module_locals.contains_key(name.id.sym.as_ref())
                    || module_functions.contains_key(name.id.sym.as_ref())
                    || module_callable_tables.contains_key(name.id.sym.as_ref())
                {
                    return None;
                }
                let initializer = declarator.init.as_deref()?;
                if let Expr::Object(object) = initializer {
                    if let Some(table) = resolve_callable_table(object, &module_functions) {
                        module_callable_tables.insert(name.id.sym.to_string(), table);
                        if declaration.kind != VarDeclKind::Const {
                            module_callable_table_mutable.insert(name.id.sym.to_string());
                        }
                        continue;
                    }
                }
                if let Some(callable) = resolve_callable(initializer, &module_functions) {
                    module_functions.insert(name.id.sym.to_string(), callable);
                    if declaration.kind != VarDeclKind::Const {
                        module_callable_mutable.insert(name.id.sym.to_string());
                    }
                    continue;
                }
                let mut encoded = Vec::new();
                let empty_helpers = std::collections::HashMap::new();
                let mut context = InlineContext {
                    helpers: &empty_helpers,
                    module_locals: module_locals.clone(),
                    module_globals: std::collections::HashMap::new(),
                    callable_tables: module_callable_tables.clone(),
                    callable_table_states: std::collections::HashMap::new(),
                    dynamic_callable_tables: std::collections::HashMap::new(),
                    active: Vec::new(),
                    recursive_names: Vec::new(),
                    recursive_parameters: Vec::new(),
                    recursive_result: None,
                    loop_depth: 0,
                    loop_labels: Vec::new(),
                };
                encode_expression(
                    initializer,
                    &no_parameters,
                    &module_locals,
                    &mut context,
                    &mut encoded,
                )?;
                if declaration.kind != VarDeclKind::Const && !stable_jit_tokens(&encoded) {
                    return None;
                }
                module_locals.insert(name.id.sym.to_string(), encoded);
                if declaration.kind != VarDeclKind::Const {
                    module_mutable.insert(name.id.sym.to_string());
                }
            }
            continue;
        }
        let Stmt::Expr(statement) = statement else {
            return None;
        };
        if matches!(statement.expr.as_ref(), Expr::Lit(Lit::Str(_))) {
            continue;
        }
        let Expr::Assign(assignment) = statement.expr.as_ref() else {
            if let Expr::Update(update) = statement.expr.as_ref() {
                if let Expr::Ident(name) = update.arg.as_ref() {
                    if module_mutable.contains(name.sym.as_ref()) {
                        let mut encoded = module_locals.get(name.sym.as_ref())?.clone();
                        encoded.extend([
                            format!("c{:016x}", 1.0_f64.to_bits()),
                            match update.op {
                                UpdateOp::PlusPlus => "+".into(),
                                UpdateOp::MinusMinus => "-".into(),
                            },
                        ]);
                        module_locals.insert(name.sym.to_string(), encoded);
                        continue;
                    }
                }
            }
            return None;
        };
        if let AssignTarget::Simple(SimpleAssignTarget::Ident(name)) = &assignment.left {
            if module_callable_table_mutable.contains(name.id.sym.as_ref()) {
                if assignment.op != AssignOp::Assign {
                    return None;
                }
                let Expr::Object(object) = assignment.right.as_ref() else {
                    return None;
                };
                let table = resolve_callable_table(object, &module_functions)?;
                module_callable_tables.insert(name.id.sym.to_string(), table);
                continue;
            }
            if module_callable_mutable.contains(name.id.sym.as_ref()) {
                if assignment.op != AssignOp::Assign {
                    return None;
                }
                let callable = resolve_callable(assignment.right.as_ref(), &module_functions)?;
                module_functions.insert(name.id.sym.to_string(), callable);
                continue;
            }
            if !module_mutable.contains(name.id.sym.as_ref()) {
                return None;
            }
            let mut encoded = Vec::new();
            if assignment.op == AssignOp::AddAssign {
                let mut right = Vec::new();
                encode_expression(
                    assignment.right.as_ref(),
                    &no_parameters,
                    &module_locals,
                    &mut InlineContext {
                        helpers: &module_functions,
                        module_locals: module_locals.clone(),
                        module_globals: std::collections::HashMap::new(),
                        callable_tables: module_callable_tables.clone(),
                        callable_table_states: std::collections::HashMap::new(),
                        dynamic_callable_tables: std::collections::HashMap::new(),
                        active: Vec::new(),
                        recursive_names: Vec::new(),
                        recursive_parameters: Vec::new(),
                        recursive_result: None,
                        loop_depth: 0,
                        loop_labels: Vec::new(),
                    },
                    &mut right,
                )?;
                append_add(
                    module_locals.get(name.id.sym.as_ref())?.clone(),
                    right,
                    &mut encoded,
                )?;
            } else {
                if assignment.op != AssignOp::Assign {
                    encoded.extend(
                        module_locals
                            .get(name.id.sym.as_ref())?
                            .iter()
                            .cloned(),
                    );
                }
                let mut context = InlineContext {
                    helpers: &module_functions,
                    module_locals: module_locals.clone(),
                    module_globals: std::collections::HashMap::new(),
                    callable_tables: module_callable_tables.clone(),
                    callable_table_states: std::collections::HashMap::new(),
                    dynamic_callable_tables: std::collections::HashMap::new(),
                    active: Vec::new(),
                    recursive_names: Vec::new(),
                    recursive_parameters: Vec::new(),
                    recursive_result: None,
                    loop_depth: 0,
                    loop_labels: Vec::new(),
                };
                encode_expression(
                    assignment.right.as_ref(),
                    &no_parameters,
                    &module_locals,
                    &mut context,
                    &mut encoded,
                )?;
                if assignment.op != AssignOp::Assign {
                    encoded.push(
                        match assignment.op {
                            AssignOp::SubAssign => "-",
                            AssignOp::MulAssign => "*",
                            AssignOp::DivAssign => "/",
                            AssignOp::ModAssign => "%",
                            AssignOp::LShiftAssign => "shl",
                            AssignOp::RShiftAssign => "shr",
                            AssignOp::ZeroFillRShiftAssign => "ushr",
                            AssignOp::BitOrAssign => "bor",
                            AssignOp::BitXorAssign => "bxor",
                            AssignOp::BitAndAssign => "band",
                            AssignOp::ExpAssign => "pow",
                            _ => return None,
                        }
                        .into(),
                    );
                }
            }
            if !stable_jit_tokens(&encoded) {
                return None;
            }
            module_locals.insert(name.id.sym.to_string(), encoded);
            continue;
        }
        if let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left {
            if let Expr::Ident(table) = target.obj.as_ref() {
                if let Some(entries) = module_callable_tables.get_mut(table.sym.as_ref()) {
                    if assignment.op != AssignOp::Assign {
                        return None;
                    }
                    let key = match &target.prop {
                        MemberProp::Ident(property) => property.sym.to_string(),
                        MemberProp::Computed(property) => {
                            match property.expr.as_ref() {
                                Expr::Lit(Lit::Str(key)) => {
                                    key.value.to_string_lossy().into_owned()
                                }
                                Expr::Ident(key) => encoded_string_literal(
                                    module_locals.get(key.sym.as_ref())?,
                                )?,
                                _ => return None,
                            }
                        }
                        MemberProp::PrivateName(_) => return None,
                    };
                    let helper = match assignment.right.as_ref() {
                        Expr::Ident(helper) if module_functions.contains_key(helper.sym.as_ref()) => {
                            helper.sym.to_string()
                        }
                        _ => return None,
                    };
                    if let Some((_, value)) = entries.iter_mut().find(|(name, _)| name == &key) {
                        *value = helper;
                    } else {
                        entries.push((key, helper));
                    }
                    continue;
                }
            }
        }
        let (assignment_style, selected) =
            exported_callable(assignment, export_name, allow_default, &module_functions)?;
        if style.replace(assignment_style).is_some_and(|style| {
            style != assignment_style || assignment_style == ExportStyle::Whole
        }) {
            return None;
        }
        if let Some(selected) = selected {
            if callable.replace(selected).is_some() {
                return None;
            }
        }
    }
    let callable = callable?;

    let (params, local_steps, body) = callable_parts(callable)?;
    if params.len() != function.params.len() {
        return None;
    }
    let mut bindings = Vec::with_capacity(params.len());
    let mut names = std::collections::HashSet::new();
    for (index, (parameter, (_, ty))) in params.iter().zip(&function.params).enumerate() {
        let (parameter, default) = match parameter {
            Pat::Ident(parameter) => (parameter, None),
            Pat::Assign(assignment) => {
                let Pat::Ident(parameter) = assignment.left.as_ref() else {
                    return None;
                };
                (parameter, Some(assignment.right.as_ref()))
            }
            _ => return None,
        };
        let name = parameter.id.sym.to_string();
        if !names.insert(name.clone()) {
            return None;
        }
        let thaw_bridge::DtsType::Native(ty) = ty else {
            return None;
        };
        bindings.push((name, ty, default, index >= function.required_params));
    }
    let mut expression = Vec::new();
    let mut module_globals = std::collections::HashMap::new();
    let stateful_module_names = module_functions
        .values()
        .filter_map(|callable| callable_parts(*callable))
        .flat_map(|(_, steps, _)| steps)
        .filter_map(|step| match step {
            LocalStep::Assign { name, .. } | LocalStep::Update { name, .. }
                if module_mutable.contains(name.sym.as_ref()) =>
            {
                Some(name.sym.to_string())
            }
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    let mut global_names = stateful_module_names
        .iter()
        .filter_map(|name| {
            let initial = module_locals.get(name)?;
            (jit_expression_kind(initial)?.0 == JitKind::Number).then(|| name.clone())
        })
        .collect::<Vec<_>>();
    global_names.sort();
    global_names.dedup();
    if global_names.len() > 16 {
        return None;
    }
    let mut next_global_slot = global_names.len();
    for (slot, name) in global_names.into_iter().enumerate() {
        let initial = module_locals.get(&name)?.clone();
        let mut read = vec![format!("c{:016x}", (slot as f64).to_bits())];
        read.extend(initial);
        read.push("globalinit".into());
        module_locals.insert(name.clone(), read);
        module_globals.insert(name, slot as u8);
    }
    let mut table_candidates = std::collections::BTreeMap::<
        (String, String),
        std::collections::BTreeSet<String>,
    >::new();
    let mut assignments = Vec::new();
    for (_, steps, body) in module_functions
        .values()
        .filter_map(|callable| callable_parts(*callable))
    {
        for step in steps {
            if let LocalStep::Effect(expression) = step {
                if let Some(assignment) = callable_table_assignment(expression) {
                    assignments.push(assignment);
                }
            }
        }
        if let NumericBody::Statements(statements) = body {
            for statement in statements {
                collect_callable_table_assignments(statement, &mut assignments);
            }
        }
    }
    let dynamic_table_names = assignments
        .iter()
        .filter_map(|(table, keys, _)| keys.is_none().then_some(table.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut dynamic_table_candidates = std::collections::BTreeMap::<
        String,
        std::collections::BTreeSet<String>,
    >::new();
    for (table, keys, helpers) in assignments {
        if helpers
            .iter()
            .any(|helper| !module_functions.contains_key(helper))
            || !module_callable_tables.contains_key(&table)
        {
            continue;
        }
        if dynamic_table_names.contains(&table) {
            dynamic_table_candidates
                .entry(table)
                .or_default()
                .extend(helpers);
            continue;
        }
        for key in keys? {
                if !module_callable_tables
                    .get(&table)
                    .is_some_and(|entries| entries.iter().any(|(name, _)| name == &key))
                {
                    continue;
                }
                table_candidates
                    .entry((table.clone(), key))
                    .or_default()
                    .extend(helpers.iter().cloned());
        }
    }
    for table in &dynamic_table_names {
        let Some(entries) = module_callable_tables.get(table) else {
            continue;
        };
        let helpers = entries.iter().map(|(_, helper)| helper.clone());
        dynamic_table_candidates
            .entry(table.clone())
            .or_default()
            .extend(helpers);
    }
    if next_global_slot + table_candidates.len() > 16 {
        return None;
    }
    let mut callable_table_states = std::collections::HashMap::<
        String,
        std::collections::HashMap<String, (u8, Vec<String>)>,
    >::new();
    for ((table, key), candidates) in table_candidates {
        let initial = module_callable_tables
            .get(&table)?
            .iter()
            .find(|(name, _)| name == &key)?
            .1
            .clone();
        let mut helpers = vec![initial.clone()];
        helpers.extend(candidates.into_iter().filter(|helper| helper != &initial));
        callable_table_states
            .entry(table)
            .or_default()
            .insert(key, (next_global_slot as u8, helpers));
        next_global_slot += 1;
    }
    let dynamic_callable_tables = dynamic_table_candidates
        .into_iter()
        .enumerate()
        .map(|(table, (name, helpers))| {
            Some((
                name,
                (u8::try_from(table).ok()?, helpers.into_iter().collect()),
            ))
        })
        .collect::<Option<std::collections::HashMap<_, _>>>()?;
    let helper_module_locals = module_locals.clone();
    let helper_callable_tables = module_callable_tables.clone();
    let mut locals = module_locals;
    for (parameter, _, _, _) in &bindings {
        locals.remove(parameter);
    }
    let recursive_parameters = bindings
        .iter()
        .try_fold((0usize, Vec::new()), |(slots, mut types), (_, ty, default, optional)| {
            if default.is_some() || *optional {
                return None;
            }
            let count = jit_parameter_slots(ty)?;
            if slots + count > 8 {
                return None;
            }
            types.push((*ty).clone());
            Some((slots + count, types))
        })
        .map(|(_, types)| types)
        .unwrap_or_default();
    let recursive_result = match &function.ret {
        thaw_bridge::DtsType::Native(thaw_hir::HirType::F64) => Some(JitKind::Number),
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool) => Some(JitKind::Boolean),
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Str) => Some(JitKind::String),
        _ => None,
    };
    let recursive_names = if !params.is_empty()
        && recursive_parameters.len() == params.len()
        && recursive_result.is_some()
    {
        module_functions
            .iter()
            .filter(|(_, candidate)| same_callable(**candidate, callable))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut context = InlineContext {
        helpers: &module_functions,
        module_locals: helper_module_locals,
        module_globals,
        callable_tables: helper_callable_tables,
        callable_table_states,
        dynamic_callable_tables,
        active: Vec::new(),
        recursive_names,
        recursive_parameters,
        recursive_result,
        loop_depth: 0,
        loop_labels: Vec::new(),
    };
    let mut parameters = std::collections::HashMap::new();
    let mut slot = 0usize;
    for (parameter, ty, default, optional_parameter) in &bindings {
        if let thaw_hir::HirType::Nullable(payload) | thaw_hir::HirType::Nullish(payload) = ty {
            if default.is_some() || *optional_parameter {
                return None;
            }
            if matches!(
                payload.as_ref(),
                thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)
            ) {
                let presence = if matches!(ty, thaw_hir::HirType::Nullable(_)) {
                    format!("a{slot}")
                } else {
                    format!("a{slot},c0000000000000000,==")
                };
                slot += 1;
                bind_jit_aggregate_fields(parameter, payload, &mut parameters, &mut slot)?;
                parameters.insert(
                    parameter.clone(),
                    format!("optional:{presence}:aggregate"),
                );
                continue;
            }
            let (prefix, null, absent) = match payload.as_ref() {
                thaw_hir::HirType::F64 => ("a", "nulln", "absentn"),
                thaw_hir::HirType::Bool => ("b", "nullb", "absentb"),
                thaw_hir::HirType::Str => ("s", "nulls", "absents"),
                thaw_hir::HirType::Array(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => ("rn", "nulla", "absenta"),
                    thaw_hir::HirType::Bool => ("rb", "nulla", "absenta"),
                    thaw_hir::HirType::Str => ("rs", "nulla", "absenta"),
                    _ => return None,
                },
                thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                    thaw_hir::HirType::F64 => ("dn", "nulld", "absentd"),
                    thaw_hir::HirType::Bool => ("db", "nulld", "absentd"),
                    thaw_hir::HirType::Str => ("ds", "nulld", "absentd"),
                    _ => return None,
                },
                _ => return None,
            };
            let tag = format!("a{slot}");
            let value = format!("{prefix}{}", slot + 1);
            let selected = if matches!(ty, thaw_hir::HirType::Nullable(_)) {
                vec![
                    tag,
                    "asbool".into(),
                    "if".into(),
                    value,
                    "else".into(),
                    null.into(),
                    "end".into(),
                ]
            } else {
                vec![
                    tag.clone(),
                    "c0000000000000000".into(),
                    "==".into(),
                    "if".into(),
                    value,
                    "else".into(),
                    tag,
                    format!("c{:016x}", 1.0f64.to_bits()),
                    "==".into(),
                    "if".into(),
                    null.into(),
                    "else".into(),
                    absent.into(),
                    "end".into(),
                    "end".into(),
                ]
            };
            jit_expression_kind(&selected)?;
            locals.insert(parameter.clone(), selected);
            slot += 2;
            continue;
        }
        let (ty, optional_type) = match ty {
            thaw_hir::HirType::Optional(payload) => (payload.as_ref(), true),
            ty => (*ty, false),
        };
        let optional = *optional_parameter || optional_type;
        if let thaw_hir::HirType::Union(elements) = ty {
            if !jit_argument_tagged_union(elements) {
                return None;
            }
            let kinds = elements
                .iter()
                .map(jit_union_member_code)
                .collect::<Option<String>>()?;
            if let Some(default) = default {
                if !optional {
                    return None;
                }
                if elements
                    .iter()
                    .any(|element| matches!(element, thaw_hir::HirType::Object(_)))
                {
                    return None;
                }
                let mut present = vec![format!("u{}{kinds}", slot + 1)];
                let mut fallback = Vec::new();
                encode_expression(
                    default,
                    &parameters,
                    &locals,
                    &mut context,
                    &mut fallback,
                )?;
                normalize_callable_branches([&mut present, &mut fallback])?;
                let mut selected = vec![format!("a{slot}"), "asbool".into(), "if".into()];
                selected.extend(present);
                selected.push("else".into());
                selected.extend(fallback);
                selected.push("end".into());
                locals.insert(parameter.clone(), selected);
                slot += 3;
                continue;
            }
            if optional {
                let source = vec![format!("u{}{kinds}", slot + 1)];
                bind_jit_union_object_fields(parameter, elements, &source, &mut locals)?;
                parameters.insert(
                    parameter.clone(),
                    format!("optional:a{slot}:u{}{kinds}", slot + 1),
                );
                slot += 3;
                continue;
            }
            let source = vec![format!("u{slot}{kinds}")];
            bind_jit_union_object_fields(parameter, elements, &source, &mut locals)?;
            locals.insert(parameter.clone(), source);
            slot += 2;
            continue;
        }
        if matches!(
            ty,
            thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)
        ) {
            if default.is_some() {
                return None;
            }
            if optional {
                let presence = format!("a{slot}");
                slot += 1;
                bind_jit_aggregate_fields(parameter, ty, &mut parameters, &mut slot)?;
                parameters.insert(
                    parameter.clone(),
                    format!("optional:{presence}:aggregate"),
                );
                continue;
            }
            bind_jit_aggregate_fields(parameter, ty, &mut parameters, &mut slot)?;
            continue;
        }
        let prefix = match ty {
            thaw_hir::HirType::Str => "s",
            thaw_hir::HirType::Bool => "b",
            thaw_hir::HirType::Array(element) => match element.as_ref() {
                thaw_hir::HirType::F64 => "rn",
                thaw_hir::HirType::Bool => "rb",
                thaw_hir::HirType::Str => "rs",
                thaw_hir::HirType::Tuple(elements) => match elements.as_slice() {
                    [thaw_hir::HirType::Str, thaw_hir::HirType::F64] => "en",
                    [thaw_hir::HirType::Str, thaw_hir::HirType::Bool] => "eb",
                    [thaw_hir::HirType::Str, thaw_hir::HirType::Str] => "es",
                    _ => return None,
                },
                _ => return None,
            },
            thaw_hir::HirType::Dictionary(element) => match element.as_ref() {
                thaw_hir::HirType::F64 => "dn",
                thaw_hir::HirType::Bool => "db",
                thaw_hir::HirType::Str => "ds",
                _ => return None,
            },
            _ => "a",
        };
        if optional {
            let presence = format!("a{slot}");
            let value = format!("{prefix}{}", slot + 1);
            let Some(default) = default.as_ref() else {
                parameters.insert(
                    parameter.clone(),
                    format!("optional:{presence}:{value}"),
                );
                slot += 2;
                continue;
            };
            let mut fallback = Vec::new();
            encode_expression(
                default,
                &parameters,
                &locals,
                &mut context,
                &mut fallback,
            )?;
            if *ty == thaw_hir::HirType::Bool {
                let mut boolean = Vec::new();
                append_boolean(fallback, &mut boolean)?;
                fallback = boolean;
            }
            let mut selected = vec![presence, "asbool".into(), "if".into(), value, "else".into()];
            selected.extend(fallback);
            selected.push("end".into());
            jit_expression_kind(&selected)?;
            locals.insert(parameter.clone(), selected);
            slot += 2;
        } else {
            parameters.insert(parameter.clone(), format!("{prefix}{slot}"));
            slot += 1;
        }
    }
    if slot > 16 {
        return None;
    }
    let starts_loop = matches!(body, NumericBody::Statements(statements)
        if matches!(statements.first(), Some(Stmt::While(_) | Stmt::For(_) | Stmt::DoWhile(_) | Stmt::ForIn(_) | Stmt::ForOf(_)))
            || statements.first().is_some_and(|statement| matches!(statement, Stmt::Labeled(labeled) if matches!(labeled.body.as_ref(), Stmt::While(_) | Stmt::For(_) | Stmt::DoWhile(_) | Stmt::ForIn(_) | Stmt::ForOf(_)))));
    if !starts_loop
        && matches!(
            &function.ret,
            thaw_bridge::DtsType::Native(
                thaw_hir::HirType::Object(_)
                    | thaw_hir::HirType::Dictionary(_)
                    | thaw_hir::HirType::Tuple(_)
            )
        )
    {
        let thaw_bridge::DtsType::Native(ty) = &function.ret else {
            unreachable!()
        };
        let mut jit_locals = Vec::new();
        let mut mutable = std::collections::HashSet::new();
        for step in local_steps {
            if slot == 16 {
                return None;
            }
            let mut encoded = Vec::new();
            let binding = match step {
                LocalStep::Declare {
                    name,
                    initializer,
                    mutable: is_mutable,
                } => {
                    if parameters.contains_key(name.sym.as_ref())
                        || locals.contains_key(name.sym.as_ref())
                    {
                        return None;
                    }
                    encode_expression(
                        initializer,
                        &parameters,
                        &locals,
                        &mut context,
                        &mut encoded,
                    )?;
                    if is_mutable {
                        mutable.insert(name.sym.to_string());
                    }
                    Some(name)
                }
                LocalStep::Assign {
                    name,
                    operation,
                    value,
                } => {
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    if operation == AssignOp::AddAssign {
                        let mut right = Vec::new();
                        encode_expression(
                            value,
                            &parameters,
                            &locals,
                            &mut context,
                            &mut right,
                        )?;
                        append_add(
                            locals.get(name.sym.as_ref())?.clone(),
                            right,
                            &mut encoded,
                        )?;
                    } else {
                        if operation != AssignOp::Assign {
                            encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                        }
                        encode_expression(
                            value,
                            &parameters,
                            &locals,
                            &mut context,
                            &mut encoded,
                        )?;
                        if operation != AssignOp::Assign {
                            encoded.push(
                                match operation {
                                    AssignOp::SubAssign => "-",
                                    AssignOp::MulAssign => "*",
                                    AssignOp::DivAssign => "/",
                                    AssignOp::ModAssign => "%",
                                    AssignOp::LShiftAssign => "shl",
                                    AssignOp::RShiftAssign => "shr",
                                    AssignOp::ZeroFillRShiftAssign => "ushr",
                                    AssignOp::BitOrAssign => "bor",
                                    AssignOp::BitXorAssign => "bxor",
                                    AssignOp::BitAndAssign => "band",
                                    AssignOp::ExpAssign => "pow",
                                    _ => return None,
                                }
                                .into(),
                            );
                        }
                    }
                    Some(name)
                }
                LocalStep::Update { name, operation } => {
                    if !mutable.contains(name.sym.as_ref()) {
                        return None;
                    }
                    encoded.extend(locals.get(name.sym.as_ref())?.iter().cloned());
                    encoded.push(format!("c{:016x}", 1.0f64.to_bits()));
                    encoded.push(
                        match operation {
                            UpdateOp::PlusPlus => "+",
                            UpdateOp::MinusMinus => "-",
                        }
                        .into(),
                    );
                    Some(name)
                }
                LocalStep::DestructureArray { .. } | LocalStep::DestructureObject { .. } => {
                    return None;
                }
                LocalStep::Effect(expression) => {
                    encode_expression(
                        expression,
                        &parameters,
                        &locals,
                        &mut context,
                        &mut encoded,
                    )?;
                    None
                }
            };
            let kind = jit_expression_kind(&encoded)?.0;
            let (ty, prefix) = match kind {
                JitKind::Number => (thaw_hir::HirType::F64, "a"),
                JitKind::Boolean => (thaw_hir::HirType::Bool, "b"),
                JitKind::String => (thaw_hir::HirType::Str, "s"),
                JitKind::Dynamic => (
                    thaw_hir::HirType::Union(vec![
                        thaw_hir::HirType::F64,
                        thaw_hir::HirType::Bool,
                        thaw_hir::HirType::Str,
                    ]),
                    "ld",
                ),
                JitKind::Array => match array_prefix(&encoded)? {
                    "rn" => (
                        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::F64)),
                        "rn",
                    ),
                    "rb" => (
                        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Bool)),
                        "rb",
                    ),
                    "rs" => (
                        thaw_hir::HirType::Array(Box::new(thaw_hir::HirType::Str)),
                        "rs",
                    ),
                    _ => return None,
                },
                JitKind::Dictionary => return None,
            };
            let operation = validated_jit_expression(encoded, kind)?;
            if let Some(name) = binding {
                locals.insert(name.sym.to_string(), vec![format!("{prefix}{slot}")]);
            }
            jit_locals.push(JitLocal { ty, operation });
            slot += 1;
        }
        let result = encode_aggregate_body(
            body,
            ty,
            &parameters,
            &locals,
            &mut context,
        )?;
        return Some(if jit_locals.is_empty() {
            result
        } else {
            JitExport::WithLocals(jit_locals, Box::new(result))
        });
    }
    if local_steps.is_empty() && !starts_loop {
        if let thaw_bridge::DtsType::Native(ty) = &function.ret {
            let aggregate_wrapper = matches!(
                ty,
                thaw_hir::HirType::Optional(payload)
                    | thaw_hir::HirType::Nullable(payload)
                    | thaw_hir::HirType::Nullish(payload)
                    if matches!(payload.as_ref(), thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_))
            );
            if jit_result_supported(ty)
                && (aggregate_wrapper
                    || !matches!(
                        ty,
                        thaw_hir::HirType::Optional(_)
                            | thaw_hir::HirType::Nullable(_)
                            | thaw_hir::HirType::Nullish(_)
                    ))
            {
                return encode_aggregate_body(body, ty, &parameters, &locals, &mut context);
            }
        }
    }
    encode_steps_and_body(
        local_steps,
        body,
        &parameters,
        locals,
        &mut context,
        &mut expression,
    )?;
    let expected = match &function.ret {
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Str) => JitKind::String,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Union(elements))
            if jit_tagged_union(elements) =>
        {
            JitKind::Dynamic
        }
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Bool) => JitKind::Boolean,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if matches!(element.as_ref(), thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str) => JitKind::Array,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Dictionary(element))
            if matches!(element.as_ref(), thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Str) => JitKind::Dictionary,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::Optional(payload)
            | thaw_hir::HirType::Nullable(payload)
            | thaw_hir::HirType::Nullish(payload),
        ) => jit_return_kind(payload)?,
        _ => JitKind::Number,
    };
    validated_jit_expression(expression, expected).map(JitExport::Value)
}
