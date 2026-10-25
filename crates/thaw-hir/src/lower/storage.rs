use swc_common::{Span, Spanned};

#[derive(Default)]
struct StoragePlan {
    dynamic: HashSet<(u32, u32)>,
    source_types: HashMap<(u32, u32), HirType>,
    source_conflicts: HashSet<(u32, u32)>,
    class_fields: HashMap<(String, String), (Span, bool)>,
    /// `(class, field)` pairs widened to `Optional` because the constructor
    /// may leave them unset (see `maybe_uninit_fields`).
    uninit_fields: HashSet<(String, String)>,
    /// Spans of member reads in `undefined`-observing positions (see
    /// `ObservedMembers`); they keep the raw `Optional` of an uninit field.
    observed_members: HashSet<(u32, u32)>,
    /// Property names read in an observing position, and whether the module
    /// prints or serialises whole objects at all (`console.*`, `JSON.*`,
    /// `Object.*`); constructor-less classes with a bare `x: T;` only widen
    /// `x` when one of these could expose its `undefined`.
    observed_names: HashSet<String>,
    observes_objects: bool,
    /// Classes that are instantiated (`new C`) or have an instantiated subclass.
    instantiated_classes: HashSet<String>,
    /// Source spans of anonymous function/arrow expressions to the name JavaScript infers
    /// for them (`const f = () => 1`, `{ cb: function () {} }`, `x = () => 1`, `class { f = () => 1 }`).
    function_names: HashMap<(u32, u32), String>,
    /// Whether function names are registered at all: only when the module can print a function.
    names_functions: bool,
    /// Member reads of a maybe-uninitialised field whose `undefined` must survive (read as `Json`).
    dynamic_reads: HashSet<(u32, u32)>,
    /// Names of fields some class may leave unset.
    uninit_names: HashSet<String>,
    /// Variables (and the containers they hold) that receive a sink read: their type widens
    /// to keep `undefined` (see `widen_sink_type`).
    sink_slots: HashSet<(u32, u32)>,
    /// Function results (`(function start, 0)`) that return a sink read: widened the same way.
    sink_results: HashSet<(u32, u32)>,
    argument_parameters: HashMap<(u32, u32), HashSet<(u32, u32)>>,
    callable_parameters: HashMap<u32, Vec<(u32, u32)>>,
    quickjs_function_value_nodes: HashSet<(u32, u32)>,
    quickjs_function_value_slots: HashSet<(u32, u32)>,
    quickjs_function_sources: HashSet<u32>,
    jit_calls: Vec<JitStorageCall>,
}

struct JitStorageCall {
    symbols: Vec<Symbol>,
    arguments: Vec<Span>,
    result: Span,
}

struct StorageCallConnections {
    jit_calls: Vec<JitStorageCall>,
    quickjs_function_value_nodes: HashSet<(u32, u32)>,
    quickjs_function_value_slots: HashSet<(u32, u32)>,
    quickjs_function_sources: HashSet<u32>,
}

impl StoragePlan {
    fn analyze(module: &Module) -> Result<Self, String> {
        let mut tables = ValueKinds::default();
        module.visit_with(&mut tables);
        let mut observed = ObservedMembers {
            tables,
            extended: EXTENDED_STORAGE.with(Cell::get),
            ..ObservedMembers::default()
        };
        module.visit_with(&mut observed);
        let mut instantiated_classes = HashSet::new();
        for class in &observed.tables.instantiated {
            let mut current = Some(class.clone());
            while let Some(name) = current {
                if !instantiated_classes.insert(name.clone()) {
                    break;
                }
                current = observed.tables.supers.get(&name).cloned();
            }
        }
        // Reads of a maybe-uninitialised field in a sink that keeps `undefined`
        // (`return p.n`, `arr.push(p.n)`, ...) are read as `Json`.
        let mut uninit = UninitNames {
            names: HashSet::new(),
            instantiated: &instantiated_classes,
            observed: &observed,
        };
        module.visit_with(&mut uninit);
        let uninit_names = uninit.names;
        let uninit_sinks = observed
            .sinks
            .iter()
            .filter(|(_, name)| uninit_names.contains(name))
            .map(|(span, _)| storage_key(*span))
            .collect::<HashSet<_>>();
        if !uninit_sinks.is_empty() {
            EXTENDED_STORAGE_USED.with(|used| used.set(true));
        }
        let mut graph = StorageGraph { uninit_sinks, ..StorageGraph::default() };
        module.visit_with(&mut graph);
        let graph_uninit_sinks = graph.uninit_sinks.clone();
        let graph_sink_slots = graph.sink_slots.clone();
        let graph_sink_results = graph.sink_results.clone();
        let StorageCallConnections {
            jit_calls,
            quickjs_function_value_nodes,
            quickjs_function_value_slots,
            quickjs_function_sources,
        } = graph.connect_calls();
        let mut pending: Vec<_> = graph.seeds.iter().copied().collect();
        let mut dynamic = graph.seeds;
        while let Some(node) = pending.pop() {
            if let Some(neighbors) = graph.edges.get(&node) {
                for &neighbor in neighbors {
                    if dynamic.insert(neighbor) {
                        pending.push(neighbor);
                    }
                }
            }
            if let Some(callable) = graph.callables.get(&node) {
                for &target in &callable.escaped {
                    if dynamic.insert(target) { pending.push(target); }
                }
            }
        }
        let callable_parameters = graph
            .callables
            .into_iter()
            .map(|((start, _), callable)| {
                (start, callable.params.iter().copied().map(storage_key).collect())
            })
            .collect();
        let mut names = FunctionNames::default();
        module.visit_with(&mut names);
        Ok(Self {
            dynamic,
            source_types: HashMap::new(),
            source_conflicts: HashSet::new(),
            class_fields: HashMap::new(),
            uninit_fields: HashSet::new(),
            observed_members: observed.members,
            observed_names: observed.names,
            observes_objects: observed.objects,
            instantiated_classes,
            function_names: names.names,
            names_functions: observed.objects,
            dynamic_reads: graph_uninit_sinks,
            uninit_names,
            sink_slots: graph_sink_slots,
            sink_results: graph_sink_results,
            argument_parameters: graph.argument_parameters,
            callable_parameters,
            quickjs_function_value_nodes,
            quickjs_function_value_slots,
            quickjs_function_sources,
            jit_calls,
        })
    }

    fn jit_calls_requiring_dynamic_storage(&self) -> HashSet<Symbol> {
        self.jit_calls
            .iter()
            .filter(|call| {
                self.is_dynamic(call.result)
                    || call.arguments.iter().any(|argument| self.is_dynamic(*argument))
            })
            .flat_map(|call| call.symbols.iter().cloned())
            .collect()
    }

    fn result_is_dynamic(&self, span: Span) -> bool {
        self.dynamic.contains(&(span.lo.0, 0))
    }

    fn is_dynamic(&self, span: Span) -> bool {
        self.dynamic.contains(&storage_key(span))
    }
}

/// Returns private JIT entry symbols whose arguments or results cross a
/// dynamic-storage boundary in the same normalized storage graph used by
/// `lower_module`.
pub fn jit_calls_requiring_dynamic_storage(module: &Module) -> Result<HashSet<Symbol>, String> {
    let normalized = normalize_module_for_lowering(module)?;
    Ok(StoragePlan::analyze(&normalized)?.jit_calls_requiring_dynamic_storage())
}

fn storage_key(span: Span) -> (u32, u32) {
    (span.lo.0, span.hi.0)
}

fn storage_has_any(ty: &TsType) -> bool {
    matches!(ty, TsType::TsKeywordType(keyword)
        if matches!(keyword.kind, TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsUnknownKeyword))
}

struct StorageCallable {
    params: Vec<Span>,
    rest: bool,
    result: (u32, u32),
    escaped: Vec<(u32, u32)>,
}
struct StorageCall {
    callee: Span,
    arguments: Vec<(Span, bool)>,
    result: Span,
    source_function: Option<(u32, u32)>,
}
type StorageAlias = (Span, Vec<Symbol>, Box<TsType>);

#[derive(Default)]
struct StorageGraph {
    edges: HashMap<(u32, u32), Vec<(u32, u32)>>,
    argument_parameters: HashMap<(u32, u32), HashSet<(u32, u32)>>,
    seeds: HashSet<(u32, u32)>,
    scopes: Vec<HashMap<String, Span>>,
    aliases: Vec<HashMap<Symbol, StorageAlias>>,
    class_types: Vec<HashMap<Symbol, Span>>,
    native_results: HashMap<(u32, u32), Span>,
    source_functions: HashMap<(u32, u32), (u32, u32)>,
    source_methods: HashMap<(u32, u32, Symbol, bool), (u32, u32)>,
    source_parents: HashMap<(u32, u32), (u32, u32)>,
    source_constructors: HashMap<(u32, u32), (u32, u32)>,
    annotations: HashMap<(u32, u32), Box<TsType>>,
    functions: Vec<(Span, usize)>,
    classes: Vec<Span>,
    callables: HashMap<(u32, u32), StorageCallable>,
    calls: Vec<StorageCall>,
    call_result_bindings: HashMap<(u32, u32), Vec<(u32, u32)>>,
    captures: HashSet<((u32, u32), (u32, u32))>,
    jit_declarations: HashMap<(u32, u32), Symbol>,
    quickjs_function_declarations: HashMap<(u32, u32), u32>,
    exceptions: Vec<Span>,
    /// Member reads (`p.n`) of maybe-uninitialised fields in `undefined`-keeping sinks.
    uninit_sinks: HashSet<(u32, u32)>,
    /// Function results that return a sink read. Deliberately not part of the dynamic
    /// propagation: spreading it over call arguments would turn the receiver's class dynamic.
    sink_results: HashSet<(u32, u32)>,
    /// Variables (and the arrays/maps they hold) that receive a sink read.
    sink_slots: HashSet<(u32, u32)>,
}

impl StorageGraph {
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.aliases.push(HashMap::new());
        self.class_types.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
        self.aliases.pop();
        self.class_types.pop();
    }
    fn resolve_annotation(&self, source: &TsType) -> Box<TsType> {
        struct Resolve<'g> { graph: &'g StorageGraph, active: HashSet<(u32, u32)> }
        impl VisitMut for Resolve<'_> {
            fn visit_mut_ts_type(&mut self, ty: &mut TsType) {
                if let TsType::TsTypeRef(reference) = ty {
                    if let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name {
                        if let Some((span, parameters, target)) = self.graph.aliases.iter().rev().find_map(|scope| scope.get(name.sym.as_ref())) {
                            let key = storage_key(*span);
                            if !self.active.insert(key) { return; }
                            let substitutions = parameters.iter().cloned().zip(reference.type_params.iter().flat_map(|args| args.params.iter().cloned())).collect();
                            let mut target = target.clone();
                            target.visit_mut_with(&mut GenericClassTypeSubstituter::new(&substitutions));
                            target.visit_mut_with(self);
                            *ty = *target;
                            self.active.remove(&key);
                            return;
                        }
                    }
                }
                ty.visit_mut_children_with(self);
            }
        }
        let mut resolved = Box::new(source.clone());
        resolved.visit_mut_with(&mut Resolve { graph: self, active: HashSet::new() });
        resolved
    }

    fn source_constructor(&self, expression: &Expr) -> Option<(u32, u32)> {
        let Expr::Ident(name) = expression else { return None };
        let mut class = storage_key(self.class_types.iter().rev().find_map(|scope| scope.get(name.sym.as_ref()).copied())?);
        let mut seen = HashSet::new();
        while seen.insert(class) {
            if let Some(constructor) = self.source_constructors.get(&class) { return Some(*constructor); }
            class = *self.source_parents.get(&class)?;
        }
        None
    }

    fn source_callable(&self, expression: &Expr) -> Option<(u32, u32)> {
        match expression {
            Expr::Ident(name) => self.lookup(name.sym.as_ref()).and_then(|(binding, _)| self.source_functions.get(&storage_key(binding)).copied()),
            Expr::Paren(value) => self.source_callable(&value.expr),
            Expr::Arrow(value) => Some(storage_key(value.span)),
            Expr::Fn(value) => Some(storage_key(value.function.span)),
            Expr::Member(member) => {
                let MemberProp::Ident(method) = &member.prop else { return None };
                let (class, is_static) = if matches!(member.obj.as_ref(), Expr::This(_)) {
                    (*self.classes.last()?, false)
                } else if let Some(annotation) = self.source_annotation(&member.obj) {
                    (self.annotated_class(&annotation)?, false)
                } else if let Expr::Ident(name) = member.obj.as_ref() {
                    (self.class_types.iter().rev().find_map(|scope| scope.get(name.sym.as_ref()).copied())?, true)
                } else { return None };
                let mut class = storage_key(class);
                let mut seen = HashSet::new();
                while seen.insert(class) {
                    if let Some(function) = self.source_methods.get(&(class.0, class.1, method.sym.to_string(), is_static)) { return Some(*function); }
                    class = *self.source_parents.get(&class)?;
                }
                None
            }
            _ => None,
        }
    }

    fn source_annotation(&self, expression: &Expr) -> Option<Box<TsType>> {
        match expression {
            Expr::Ident(name) => self.lookup(name.sym.as_ref()).and_then(|(span, _)| self.annotations.get(&storage_key(span)).cloned()),
            Expr::Paren(value) => self.source_annotation(&value.expr),
            Expr::TsAs(value) => Some(self.resolve_annotation(&value.type_ann)),
            Expr::TsTypeAssertion(value) => Some(self.resolve_annotation(&value.type_ann)),
            Expr::New(value) => {
                let Expr::Ident(name) = value.callee.as_ref() else { return None };
                Some(Box::new(TsType::TsTypeRef(swc_ecma_ast::TsTypeRef { span: value.span,
                    type_name: swc_ecma_ast::TsEntityName::Ident(name.clone()), type_params: value.type_args.clone() })))
            }
            Expr::Object(object) if !object_literal_has_proto_setter(object) => {
                let members = object.props.iter().filter_map(|property| {
                    let PropOrSpread::Prop(property) = property else { return None };
                    let (key, value) = match property.as_ref() {
                        Prop::KeyValue(property) => {
                            let PropName::Ident(key) = &property.key else { return None };
                            (Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(key.sym.clone(), key.span)), property.value.clone())
                        }
                        Prop::Shorthand(name) => (Expr::Ident(name.clone()), Box::new(Expr::Ident(name.clone()))),
                        _ => return None,
                    };
                    Some(swc_ecma_ast::TsTypeElement::TsPropertySignature(swc_ecma_ast::TsPropertySignature {
                        span: object.span, readonly: false, key: Box::new(key), computed: false, optional: false,
                        type_ann: Some(Box::new(swc_ecma_ast::TsTypeAnn { span: object.span, type_ann: self.source_annotation(&value)? })),
                    }))
                }).collect();
                Some(Box::new(TsType::TsTypeLit(swc_ecma_ast::TsTypeLit { span: object.span, members })))
            }
            Expr::Member(member) => {
                let owner = self.source_annotation(&member.obj)?;
                if storage_has_any(&owner) { return Some(owner); }
                match owner.as_ref() {
                    TsType::TsTypeLit(object) => {
                        let MemberProp::Ident(name) = &member.prop else { return None };
                        object.members.iter().find_map(|property| {
                            let swc_ecma_ast::TsTypeElement::TsPropertySignature(property) = property else { return None };
                            matches!(property.key.as_ref(), Expr::Ident(key) if key.sym == name.sym)
                                .then(|| property.type_ann.as_ref().map(|ann| ann.type_ann.clone())).flatten()
                        })
                    }
                    TsType::TsArrayType(array) if matches!(member.prop, MemberProp::Computed(_)) => Some(array.elem_type.clone()),
                    _ => None,
                }
            }
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else { return None };
                let Expr::Member(member) = callee.as_ref() else { return None };
                let MemberProp::Ident(method) = &member.prop else { return None };
                let owner = self.source_annotation(&member.obj)?;
                let TsType::TsTypeRef(reference) = owner.as_ref() else { return None };
                let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name else { return None };
                if matches!(name.sym.as_ref(), "Map" | "WeakMap") && method.sym == "get" {
                    reference.type_params.as_ref()?.params.get(1).cloned()
                } else { None }
            }
            _ => None,
        }
    }

    fn annotated_class(&self, annotation: &TsType) -> Option<Span> {
        let annotation = self.resolve_annotation(annotation);
        let TsType::TsTypeRef(reference) = annotation.as_ref() else { return None };
        let swc_ecma_ast::TsEntityName::Ident(name) = &reference.type_name else { return None };
        self.class_types.iter().rev().find_map(|scope| scope.get(name.sym.as_ref()).copied())
    }

    fn annotated_value(&mut self, annotation: &TsType, value: &Expr) {
        let annotation = self.resolve_annotation(annotation);
        if storage_has_any(&annotation) { self.expose(value.span()); return; }
        match (annotation.as_ref(), value) {
            (TsType::TsArrayType(array), Expr::Array(values)) => {
                for value in values.elems.iter().flatten() { self.annotated_value(&array.elem_type, &value.expr); }
            }
            (TsType::TsTupleType(tuple), Expr::Array(values)) => {
                for (slot, value) in tuple.elem_types.iter().zip(&values.elems) {
                    if let Some(value) = value { self.annotated_value(&slot.ty, &value.expr); }
                }
            }
            (TsType::TsTypeLit(record), Expr::Object(object)) => {
                for member in &record.members {
                    let swc_ecma_ast::TsTypeElement::TsPropertySignature(field) = member else { continue };
                    let Some(annotation) = &field.type_ann else { continue };
                    let name = match field.key.as_ref() {
                        Expr::Ident(name) => name.sym.to_string(),
                        Expr::Lit(Lit::Str(name)) => name.value.to_string_lossy().into_owned(),
                        _ => continue,
                    };
                    for property in &object.props {
                        let PropOrSpread::Prop(property) = property else { continue };
                        match property.as_ref() {
                            Prop::KeyValue(property) if class_property_name(&property.key).ok().as_ref() == Some(&name) => self.annotated_value(&annotation.type_ann, &property.value),
                            Prop::Shorthand(identifier) if identifier.sym == name => self.annotated_value(&annotation.type_ann, &Expr::Ident(identifier.clone())),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fn is_fresh_static_builtin(&self, member: &MemberExpr) -> bool {
        let (Expr::Ident(object), swc_ecma_ast::MemberProp::Ident(property)) =
            (member.obj.as_ref(), &member.prop) else { return false };
        self.lookup(object.sym.as_ref()).is_none()
            && matches!((object.sym.as_ref(), property.sym.as_ref()),
                ("Object", "fromEntries" | "groupBy") | ("Map", "groupBy")
                | ("Buffer", "from" | "concat" | "alloc") | ("Uint8Array", "from"))
    }

    fn link(&mut self, left: Span, right: Span) {
        if left.is_dummy() || right.is_dummy() || left == right {
            return;
        }
        self.edges.entry(storage_key(left)).or_default().push(storage_key(right));
        self.edges.entry(storage_key(right)).or_default().push(storage_key(left));
    }

    fn contain(&mut self, owner: Span, value: Span) {
        if !owner.is_dummy() && !value.is_dummy() {
            self.edges.entry(storage_key(owner)).or_default().push(storage_key(value));
        }
    }

    fn link_keys(&mut self, left: (u32, u32), right: (u32, u32)) {
        self.edges.entry(left).or_default().push(right);
        self.edges.entry(right).or_default().push(left);
    }
    fn connect_calls(&mut self) -> StorageCallConnections {
        let mut connected = HashSet::new();
        let mut native_connected = HashSet::new();
        loop {
            let mut links = Vec::new();
            for (index, call) in self.calls.iter().enumerate() {
                let mut pending = vec![storage_key(call.callee)];
                let mut seen = HashSet::new();
                while let Some(node) = pending.pop() {
                    if !seen.insert(node) { continue; }
                    if let Some(class) = self.native_results.get(&node) {
                        if native_connected.insert((index, node)) { links.push((storage_key(call.result), storage_key(*class))); }
                    }
                    if let Some(callable) = self.callables.get(&node) {
                        if connected.insert((index, node)) {
                            links.push((storage_key(call.result), callable.result));
                            let mut spread_seen = false;
                            for (position, &(actual, spread)) in call.arguments.iter().enumerate() {
                                if spread || spread_seen {
                                    let start = if spread_seen { 0 } else { position.min(callable.params.len()) };
                                    for formal in &callable.params[start..] { links.push((storage_key(actual), storage_key(*formal))); }
                                    spread_seen = true;
                                } else if let Some(formal) = callable.params.get(position).or_else(|| callable.rest.then(|| callable.params.last()).flatten()) {
                                    links.push((storage_key(actual), storage_key(*formal)));
                                    if call.source_function == Some(node) && position < callable.params.len() - usize::from(callable.rest) {
                                        self.argument_parameters.entry(storage_key(actual)).or_default().insert(storage_key(*formal));
                                    }
                                }
                            }
                        }
                    }
                    if let Some(neighbors) = self.edges.get(&node) {
                        pending.extend(neighbors.iter().copied().filter(|next| !self.captures.contains(&(node, *next))));
                    }
                }
            }
            if links.is_empty() { break; }
            for (left, right) in links { self.link_keys(left, right); }
        }
        let jit_calls = self.calls.iter().filter_map(|call| {
            let symbols = self.reachable_jit_symbols(call.callee);
            (!symbols.is_empty()).then(|| JitStorageCall {
                symbols,
                arguments: call.arguments.iter().map(|(span, _)| *span).collect(),
                result: call.result,
            })
        }).collect();
        let quickjs_function_value_nodes = self.reachable_quickjs_function_values();
        let mut quickjs_function_value_slots: HashSet<_> = quickjs_function_value_nodes
            .iter()
            .filter_map(|node| self.callables.get(node))
            .flat_map(|callable| callable.escaped.iter().copied())
            .collect();
        for call in &self.calls {
            if quickjs_function_value_nodes.contains(&storage_key(call.callee)) {
                quickjs_function_value_slots.insert((call.result.lo.0, 0));
                if let Some(bindings) = self.call_result_bindings.get(&storage_key(call.result)) {
                    quickjs_function_value_slots.extend(bindings.iter().copied());
                }
            }
        }
        let quickjs_function_sources = self
            .quickjs_function_declarations
            .values()
            .copied()
            .collect();
        let unresolved: Vec<_> = self.calls.iter().enumerate()
            .filter(|(index, _)| !connected.iter().any(|(call, _)| call == index))
            .map(|(_, call)| (call.callee, call.result, call.arguments.clone())).collect();
        for (callee, result, arguments) in unresolved {
            self.contain(callee, result);
            for (argument, _) in arguments { self.contain(callee, argument); }
        }
        StorageCallConnections {
            jit_calls,
            quickjs_function_value_nodes,
            quickjs_function_value_slots,
            quickjs_function_sources,
        }
    }

    fn reachable_quickjs_function_values(&self) -> HashSet<(u32, u32)> {
        let mut values = HashSet::new();
        let mut pending = self
            .quickjs_function_declarations
            .keys()
            .copied()
            .collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            if !values.insert(node) {
                continue;
            }
            if let Some(neighbors) = self.edges.get(&node) {
                pending.extend(
                    neighbors
                        .iter()
                        .copied()
                        .filter(|next| !self.captures.contains(&(node, *next))),
                );
            }
        }
        values
    }

    fn reachable_jit_symbols(&self, callee: Span) -> Vec<Symbol> {
        if self.jit_declarations.is_empty() {
            return Vec::new();
        }
        let mut symbols = HashSet::new();
        let mut pending = vec![storage_key(callee)];
        let mut seen = HashSet::new();
        while let Some(node) = pending.pop() {
            if !seen.insert(node) { continue; }
            if let Some(symbol) = self.jit_declarations.get(&node) {
                symbols.insert(symbol.clone());
            }
            if let Some(neighbors) = self.edges.get(&node) {
                pending.extend(neighbors.iter().copied()
                    .filter(|next| !self.captures.contains(&(node, *next))));
            }
        }
        symbols.into_iter().collect()
    }
    fn callable(&mut self, span: Span, parameters: &[Pat], receiver: Option<&swc_ecma_ast::TsThisParam>, result: Option<&TsType>) {
        let result_node = (span.lo.0, 0);
        let callable_node = storage_key(span);
        self.edges.entry(result_node).or_default().push(callable_node);
        self.captures.insert((result_node, callable_node));
        let mut escaped = Vec::new();
        if result.is_none_or(storage_reference_type) { escaped.push(result_node); }
        fn reference_parameter(parameter: &Pat) -> bool {
            match parameter {
                Pat::Ident(binding) => binding.type_ann.as_ref().is_none_or(|ann| storage_reference_type(&ann.type_ann)),
                Pat::Assign(default) => reference_parameter(&default.left),
                Pat::Rest(_) | Pat::Array(_) | Pat::Object(_) => true,
                _ => false,
            }
        }
        for parameter in parameters {
            if reference_parameter(parameter) { escaped.push(storage_key(parameter.span())); }
        }
        if let Some(receiver) = receiver {
            if let Some(annotation) = &receiver.type_ann {
                let annotation_type = self.resolve_annotation(&annotation.type_ann);
                if storage_reference_type(&annotation_type) {
                    escaped.push(storage_key(receiver.span));
                }
                // Generic receiver templates are reified from the inner TsType span,
                // while the physical hidden parameter uses the whole `this` parameter.
                self.link(receiver.span, annotation.span);
                self.link(annotation.span, annotation.type_ann.span());
            }
        }
        self.callables.insert(storage_key(span), StorageCallable {
            params: parameters.iter().map(Spanned::span).collect(),
            rest: parameters.last().is_some_and(|parameter| matches!(parameter, Pat::Rest(_))),
            result: result_node, escaped,
        });
    }

    /// `expr` (through parentheses and casts) is a sink read of a maybe-uninitialised field.
    fn is_uninit_sink(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Paren(paren) => self.is_uninit_sink(&paren.expr),
            Expr::TsAs(cast) => self.is_uninit_sink(&cast.expr),
            Expr::TsNonNull(cast) => self.is_uninit_sink(&cast.expr),
            Expr::Member(member) => self.uninit_sinks.contains(&storage_key(member.span)),
            _ => false,
        }
    }

    fn expose(&mut self, span: Span) {
        if !span.is_dummy() {
            self.seeds.insert(storage_key(span));
        }
    }

    fn bind(&mut self, name: &swc_ecma_ast::Ident) {
        self.scopes.last_mut().unwrap().entry(name.sym.to_string()).or_insert(name.span);
    }

    fn pattern(&mut self, pattern: &Pat) {
        struct Bindings(Vec<Span>);
        impl Visit for Bindings {
            fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
                self.0.push(binding.id.span);
            }
        }
        let mut bindings = Bindings(Vec::new());
        pattern.visit_with(&mut bindings);
        for binding in bindings.0 { self.link(pattern.span(), binding); }
        match pattern {
            Pat::Ident(binding) => self.bind(&binding.id),
            Pat::Array(array) => {
                for item in array.elems.iter().flatten() { self.pattern(item); }
            }
            Pat::Object(object) => {
                for property in &object.props {
                    match property {
                        swc_ecma_ast::ObjectPatProp::KeyValue(property) => self.pattern(&property.value),
                        swc_ecma_ast::ObjectPatProp::Assign(property) => self.bind(&property.key.id),
                        swc_ecma_ast::ObjectPatProp::Rest(rest) => self.pattern(&rest.arg),
                    }
                }
            }
            Pat::Rest(rest) => self.pattern(&rest.arg),
            Pat::Assign(assign) => self.pattern(&assign.left),
            _ => {}
        }
    }

    fn declare(&mut self, declaration: &Decl) {
        match declaration {
            Decl::Var(variable) => {
                for declaration in &variable.decls { self.pattern(&declaration.name); }
            }
            Decl::Fn(function) => {
                self.bind(&function.ident);
                if function.function.body.is_some()
                    || ordinary_quickjs_typed_symbol(function.ident.sym.as_ref()).is_some()
                        && function.function.type_params.as_ref().is_none_or(|params| params.params.is_empty())
                        && function.function.this_param.is_none()
                        && function.function.params.iter().all(|parameter| {
                            !matches!(parameter.pat, Pat::Rest(_))
                                && !matches!(&parameter.pat, Pat::Ident(binding) if binding.id.optional)
                        })
                {
                    self.source_functions.insert(
                        storage_key(function.ident.span),
                        storage_key(function.function.span),
                    );
                }
            },
            Decl::Class(class) => {
                self.bind(&class.ident);
                self.class_types.last_mut().unwrap().insert(class.ident.sym.to_string(), class.class.span);
                let key = storage_key(class.class.span);
                if let Some(base) = class.class.super_class.as_ref().and_then(|base| {
                    let Expr::Ident(base) = base.as_ref() else { return None };
                    self.class_types.iter().rev().find_map(|scope| scope.get(base.sym.as_ref()).copied())
                }) { self.source_parents.insert(key, storage_key(base)); }
                for member in &class.class.body {
                    if let ClassMember::Constructor(constructor) = member { self.source_constructors.insert(key, storage_key(constructor.span)); }
                    if let ClassMember::Method(method) = member {
                        if method.kind == swc_ecma_ast::MethodKind::Method {
                            if let Ok(name) = class_property_name(&method.key) {
                                self.source_methods.insert((key.0, key.1, name, method.is_static), storage_key(method.function.span));
                            }
                        }
                    }
                }
            }
            Decl::TsTypeAlias(alias) => {
                self.aliases.last_mut().unwrap().insert(alias.id.sym.to_string(), (alias.span,
                    alias.type_params.iter().flat_map(|params| params.params.iter().map(|param| param.name.sym.to_string())).collect(), alias.type_ann.clone()));
            }
            Decl::TsInterface(interface) => {
                self.aliases.last_mut().unwrap().insert(interface.id.sym.to_string(), (interface.span,
                    interface.type_params.iter().flat_map(|params| params.params.iter().map(|param| param.name.sym.to_string())).collect(),
                    Box::new(TsType::TsTypeLit(swc_ecma_ast::TsTypeLit { span: interface.body.span, members: interface.body.body.clone() }))));
            }
            _ => {}
        }
    }

    fn lookup(&self, name: &str) -> Option<(Span, usize)> {
        self.scopes.iter().enumerate().rev()
            .find_map(|(depth, scope)| scope.get(name).map(|span| (*span, depth)))
    }

    fn hoist_vars(&mut self, statements: &[Stmt]) {
        struct Hoists(Vec<swc_ecma_ast::Ident>);
        impl Visit for Hoists {
            fn visit_var_decl(&mut self, variable: &VarDecl) {
                if variable.kind != VarDeclKind::Var { return; }
                struct Names<'a>(&'a mut Vec<swc_ecma_ast::Ident>);
                impl Visit for Names<'_> {
                    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
                        self.0.push(binding.id.clone());
                    }
                    fn visit_expr(&mut self, _: &Expr) {}
                }
                for declaration in &variable.decls { declaration.name.visit_with(&mut Names(&mut self.0)); }
            }
            fn visit_function(&mut self, _: &swc_ecma_ast::Function) {}
            fn visit_arrow_expr(&mut self, _: &swc_ecma_ast::ArrowExpr) {}
            fn visit_class(&mut self, _: &swc_ecma_ast::Class) {}
        }
        let mut hoists = Hoists(Vec::new());
        for statement in statements { statement.visit_with(&mut hoists); }
        for name in hoists.0 { self.bind(&name); }
    }

    fn connect_annotated_classes(&mut self, span: Span, annotation: &TsType) {
        if let Some(class) = self.annotated_class(annotation) { self.link(span, class); }
        match annotation {
            TsType::TsUnionOrIntersectionType(swc_ecma_ast::TsUnionOrIntersectionType::TsUnionType(union)) => {
                for member in &union.types { self.connect_annotated_classes(span, member); }
            }
            TsType::TsUnionOrIntersectionType(swc_ecma_ast::TsUnionOrIntersectionType::TsIntersectionType(intersection)) => {
                for member in &intersection.types { self.connect_annotated_classes(span, member); }
            }
            TsType::TsParenthesizedType(parenthesized) => self.connect_annotated_classes(span, &parenthesized.type_ann),
            _ => {}
        }
    }

    fn annotation(&mut self, span: Span, annotation: Option<&swc_ecma_ast::TsTypeAnn>) {
        if let Some(annotation) = annotation {
            let annotation = self.resolve_annotation(&annotation.type_ann);
            if storage_has_any(&annotation) { self.expose(span); }
            self.connect_annotated_classes(span, &annotation);
            self.annotations.insert(storage_key(span), annotation);
        }
    }
}

impl Visit for StorageGraph {
    fn visit_module(&mut self, module: &Module) {
        self.push_scope();
        for item in &module.body {
            match item {
                ModuleItem::Stmt(Stmt::Decl(declaration)) => self.declare(declaration),
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => self.declare(&export.decl),
                _ => {}
            }
        }
        module.visit_children_with(self);
        self.pop_scope();
    }

    fn visit_block_stmt(&mut self, block: &swc_ecma_ast::BlockStmt) {
        self.push_scope();
        for statement in &block.stmts {
            if let Stmt::Decl(declaration) = statement { self.declare(declaration); }
        }
        block.visit_children_with(self);
        self.pop_scope();
    }

    fn visit_function_body(&mut self, body: &swc_ecma_ast::FunctionBody) {
        self.push_scope();
        for statement in &body.stmts {
            if let Stmt::Decl(declaration) = statement { self.declare(declaration); }
        }
        body.visit_children_with(self);
        self.pop_scope();
    }

    fn visit_for_stmt(&mut self, statement: &swc_ecma_ast::ForStmt) {
        self.push_scope();
        statement.visit_children_with(self);
        self.pop_scope();
    }

    fn visit_for_in_stmt(&mut self, statement: &swc_ecma_ast::ForInStmt) {
        self.push_scope();
        statement.visit_children_with(self);
        self.pop_scope();
    }

    fn visit_for_of_stmt(&mut self, statement: &swc_ecma_ast::ForOfStmt) {
        self.push_scope();
        match &statement.left {
            swc_ecma_ast::ForHead::VarDecl(declaration) => {
                for binding in &declaration.decls { self.link(binding.name.span(), statement.right.span()); }
            }
            swc_ecma_ast::ForHead::Pat(pattern) => self.link(pattern.span(), statement.right.span()),
            _ => {}
        }
        statement.visit_children_with(self);
        self.pop_scope();
    }

    fn visit_switch_stmt(&mut self, statement: &swc_ecma_ast::SwitchStmt) {
        statement.discriminant.visit_with(self);
        self.push_scope();
        for case in &statement.cases {
            for statement in &case.cons {
                if let Stmt::Decl(declaration) = statement { self.declare(declaration); }
            }
        }
        for case in &statement.cases { case.visit_with(self); }
        self.pop_scope();
    }

    fn visit_fn_expr(&mut self, expression: &swc_ecma_ast::FnExpr) {
        self.push_scope();
        if let Some(identifier) = &expression.ident {
            self.bind(identifier);
            self.link(identifier.span, expression.function.span);
        }
        expression.function.visit_with(self);
        self.pop_scope();
    }

    fn visit_assign_pat(&mut self, pattern: &swc_ecma_ast::AssignPat) {
        self.link(pattern.span, pattern.left.span());
        self.link(pattern.span, pattern.right.span());
        pattern.visit_children_with(self);
    }

    fn visit_assign_pat_prop(&mut self, property: &swc_ecma_ast::AssignPatProp) {
        if let Some(value) = &property.value { self.link(property.key.id.span, value.span()); }
        property.visit_children_with(self);
    }

    fn visit_ident(&mut self, identifier: &swc_ecma_ast::Ident) {
        if let Some((binding, depth)) = self.lookup(identifier.sym.as_ref()) {
            self.link(identifier.span, binding);
            if let Some(&(function, local_depth)) = self.functions.last() {
                if depth < local_depth {
                    self.contain(function, binding);
                    self.captures.insert((storage_key(function), storage_key(binding)));
                    self.captures.insert((storage_key(binding), storage_key(function)));
                }
            }
        }
    }

    fn visit_member_expr(&mut self, member: &MemberExpr) {
        if self.source_annotation(&Expr::Member(member.clone())).is_some_and(|ty| storage_has_any(&ty)) { self.expose(member.span); }
        // A sink read produces the value; it must not make its receiver dynamic.
        if !self.uninit_sinks.contains(&storage_key(member.span)) {
            self.link(member.span, member.obj.span());
        }
        member.visit_children_with(self);
    }

    fn visit_var_declarator(&mut self, declaration: &swc_ecma_ast::VarDeclarator) {
        self.pattern(&declaration.name);
        if let Some(init) = &declaration.init {
            self.link(declaration.name.span(), init.span());
            if let (Pat::Ident(binding), true) = (&declaration.name, self.is_uninit_sink(init)) {
                self.sink_slots.insert(storage_key(binding.id.span));
                self.sink_slots.insert(storage_key(binding.span()));
            }
            if let Expr::Call(call) = init.as_ref() {
                self.call_result_bindings
                    .entry(storage_key(call.span))
                    .or_default()
                    .push(match &declaration.name {
                        Pat::Ident(binding) => storage_key(binding.id.span),
                        other => storage_key(other.span()),
                    });
            }
            if let Pat::Ident(binding) = &declaration.name {
                if binding.type_ann.as_ref().is_none_or(|ann| !storage_has_any(&self.resolve_annotation(&ann.type_ann))) {
                    if let Some(function) = self.source_callable(init) { self.source_functions.insert(storage_key(binding.id.span), function); }
                }
                if let Some(annotation) = &binding.type_ann { self.annotated_value(&annotation.type_ann, init); }
                else if let Some(annotation) = self.source_annotation(init) { if storage_has_any(&annotation) { self.expose(binding.id.span); } self.annotations.insert(storage_key(binding.id.span), annotation); }
            }
        }
        declaration.visit_children_with(self);
    }

    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        self.link(binding.span(), binding.id.span);
        self.annotation(binding.id.span, binding.type_ann.as_deref());
        binding.visit_children_with(self);
    }

    fn visit_fn_decl(&mut self, declaration: &FnDecl) {
        self.link(declaration.ident.span, declaration.function.span);
        if declaration.function.body.is_none() {
            if ordinary_quickjs_typed_symbol(declaration.ident.sym.as_ref()).is_some()
                && declaration.function.type_params.as_ref().is_none_or(|params| params.params.is_empty())
                && declaration.function.this_param.is_none()
                && declaration.function.params.iter().all(|parameter| {
                    !matches!(parameter.pat, Pat::Rest(_))
                        && !matches!(&parameter.pat, Pat::Ident(binding) if binding.id.optional)
                })
            {
                let function_span = storage_key(declaration.function.span);
                self.quickjs_function_declarations
                    .insert(function_span, declaration.function.span.lo.0);
                let parameters = declaration
                    .function
                    .params
                    .iter()
                    .map(|parameter| parameter.pat.clone())
                    .collect::<Vec<_>>();
                self.callable(
                    declaration.function.span,
                    &parameters,
                    None,
                    declaration
                        .function
                        .return_type
                        .as_ref()
                        .map(|annotation| annotation.type_ann.as_ref()),
                );
            }
            if dynamic_symbol(declaration.ident.sym.as_ref())
                .is_some_and(|(backend, _)| backend == DynamicBackend::Jit)
            {
                self.jit_declarations.insert(
                    storage_key(declaration.ident.span),
                    declaration.ident.sym.to_string(),
                );
            }
            if let Some(class) = declaration.function.return_type.as_ref().and_then(|annotation| self.annotated_class(&annotation.type_ann)) {
                self.native_results.insert(storage_key(declaration.ident.span), class);
            }
        }
        if declaration.function.body.is_none() && dynamic_symbol(declaration.ident.sym.as_ref()).is_some_and(|(backend, _)| !matches!(backend, DynamicBackend::Jit | DynamicBackend::ThawJs)) {
            self.expose(declaration.ident.span);
        }
        declaration.visit_children_with(self);
    }

    fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
        self.push_scope();
        self.functions.push((function.span, self.scopes.len() - 1));
        if function.body.is_some() {
            self.callable(function.span,
                &function.params.iter().map(|p| p.pat.clone()).collect::<Vec<_>>(),
                function.this_param.as_deref(),
                function.return_type.as_ref().map(|ann| ann.type_ann.as_ref()));
        }
        if function.return_type.as_ref().is_some_and(|ann| storage_has_any(&self.resolve_annotation(&ann.type_ann))) {
            self.seeds.insert((function.span.lo.0, 0));
        }
        for parameter in &function.params {
            self.pattern(&parameter.pat);

        }
        if let Some(body) = &function.body { self.hoist_vars(&body.stmts); }
        function.visit_children_with(self);
        self.functions.pop();
        self.pop_scope();
    }

    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        self.push_scope();
        self.functions.push((arrow.span, self.scopes.len() - 1));
        self.callable(arrow.span, &arrow.params, None,
            arrow.return_type.as_ref().map(|ann| ann.type_ann.as_ref()));
        if arrow.return_type.as_ref().is_some_and(|ann| storage_has_any(&self.resolve_annotation(&ann.type_ann))) {
            self.seeds.insert((arrow.span.lo.0, 0));
        }
        for parameter in &arrow.params {
            self.pattern(parameter);

        }
        if let ArrowFunctionBody::Expr(body) = arrow.body.as_ref() {
            self.link_keys((arrow.span.lo.0, 0), storage_key(body.span()));
            if self.is_uninit_sink(body) { self.sink_results.insert((arrow.span.lo.0, 0)); }
        }
        if let ArrowFunctionBody::FunctionBody(body) = arrow.body.as_ref() { self.hoist_vars(&body.stmts); }
        arrow.visit_children_with(self);
        self.functions.pop();
        self.pop_scope();
    }

    fn visit_return_stmt(&mut self, statement: &swc_ecma_ast::ReturnStmt) {
        if let (Some(&(function, _)), Some(value)) = (self.functions.last(), &statement.arg) {
            self.link_keys((function.lo.0, 0), storage_key(value.span()));
            if self.is_uninit_sink(value) {
                self.sink_results.insert((function.lo.0, 0));
            }
        }
        statement.visit_children_with(self);
    }

    fn visit_try_stmt(&mut self, statement: &swc_ecma_ast::TryStmt) {
        let exception = statement.handler.as_ref().and_then(|catch| catch.param.as_ref()).map(Spanned::span);
        if let Some(exception) = exception { self.exceptions.push(exception); }
        statement.block.visit_with(self);
        if exception.is_some() { self.exceptions.pop(); }
        statement.handler.visit_with(self);
        statement.finalizer.visit_with(self);
    }

    fn visit_catch_clause(&mut self, catch: &swc_ecma_ast::CatchClause) {
        self.push_scope();
        if let Some(parameter) = &catch.param { self.pattern(parameter); self.expose(parameter.span()); }
        catch.visit_children_with(self);
        self.pop_scope();
    }

    fn visit_throw_stmt(&mut self, statement: &swc_ecma_ast::ThrowStmt) {
        if let Some(&exception) = self.exceptions.last() { self.link(exception, statement.arg.span()); }
        statement.visit_children_with(self);
    }

    fn visit_getter_prop(&mut self, getter: &swc_ecma_ast::GetterProp) {
        self.link(getter.span, getter.function.span);
        getter.visit_children_with(self);
    }

    fn visit_setter_prop(&mut self, setter: &swc_ecma_ast::SetterProp) {
        self.link(setter.span, setter.function.span);
        setter.visit_children_with(self);
    }

    fn visit_class_decl(&mut self, declaration: &ClassDecl) {
        self.link(declaration.ident.span, declaration.class.span);
        if let Some(base) = &declaration.class.super_class { self.contain(declaration.class.span, base.span()); }
        self.classes.push(declaration.class.span);
        declaration.visit_children_with(self);
        self.classes.pop();
    }

    fn visit_constructor(&mut self, constructor: &swc_ecma_ast::Constructor) {
        if let Some(&class) = self.classes.last() { self.link(class, constructor.span); }
        let parameters: Vec<_> = constructor.params.iter().map(class_constructor_param_pattern).collect();
        self.push_scope();
        self.functions.push((constructor.span, self.scopes.len() - 1));
        self.callable(constructor.span, &parameters, None, None);
        for parameter in &parameters { self.pattern(parameter); }
        if let Some(body) = &constructor.body { self.hoist_vars(&body.stmts); }
        constructor.visit_children_with(self);
        self.functions.pop();
        self.pop_scope();
    }

    fn visit_class_method(&mut self, method: &ClassMethod) {
        if let Some(&class) = self.classes.last() { self.link(class, method.function.span); }
        method.visit_children_with(self);
    }

    fn visit_class_prop(&mut self, property: &ClassProp) {
        if let Some(&class) = self.classes.last() {
            self.contain(class, property.span);
        }
        if let Some(value) = &property.value { self.link(property.span, value.span()); }
        self.annotation(property.span, property.type_ann.as_deref());
        property.visit_children_with(self);
    }

    fn visit_expr(&mut self, expression: &Expr) {
        let span = expression.span();
        match expression {
            Expr::Array(array) => {
                for element in array.elems.iter().flatten() { self.contain(span, element.expr.span()); }
            }
            Expr::Object(object) => {
                for property in &object.props {
                    match property {
                        PropOrSpread::Spread(spread) => self.contain(span, spread.expr.span()),
                        PropOrSpread::Prop(property) => match property.as_ref() {
                            Prop::KeyValue(property) => self.contain(span, property.value.span()),
                            Prop::Shorthand(identifier) => self.contain(span, identifier.span),
                            Prop::Method(method) => self.link(span, method.function.span),
                            Prop::Getter(getter) => self.link(span, getter.span),
                            Prop::Setter(setter) => self.link(span, setter.span),
                            Prop::Assign(assign) => self.link(span, assign.value.span()),
                        }
                    }
                }
            }
            Expr::Assign(assign) => {
                self.link(span, assign.left.span());
                self.link(span, assign.right.span());
            }
            Expr::Call(call) => {
                if let Callee::Expr(callee) = &call.callee {
                    self.calls.push(StorageCall { callee: callee.span(),
                        arguments: call.args.iter().map(|arg| (arg.expr.span(), arg.spread.is_some())).collect(), result: span,
                        source_function: self.source_callable(callee) });
                    if let Expr::Ident(name) = callee.as_ref() {
                        // (thaw-js copies plain data at the call, so its arguments need no live dynamic storage.)
                        if dynamic_symbol(name.sym.as_ref()).is_some_and(|(backend, _)| !matches!(backend, DynamicBackend::Jit | DynamicBackend::ThawJs)) {
                            for argument in &call.args { self.expose(argument.expr.span()); }
                        }
                    }
                    if let Expr::Member(member) = callee.as_ref() {
                        // A static builtin that builds a fresh container from its arguments
                        // (`Object.fromEntries(..)`, `Buffer.from([..])`) does not alias the argument: its result
                        // reaching `any` must not turn the argument literal dynamic.
                        if matches!(&member.prop, swc_ecma_ast::MemberProp::Ident(method)
                            if matches!(method.sym.as_ref(), "push" | "unshift" | "add" | "set"))
                            && call.args.iter().any(|argument| self.is_uninit_sink(&argument.expr))
                        {
                            // The receiver's variable holds `undefined`-capable elements.
                            if let Expr::Ident(receiver) = member.obj.as_ref() {
                                if let Some((binding, _)) = self.lookup(receiver.sym.as_ref()) {
                                    self.sink_slots.insert(storage_key(binding));
                                }
                            }
                        }
                        if !self.is_fresh_static_builtin(member) {
                            self.link(span, member.obj.span());
                            for argument in &call.args { self.contain(member.obj.span(), argument.expr.span()); }
                        }
                    }
                }
            }
            Expr::New(new) => {
                self.link(span, new.callee.span());
                self.calls.push(StorageCall { callee: new.callee.span(),
                    arguments: new.args.iter().flatten().map(|argument| (argument.expr.span(), argument.spread.is_some())).collect(), result: span, source_function: self.source_constructor(&new.callee) });
                if matches!(new.callee.as_ref(), Expr::Ident(name)
                    if matches!(name.sym.as_ref(), "Proxy" | "Function") && self.lookup(name.sym.as_ref()).is_none()) {
                    self.expose(span);
                }
                for argument in new.args.iter().flatten() { self.contain(span, argument.expr.span()); }
            }
            Expr::Bin(binary) if matches!(binary.op, swc_ecma_ast::BinaryOp::LogicalOr
                | swc_ecma_ast::BinaryOp::LogicalAnd | swc_ecma_ast::BinaryOp::NullishCoalescing) => {
                self.link(span, binary.left.span());
                self.link(span, binary.right.span());
            }
            Expr::OptChain(chain) => match chain.base.as_ref() {
                swc_ecma_ast::OptChainBase::Member(member) => self.link(span, member.span),
                swc_ecma_ast::OptChainBase::Call(call) => {
                    self.calls.push(StorageCall { callee: call.callee.span(),
                        arguments: call.args.iter().map(|argument| (argument.expr.span(), argument.spread.is_some())).collect(), result: span, source_function: None });
                }
            },
            Expr::Cond(conditional) => {
                self.link(span, conditional.cons.span());
                self.link(span, conditional.alt.span());
            }
            Expr::Paren(parenthesized) => self.link(span, parenthesized.expr.span()),
            Expr::TsAs(assertion) => {
                self.link(span, assertion.expr.span());
                if storage_has_any(&self.resolve_annotation(&assertion.type_ann)) { self.expose(span); }
            }
            Expr::TsTypeAssertion(assertion) => {
                self.link(span, assertion.expr.span());
                if storage_has_any(&self.resolve_annotation(&assertion.type_ann)) { self.expose(span); }
            }
            Expr::TsNonNull(assertion) => self.link(span, assertion.expr.span()),
            Expr::Await(awaited) => self.link(span, awaited.arg.span()),
            Expr::Fn(function) => self.link(span, function.function.span),
            Expr::Seq(sequence) => {
                if let Some(last) = sequence.exprs.last() { self.link(span, last.span()); }
            }
            Expr::This(_) => {
                if let Some(&class) = self.classes.last() { self.link(span, class); }
            }
            _ => {}
        }
        expression.visit_children_with(self);
    }
}

thread_local! {
    /// Whether this lowering attempt also keeps `undefined` through returns, variable
    /// initialisers, container writes and object literals (see `lower_module`'s retry).
    static EXTENDED_STORAGE: Cell<bool> = const { Cell::new(false) };
    /// Whether the extended analysis changed anything (otherwise a retry would repeat itself).
    static EXTENDED_STORAGE_USED: Cell<bool> = const { Cell::new(false) };
}

/// A `number` field of a structural (non-class) object that a class may leave unset: a live
/// view over such an instance reads it as the marker NaN (see `UNSET_NUMBER_BITS`).
fn is_unset_number_field(fields: &[(Symbol, HirType)], property: &str) -> bool {
    !matches!(fields.first(), Some((marker, HirType::Bool)) if marker.starts_with("__thaw_class_identity_\u{1e}"))
        && fields.iter().any(|(name, ty)| name == property && *ty == HirType::F64)
        && ACTIVE_STORAGE_PLAN.with(|active| active.borrow().uninit_names.contains(property))
}

/// The type of a slot that receives a sink read (`undefined` survives): scalars become `any`,
/// containers keep their shape with `any` elements.
fn widen_sink_type(source: HirType) -> HirType {
    match source {
        HirType::F64 | HirType::I64 | HirType::Bool | HirType::Str => HirType::Json,
        HirType::Array(_) => HirType::Array(Box::new(HirType::Json)),
        HirType::Map(key, _) => HirType::Map(key, Box::new(HirType::Json)),
        HirType::Set(_) => HirType::Set(Box::new(HirType::Json)),
        other => other,
    }
}

/// A variable declared where a sink read (`arr.push(p.n)`, `const v = p.n`) is stored.
fn is_sink_slot(span: Span) -> bool {
    !span.is_dummy()
        && ACTIVE_STORAGE_PLAN.with(|active| active.borrow().sink_slots.contains(&storage_key(span)))
}

fn is_dynamic_read(span: Span) -> bool {
    !span.is_dummy()
        && ACTIVE_STORAGE_PLAN.with(|active| active.borrow().dynamic_reads.contains(&storage_key(span)))
}

thread_local! {
    static ACTIVE_STORAGE_PLAN: RefCell<StoragePlan> = RefCell::new(StoragePlan::default());
}

struct StorageScope(StoragePlan);

impl StorageScope {
    fn suspend() -> Self {
        Self(ACTIVE_STORAGE_PLAN.with(|active| active.replace(StoragePlan::default())))
    }

    fn enter(module: &Module) -> Result<Self, String> {
        let plan = StoragePlan::analyze(module)?;
        Ok(Self(ACTIVE_STORAGE_PLAN.with(|active| active.replace(plan))))
    }
}

impl Drop for StorageScope {
    fn drop(&mut self) {
        ACTIVE_STORAGE_PLAN.with(|active| active.replace(std::mem::take(&mut self.0)));
    }
}

fn has_dynamic_storage(span: Span) -> bool {
    ACTIVE_STORAGE_PLAN.with(|active| active.borrow().is_dynamic(span))
}

fn is_quickjs_function_value(span: Span) -> bool {
    !span.is_dummy()
        && ACTIVE_STORAGE_PLAN.with(|active| {
            active
                .borrow()
                .quickjs_function_value_nodes
                .contains(&storage_key(span))
        })
}

fn is_quickjs_function_value_slot(span: Span) -> bool {
    !span.is_dummy()
        && ACTIVE_STORAGE_PLAN.with(|active| {
            active
                .borrow()
                .quickjs_function_value_slots
                .contains(&storage_key(span))
        })
}

fn is_quickjs_function_value_result_slot(span: Span) -> bool {
    !span.is_dummy()
        && ACTIVE_STORAGE_PLAN.with(|active| {
            active
                .borrow()
                .quickjs_function_value_slots
                .contains(&(span.lo.0, 0))
        })
}

fn is_quickjs_function_value_expression(expression: &Expr) -> bool {
    match expression {
        Expr::Paren(value) => is_quickjs_function_value_expression(&value.expr),
        Expr::TsAs(value) => is_quickjs_function_value_expression(&value.expr),
        Expr::TsTypeAssertion(value) => is_quickjs_function_value_expression(&value.expr),
        Expr::TsNonNull(value) => is_quickjs_function_value_expression(&value.expr),
        _ => is_quickjs_function_value(expression.span()),
    }
}

fn is_quickjs_function_source(source_start: u32) -> bool {
    source_start != 0
        && ACTIVE_STORAGE_PLAN.with(|active| {
            active
                .borrow()
                .quickjs_function_sources
                .contains(&source_start)
        })
}

fn quickjs_function_source_type(source_start: u32, source: HirType) -> HirType {
    if is_quickjs_function_source(source_start) {
        quickjs_function_value_type(source)
    } else {
        source
    }
}

fn quickjs_function_value_type(source: HirType) -> HirType {
    fn contains_reference(ty: &HirType) -> bool {
        match ty {
            HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Dictionary(_)
            | HirType::Bytes => true,
            HirType::Optional(inner)
            | HirType::Nullable(inner)
            | HirType::Nullish(inner)
            | HirType::Promise(inner) => contains_reference(inner),
            HirType::Union(types) => types.iter().any(contains_reference),
            _ => false,
        }
    }
    fn reference_slot(ty: HirType) -> HirType {
        match ty {
            HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Dictionary(_)
            | HirType::Bytes => HirType::Json,
            HirType::Optional(inner) => {
                HirType::Optional(Box::new(reference_slot(*inner)))
            }
            HirType::Nullable(inner) => {
                HirType::Nullable(Box::new(reference_slot(*inner)))
            }
            HirType::Nullish(inner) => {
                HirType::Nullish(Box::new(reference_slot(*inner)))
            }
            HirType::Promise(inner) => {
                HirType::Promise(Box::new(reference_slot(*inner)))
            }
            HirType::Union(types) if types.iter().any(contains_reference) => HirType::Json,
            other => other,
        }
    }
    fn result_slot(ty: HirType) -> HirType {
        match ty {
            HirType::Promise(inner) => HirType::Promise(Box::new(result_slot(*inner))),
            other => reference_slot(other),
        }
    }

    match source {
        HirType::Function(params, ret) => HirType::Function(
            params.into_iter().map(reference_slot).collect(),
            Box::new(result_slot(*ret)),
        ),
        HirType::CallableFunction(params, optional, rest, ret) => {
            HirType::CallableFunction(
                params
                    .into_iter()
                    .map(reference_slot)
                    .collect(),
                optional,
                rest,
                Box::new(result_slot(*ret)),
            )
        }
        HirType::FunctionWithThis(receiver, inner) => HirType::FunctionWithThis(
            receiver,
            Box::new(quickjs_function_value_type(*inner)),
        ),
        other => other,
    }
}

fn storage_source_type(span: Span) -> Option<HirType> {
    if span.is_dummy() { return None; }
    ACTIVE_STORAGE_PLAN.with(|active| { let active = active.borrow();
        (!active.source_conflicts.contains(&storage_key(span))).then(|| active.source_types.get(&storage_key(span)).cloned()).flatten()
    })
}
/// Collects member reads whose value is observed as `undefined` rather than
/// consumed as a number/string: `console.log` arguments, template
/// substitutions, `typeof`/`!`/`if`/`?:` tests, `+`/equality/`??` operands, `?.` receivers.
#[derive(Default)]
struct ObservedMembers {
    members: HashSet<(u32, u32)>,
    names: HashSet<String>,
    objects: bool,
    tables: ValueKinds,
    extended: bool,
    /// Member reads in sinks that keep `undefined` (extended analysis): span and property name.
    sinks: Vec<(Span, String)>,
}

impl ObservedMembers {
    /// A member read whose `undefined` must survive (`return p.n`, `arr.push(p.n)`, ...).
    fn sink(&mut self, expr: &Expr) {
        if !self.extended {
            return;
        }
        match expr {
            Expr::Paren(paren) => self.sink(&paren.expr),
            Expr::TsAs(cast) => self.sink(&cast.expr),
            Expr::TsNonNull(cast) => self.sink(&cast.expr),
            Expr::Member(member) => {
                if let swc_ecma_ast::MemberProp::Ident(name) = &member.prop {
                    if !matches!(member.obj.as_ref(), Expr::Ident(object) if matches!(object.sym.as_ref(), "Math" | "Number" | "JSON" | "Object")) {
                        self.sinks.push((member.span, name.sym.to_string()));
                    }
                }
            }
            _ => {}
        }
    }

    fn observe(&mut self, expr: &Expr) {
        match expr {
            Expr::Paren(paren) => self.observe(&paren.expr),
            Expr::Member(member) => {
                self.members.insert(storage_key(member.span));
                if let swc_ecma_ast::MemberProp::Ident(name) = &member.prop {
                    self.names.insert(name.sym.to_string());
                }
            }
            _ => {}
        }
    }

    /// `console.log(items.map(x => x.n))` / `console.log([x.n])`: the member value
    /// flows straight into the printed array, so an uninitialised field keeps its
    /// `undefined` (other `.map` results stay on the lenient number path).
    fn observe_mapped_members(&mut self, expr: &Expr) {
        if let Expr::Array(array) = expr {
            for element in array.elems.iter().flatten().filter(|element| element.spread.is_none()) {
                self.observe(&element.expr);
            }
        }
        let Expr::Call(call) = expr else { return };
        let swc_ecma_ast::Callee::Expr(callee) = &call.callee else { return };
        let Expr::Member(member) = callee.as_ref() else { return };
        if matches!(&member.prop, swc_ecma_ast::MemberProp::Ident(name) if name.sym == *"map") {
            if let Some(Expr::Arrow(arrow)) = call.args.first().map(|argument| argument.expr.as_ref()) {
                if let swc_ecma_ast::ArrowFunctionBody::Expr(body) = arrow.body.as_ref() {
                    self.observe(body);
                }
            }
        }
        self.observe_mapped_members(&member.obj);
    }
}

impl Visit for ObservedMembers {
    fn visit_call_expr(&mut self, call: &swc_ecma_ast::CallExpr) {
        if let swc_ecma_ast::Callee::Expr(callee) = &call.callee {
            if let Expr::Member(member) = callee.as_ref() {
                // Only a call that can be handed a class instance exposes its
                // `undefined` fields (`console.log(1 + 2)` cannot).
                if matches!(member.obj.as_ref(), Expr::Ident(ident)
                    if matches!(ident.sym.as_ref(), "console" | "JSON" | "Object"))
                    && call.args.iter().any(|argument| self.tables.may_carry_object(&argument.expr))
                {
                    self.objects = true;
                }
                if matches!(member.obj.as_ref(), Expr::Ident(ident) if ident.sym == *"console") {
                    for argument in &call.args {
                        self.observe(&argument.expr);
                        self.observe_mapped_members(&argument.expr);
                    }
                }
            }
            if let Expr::Member(member) = callee.as_ref() {
                if matches!(&member.prop, swc_ecma_ast::MemberProp::Ident(method)
                    if matches!(method.sym.as_ref(), "push" | "unshift" | "add" | "set"))
                {
                    for argument in &call.args {
                        self.sink(&argument.expr);
                    }
                }
            }
            // Arguments of a plain-identifier call keep `undefined` (`id(p.n)`,
            // `String(p.n)`); a typed `number` parameter unwraps leniently at the
            // coercion. Member calls (`arr.push(p.n)`) go to builtin lowerings that
            // only accept the bare payload, so they stay lenient.
            if matches!(callee.as_ref(), Expr::Ident(ident) if !matches!(ident.sym.as_ref(),
                "Number" | "parseInt" | "parseFloat" | "isNaN" | "isFinite" | "BigInt" | "Boolean"
                | "Symbol" | "Array" | "Object" | "setTimeout" | "setInterval" | "require"))
            {
                for argument in &call.args {
                    if argument.spread.is_none() {
                        self.observe(&argument.expr);
                    }
                }
            }
        }
        call.visit_children_with(self);
    }
    fn visit_tpl(&mut self, template: &swc_ecma_ast::Tpl) {
        for expr in &template.exprs {
            self.observe(expr);
        }
        template.visit_children_with(self);
    }
    fn visit_return_stmt(&mut self, statement: &swc_ecma_ast::ReturnStmt) {
        if let Some(value) = &statement.arg {
            self.sink(value);
        }
        statement.visit_children_with(self);
    }
    fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
        if let swc_ecma_ast::ArrowFunctionBody::Expr(body) = arrow.body.as_ref() {
            self.sink(body);
        }
        arrow.visit_children_with(self);
    }
    fn visit_key_value_prop(&mut self, property: &swc_ecma_ast::KeyValueProp) {
        self.sink(&property.value);
        property.visit_children_with(self);
    }
    fn visit_array_lit(&mut self, array: &swc_ecma_ast::ArrayLit) {
        for element in array.elems.iter().flatten().filter(|element| element.spread.is_none()) {
            self.sink(&element.expr);
        }
        array.visit_children_with(self);
    }
    fn visit_if_stmt(&mut self, statement: &swc_ecma_ast::IfStmt) {
        self.observe(&statement.test);
        statement.visit_children_with(self);
    }
    fn visit_cond_expr(&mut self, conditional: &swc_ecma_ast::CondExpr) {
        self.observe(&conditional.test);
        conditional.visit_children_with(self);
    }
    fn visit_unary_expr(&mut self, unary: &swc_ecma_ast::UnaryExpr) {
        // `typeof x.f` and `!x.f` (truthiness of an absent array/object).
        if matches!(unary.op, swc_ecma_ast::UnaryOp::TypeOf | swc_ecma_ast::UnaryOp::Bang) {
            self.observe(&unary.arg);
        }
        unary.visit_children_with(self);
    }
    fn visit_bin_expr(&mut self, binary: &swc_ecma_ast::BinExpr) {
        use swc_ecma_ast::BinaryOp::*;
        if matches!(binary.op, Add | EqEq | NotEq | EqEqEq | NotEqEq | NullishCoalescing) {
            self.observe(&binary.left);
            self.observe(&binary.right);
        }
        binary.visit_children_with(self);
    }
    fn visit_ts_as_expr(&mut self, cast: &swc_ecma_ast::TsAsExpr) {
        self.observe(&cast.expr);
        cast.visit_children_with(self);
    }
    fn visit_var_declarator(&mut self, declarator: &swc_ecma_ast::VarDeclarator) {
        let annotated = matches!(&declarator.name, swc_ecma_ast::Pat::Ident(ident) if ident.type_ann.is_some());
        if let (true, Some(init)) = (annotated, &declarator.init) {
            self.observe(init);
        }
        if let (swc_ecma_ast::Pat::Ident(_), Some(init)) = (&declarator.name, &declarator.init) {
            self.sink(init);
        }
        declarator.visit_children_with(self);
    }
    fn visit_opt_chain_expr(&mut self, chain: &swc_ecma_ast::OptChainExpr) {
        if let swc_ecma_ast::OptChainBase::Member(member) = chain.base.as_ref() {
            self.observe(&member.obj);
        }
        chain.visit_children_with(self);
    }
}

/// Whether a constructor-less class's bare `x: T;` must be widened: the module
/// could print/serialise the instance or read `x` in an observing position.
fn bare_field_observed(class: &str, field: &str) -> bool {
    ACTIVE_STORAGE_PLAN.with(|active| {
        let active = active.borrow();
        active.instantiated_classes.contains(class)
            && (active.observes_objects || active.observed_names.contains(field))
    })
}
fn is_observed_member(span: Span) -> bool {
    ACTIVE_STORAGE_PLAN.with(|active| active.borrow().observed_members.contains(&storage_key(span)))
}
fn remember_uninit_class_field(class: &str, field: &str) {
    ACTIVE_STORAGE_PLAN.with(|active| {
        active.borrow_mut().uninit_fields.insert((class.to_owned(), field.to_owned()));
    });
}
/// Whether `field` of a class-instance layout (identified by its identity
/// marker) was widened because the constructor may leave it unset.
fn is_uninit_class_field(layout: &[(Symbol, HirType)], field: &str) -> bool {
    let Some((marker, HirType::Bool)) = layout.first() else { return false };
    let Some(chain) = marker.strip_prefix("__thaw_class_identity_\u{1e}") else { return false };
    ACTIVE_STORAGE_PLAN.with(|active| {
        let active = active.borrow();
        chain.split('\u{1f}').any(|class| active.uninit_fields.contains(&(class.to_owned(), field.to_owned())))
    })
}
fn remember_class_source_field(class: &str, field: &str, span: Span, optional: bool) {
    ACTIVE_STORAGE_PLAN.with(|active| {
        active.borrow_mut().class_fields.insert((class.to_owned(), field.to_owned()), (span, optional));
    });
}

fn storage_source_type_from_plan(span: Span, active: &StoragePlan) -> Option<HirType> {
    let key = storage_key(span);
    (!active.source_conflicts.contains(&key)).then(|| active.source_types.get(&key).cloned()).flatten()
}

fn source_class_contract(ty: &HirType, active: &StoragePlan, seen: &mut HashSet<String>) -> HirType {
    match ty {
        HirType::Object(fields) => {
            let identities = fields.first().and_then(|(marker, tag)| {
                (*tag == HirType::Bool)
                    .then(|| marker.strip_prefix("__thaw_class_identity_\u{1e}"))
                    .flatten()
            });
            let mut restored = fields.iter().map(|(name, field)| {
                (name.clone(), source_class_contract(field, active, seen))
            }).collect::<Vec<_>>();
            let mut restored_names = HashSet::new();
            if let Some(identities) = identities {
                for class in identities.split('\u{1f}') {
                    if seen.insert(class.to_owned()) {
                        for ((owner, name), (span, optional)) in &active.class_fields {
                            if owner == class && restored_names.insert(name.clone()) {
                                if let Some(mut source) = storage_source_type_from_plan(*span, active) {
                                    if *optional { source = optional_parameter_type(source); }
                                    let source = source_class_contract(&source, active, seen);
                                    if let Some((_, field)) = restored.iter_mut().find(|(existing, _)| existing == name) {
                                        *field = source;
                                    } else {
                                        restored.push((name.clone(), source));
                                    }
                                }
                            }
                        }
                        seen.remove(class);
                    }
                }
            }
            HirType::Object(restored)
        }
        HirType::Array(inner) => HirType::Array(Box::new(source_class_contract(inner, active, seen))),
        HirType::Tuple(types) => HirType::Tuple(types.iter().map(|ty| source_class_contract(ty, active, seen)).collect()),
        HirType::Optional(inner) => HirType::Optional(Box::new(source_class_contract(inner, active, seen))),
        HirType::Nullable(inner) => HirType::Nullable(Box::new(source_class_contract(inner, active, seen))),
        HirType::Nullish(inner) => HirType::Nullish(Box::new(source_class_contract(inner, active, seen))),
        HirType::Union(types) => HirType::Union(types.iter().map(|ty| source_class_contract(ty, active, seen)).collect()),
        other => other.clone(),
    }
}
fn storage_source_return_type(source_start: u32) -> Option<HirType> {
    if source_start == 0 { return None; }
    let key = (source_start, 0);
    ACTIVE_STORAGE_PLAN.with(|active| {
        let active = active.borrow();
        (!active.source_conflicts.contains(&key))
            .then(|| active.source_types.get(&key).cloned())
            .flatten()
    })
}
fn storage_source_callable_parameters(source_start: u32, fallback: &[HirType]) -> Vec<HirType> {
    if source_start == 0 {
        return fallback.to_vec();
    }
    ACTIVE_STORAGE_PLAN.with(|active| {
        let active = active.borrow();
        let Some(parameters) = active.callable_parameters.get(&source_start) else {
            return fallback.to_vec();
        };
        fallback
            .iter()
            .enumerate()
            .map(|(index, fallback)| {
                parameters
                    .get(index)
                    .filter(|key| !active.source_conflicts.contains(key))
                    .and_then(|key| active.source_types.get(key))
                    .cloned()
                    .unwrap_or_else(|| fallback.clone())
            })
            .collect()
    })
}
fn remember_storage_source(key: (u32, u32), source: &HirType) {
    if key.0 == 0 { return; }
    ACTIVE_STORAGE_PLAN.with(|active| {
        let mut active = active.borrow_mut();
        match active.source_types.get(&key) {
            None | Some(HirType::Dynamic) => { active.source_types.insert(key, source.clone()); }
            Some(previous) if previous != source && *source != HirType::Dynamic => { active.source_conflicts.insert(key); }
            _ => {}
        }
    });
}
fn storage_type(span: Span, source: HirType) -> HirType {
    remember_storage_source(storage_key(span), &source);
    if is_sink_slot(span) {
        return widen_sink_type(source);
    }
    if is_quickjs_function_value(span)
        && matches!(
            &source,
            HirType::Function(..) | HirType::CallableFunction(..) | HirType::FunctionWithThis(..)
                | HirType::OverloadedFunction(..)
        )
    {
        return quickjs_function_value_type(source);
    }
    if is_quickjs_function_value_slot(span) {
        return quickjs_reference_slot_type(source);
    }
    if !has_dynamic_storage(span) { return source; }
    dynamic_storage_type(source)
}

fn quickjs_reference_slot_type(source: HirType) -> HirType {
    fn contains_reference(ty: &HirType) -> bool {
        match ty {
            HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Dictionary(_)
            | HirType::Bytes => true,
            HirType::Optional(inner)
            | HirType::Nullable(inner)
            | HirType::Nullish(inner)
            | HirType::Promise(inner) => contains_reference(inner),
            HirType::Union(types) => types.iter().any(contains_reference),
            _ => false,
        }
    }
    match source {
        HirType::Array(_)
        | HirType::Tuple(_)
        | HirType::Object(_)
        | HirType::Dictionary(_)
        | HirType::Bytes => HirType::Json,
        HirType::Optional(inner) => HirType::Optional(Box::new(quickjs_reference_slot_type(*inner))),
        HirType::Nullable(inner) => HirType::Nullable(Box::new(quickjs_reference_slot_type(*inner))),
        HirType::Nullish(inner) => HirType::Nullish(Box::new(quickjs_reference_slot_type(*inner))),
        HirType::Promise(inner) => HirType::Promise(Box::new(quickjs_reference_slot_type(*inner))),
        HirType::Union(types) if types.iter().any(contains_reference) => HirType::Json,
        other => other,
    }
}

fn dynamic_storage_type(source: HirType) -> HirType {
    match source {
        HirType::Void | HirType::Dynamic => source,
        HirType::Promise(result) => HirType::Promise(Box::new(dynamic_storage_type(*result))),
        HirType::Array(element) if matches!(element.as_ref(), HirType::Promise(_)) => {
            HirType::Array(Box::new(dynamic_storage_type(*element)))
        }
        HirType::Function(..) | HirType::CallableFunction(..) | HirType::FunctionWithThis(..)
        | HirType::OverloadedFunction(..)
        | HirType::JsValue => source,
        _ => HirType::Json,
    }
}

enum ArrayLitSegment {
    Hole,
    Value(HirExpr, HirType),
    Spread(HirExpr, HirType),
}

impl FnLowerer<'_> {
    fn lower_dynamic_storage_array(&mut self, segments: Vec<ArrayLitSegment>) -> Result<HirExpr, String> {
        let mut promise_element = None;
        let mut homogeneous_promises = true;
        for segment in &segments {
            let element = match segment {
                ArrayLitSegment::Hole => continue,
                ArrayLitSegment::Value(_, ty) | ArrayLitSegment::Spread(_, ty) => ty,
            };
            if !matches!(element, HirType::Promise(_)) {
                homogeneous_promises = false;
                break;
            }
            if promise_element.as_ref().is_some_and(|previous| previous != element) {
                homogeneous_promises = false;
                break;
            }
            promise_element = Some(element.clone());
        }
        if homogeneous_promises {
            if let Some(element) = promise_element {
                let parts = segments.into_iter().map(|segment| match segment {
                    ArrayLitSegment::Hole => HirExpr::ArrayLit(vec![HirExpr::Lit(HirLit::ArrayHole)]),
                    ArrayLitSegment::Value(value, _) => HirExpr::ArrayLit(vec![value]),
                    ArrayLitSegment::Spread(value, _) => value,
                }).collect::<Vec<_>>();
                if !parts.iter().any(contains_await) {
                    return Ok(HirExpr::ArrayConcat(parts, element));
                }
                let parts = parts.into_iter().flat_map(|part| match part {
                    HirExpr::ArrayLit(values) => values.into_iter().map(|value| HirExpr::ArrayLit(vec![value])).collect(),
                    other => vec![other],
                }).collect::<Vec<_>>();
                let mut bindings = Vec::with_capacity(parts.len());
                let mut ordered = Vec::with_capacity(parts.len());
                for (position, part) in parts.into_iter().enumerate() {
                    let ty = self.infer_expr_type(&part)?;
                    let name = format!("__thaw_array_part_{}_{}", position, self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    bindings.push((name.clone(), ty, part));
                    ordered.push(HirExpr::Var(name));
                }
                return self.wrap_call_argument_bindings(
                    HirExpr::ArrayConcat(ordered, element),
                    &bindings,
                );
            }
        }

        let mut parts = Vec::new();
        for segment in segments {
            let part = match segment {
                ArrayLitSegment::Hole => HirExpr::TypedClosure(
                    HirType::Array(Box::new(HirType::Json)),
                    Box::new(HirExpr::ArrayLit(vec![HirExpr::Lit(HirLit::ArrayHole)])),
                ),
                ArrayLitSegment::Value(value, _) => {
                    let value = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
                    HirExpr::TypedClosure(
                        HirType::Array(Box::new(HirType::Json)),
                        Box::new(HirExpr::ArrayLit(vec![value])),
                    )
                }
                ArrayLitSegment::Spread(value, _) => {
                    let value = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
                    let value = HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_map_or_set_entries".into())), vec![value]);
                    HirExpr::JsonAsNative(Box::new(value), HirType::Array(Box::new(HirType::Json)))
                }
            };
            parts.push(part);
        }
        let native = HirExpr::ArrayConcat(parts, HirType::Json);
        let array = self.wrap_native_value_as_json(native, HirType::Array(Box::new(HirType::Json)))?;
        self.retain_dynamic_storage_value(array)
    }

    fn retain_dynamic_storage_value(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let arguments = self.wrap_native_value_as_json(HirExpr::ArrayLit(vec![value]), HirType::Array(Box::new(HirType::Json)))?;
        let live = HirExpr::Call(Box::new(HirExpr::Var("callDynamicValueMixedHandle".into())), vec![
            HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())),
                vec![HirExpr::Lit(HirLit::Str("__thaw_storage_value".into()))]),
            arguments, HirExpr::ArrayLit(Vec::new()),
        ]);
        Ok(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_host_from_dynamic".into())), vec![live]))
    }
}

fn contains_native_class(ty: &HirType) -> bool {
    match ty {
        HirType::Object(fields) => {
            class_name_from_type(ty).is_some() || fields.iter().any(|(_, ty)| contains_native_class(ty))
        }
        HirType::Array(inner)
        | HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner) => contains_native_class(inner),
        HirType::Tuple(types) | HirType::Union(types) => types.iter().any(contains_native_class),
        _ => false,
    }
}

fn storage_reference_type(ty: &TsType) -> bool {
    !matches!(ty, TsType::TsKeywordType(keyword)
        if !matches!(keyword.kind, TsKeywordTypeKind::TsAnyKeyword | TsKeywordTypeKind::TsObjectKeyword | TsKeywordTypeKind::TsUnknownKeyword))
        && !matches!(ty, TsType::TsFnOrConstructorType(_))
}
fn storage_parameter_type(pattern: &Pat, source: HirType) -> HirType {
    remember_storage_source(storage_key(pattern.span()), &source);
    if is_quickjs_function_value_slot(pattern.span()) {
        return quickjs_reference_slot_type(source);
    }
    if matches!(pattern, Pat::Rest(_)) && has_dynamic_storage(pattern.span()) {
        return HirType::Array(Box::new(HirType::Json));
    }
    if matches!(pattern, Pat::Ident(binding) if binding.id.optional) {
        if let HirType::Optional(payload) = source {
            return HirType::Optional(Box::new(storage_type(pattern.span(), *payload)));
        }
    }
    storage_type(pattern.span(), source)
}
fn storage_return_type(span: Span, source: HirType) -> HirType {
    remember_storage_source((span.lo.0, 0), &source);
    if ACTIVE_STORAGE_PLAN.with(|active| active.borrow().sink_results.contains(&(span.lo.0, 0))) {
        return widen_sink_type(source);
    }
    if is_quickjs_function_value(span) || is_quickjs_function_value_result_slot(span) {
        return quickjs_reference_slot_type(source);
    }
    if ACTIVE_STORAGE_PLAN.with(|active| active.borrow().result_is_dynamic(span)) {
        if is_generator_producer_type(&source) {
            source
        } else if matches!(source, HirType::Function(..) | HirType::CallableFunction(..) | HirType::OverloadedFunction(..)) {
            HirType::Dynamic
        } else { dynamic_storage_type(source) }
    } else { source }
}

impl FnLowerer<'_> {
    fn lower_dynamic_storage_constructor(&mut self, new: &swc_ecma_ast::NewExpr) -> Result<HirExpr, String> {
        let Expr::Ident(name) = new.callee.as_ref() else { unreachable!() };
        let mut arguments = Vec::new();
        for argument in new.args.iter().flatten() {
            let value = self.lower_expr(&argument.expr)?;
            if argument.spread.is_some() {
                let value = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
                arguments.push(HirExpr::JsonAsNative(Box::new(value), HirType::Array(Box::new(HirType::Json))));
            } else {
                let value = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
                arguments.push(HirExpr::TypedClosure(HirType::Array(Box::new(HirType::Json)), Box::new(HirExpr::ArrayLit(vec![value]))));
            }
        }
        let arguments = self.wrap_native_value_as_json(HirExpr::ArrayConcat(arguments, HirType::Json), HirType::Array(Box::new(HirType::Json)))?;
        let value = HirExpr::Call(Box::new(HirExpr::Var("constructDynamicValue".into())), vec![
            HirExpr::Call(Box::new(HirExpr::Var("getDynamicValue".into())), vec![HirExpr::Lit(HirLit::Str(name.sym.to_string()))]), arguments,
        ]);
        Ok(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_host_from_dynamic".into())), vec![value]))
    }
}

impl FnLowerer<'_> {
    fn coerce_native_ffi_argument(&mut self, value: HirExpr, declared: &HirType) -> Result<HirExpr, String> {
        fn contains_dynamic(ty: &HirType) -> bool {
            match ty {
                HirType::Json | HirType::JsValue | HirType::Dictionary(_) => true,
                HirType::Object(fields) => fields.iter().any(|(_, ty)| contains_dynamic(ty)),
                HirType::Array(inner) | HirType::Optional(inner) | HirType::Nullable(inner)
                | HirType::Nullish(inner) => contains_dynamic(inner),
                _ => false,
            }
        }
        let actual = self.infer_expr_type(&value)?;
        if actual != *declared && !matches!(declared, HirType::Json | HirType::JsValue | HirType::Dictionary(_))
            && contains_dynamic(&actual) {
            let json = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
            return Ok(HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_decode_native_argument".into())),
                vec![HirExpr::TypedClosure(declared.clone(), Box::new(json))]));
        }
        self.coerce_primitive_array_argument(value, declared)
    }

    fn coerce_jit_structural_class_argument(
        &mut self,
        value: HirExpr,
        declared: &HirType,
    ) -> Result<HirExpr, String> {
        let json = self.coerce_to_declared_aliasing(&HirType::Json, value)?;
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_decode_native_argument".into())),
            vec![HirExpr::TypedClosure(
                declared.clone(),
                Box::new(json),
            )],
        ))
    }
}

impl FnLowerer<'_> {
    fn source_contract_type_for_expression(&self, expression: &Expr) -> Option<HirType> {
        let restore = |ty| ACTIVE_STORAGE_PLAN.with(|active| {
            source_class_contract(&ty, &active.borrow(), &mut HashSet::new())
        });
        match expression {
            Expr::Ident(identifier) => {
                let name = self.resolve_binding(identifier.sym.as_ref());
                self.source_scope.get(&name).cloned()
                    .or_else(|| storage_source_type(identifier.span))
                    .or_else(|| self.scope.get(&name).cloned())
                    .map(restore)
            }
            Expr::This(_) => {
                let name = self.resolve_binding("this");
                self.source_scope
                    .get(&name)
                    .cloned()
                    .or_else(|| {
                        self.active_non_arrow_this
                            .as_ref()
                            .filter(|(active, _)| active == &name)
                            .map(|(_, declared)| declared.clone())
                    })
                    .or_else(|| {
                        self.scope.get(&name).cloned().map(|ty| match ty {
                            HirType::NonArrowReceiver(receiver) => *receiver,
                            other => other,
                        })
                    })
                    .map(restore)
            }
            Expr::Paren(value) => self.source_contract_type_for_expression(&value.expr),
            Expr::TsNonNull(value) => self.source_contract_type_for_expression(&value.expr),
            Expr::TsAs(value) => lower_ts_type(&value.type_ann, self.interfaces, self.generic_interfaces).ok().map(restore),
            Expr::TsTypeAssertion(value) => lower_ts_type(&value.type_ann, self.interfaces, self.generic_interfaces).ok().map(restore),
            Expr::Member(member) => {
                if let Some(source) = storage_source_type(member.span) { return Some(restore(source)); }
                let HirType::Object(fields) = self.source_contract_type_for_expression(&member.obj)? else { return None };
                let MemberProp::Ident(property) = &member.prop else { return None };
                fields.into_iter().find(|(field, _)| field == property.sym.as_ref()).map(|(_, ty)| restore(ty))
            }
            Expr::New(new_expr) => {
                let Expr::Ident(class) = new_expr.callee.as_ref() else { return None };
                let name = self.resolve_binding(class.sym.as_ref());
                let restored = self.interfaces.get(&name).cloned().map(restore);
                restored
            }
            _ => None,
        }
    }

    fn source_value_hint(&self, expression: &Expr) -> Option<HirExpr> {
        fn uncertain(ty: &HirType) -> bool {
            match ty {
                HirType::Json | HirType::JsValue | HirType::Dynamic | HirType::Dictionary(_) => true,
                HirType::Object(fields) => fields.iter().any(|(_, ty)| uncertain(ty)),
                HirType::Array(ty) | HirType::Optional(ty) | HirType::Nullable(ty) | HirType::Nullish(ty) => uncertain(ty),
                HirType::Tuple(types) | HirType::Union(types) => types.iter().any(uncertain),
                _ => false,
            }
        }
        let typed = |ty| HirExpr::TypedClosure(if uncertain(&ty) { HirType::Json } else { ty }, Box::new(HirExpr::Lit(HirLit::Undefined)));
        let source_typed = |ty| {
            HirExpr::TypedClosure(
                if contains_native_class(&ty) { ty } else if uncertain(&ty) { HirType::Json } else { ty },
                Box::new(HirExpr::Lit(HirLit::Undefined)),
            )
        };
        match expression {
            Expr::Lit(Lit::Num(value)) => Some(HirExpr::Lit(HirLit::F64(value.value))),
            Expr::Lit(Lit::Str(value)) => Some(HirExpr::Lit(HirLit::Str(value.value.to_string_lossy().into_owned()))),
            Expr::Lit(Lit::Bool(value)) => Some(HirExpr::Lit(HirLit::Bool(value.value))),
            Expr::Lit(Lit::Null(_)) => Some(HirExpr::Lit(HirLit::Null)),
            Expr::Lit(Lit::BigInt(value)) => value.value.to_string().parse().ok().map(|value| HirExpr::Lit(HirLit::I64(value))),
            Expr::Ident(identifier) => {
                let name = self.resolve_binding(identifier.sym.as_ref());
                let narrowed = self.union_narrowings.get(&name).and_then(|(allowed, elements)| {
                    if let [index] = allowed.as_slice() {
                        elements.get(*index).cloned()
                    } else if elements.last() == Some(&HirType::Undefined)
                        && allowed.len() + 1 == elements.len()
                        && allowed.iter().copied().eq(0..allowed.len())
                    {
                        Some(HirType::Union(elements[..allowed.len()].to_vec()))
                    } else {
                        None
                    }
                }).or_else(|| self.json_narrowings.get(&name).cloned());
                narrowed.or_else(|| self.source_contract_type_for_expression(expression)).map(source_typed)
                    .or_else(|| {
                        let signature = self.signatures.get(&name)?;
                        (signature.is_extern
                            && signature.generic_type_params.is_empty()
                            && is_quickjs_function_source(signature.source_range.0))
                        .then(|| {
                            let ret = if signature.is_async {
                                HirType::Promise(Box::new(signature.ret.clone()))
                            } else {
                                signature.ret.clone()
                            };
                            typed(HirType::Function(signature.params.clone(), Box::new(ret)))
                        })
                    })
            }
            Expr::This(_) => {
                self.source_contract_type_for_expression(expression).map(source_typed)
            }
            Expr::Paren(value) => self.source_value_hint(&value.expr),
            Expr::TsNonNull(value) => self.source_value_hint(&value.expr),
            Expr::TsAs(value) => lower_ts_type(&value.type_ann, self.interfaces, self.generic_interfaces).ok().map(typed),
            Expr::TsTypeAssertion(value) => lower_ts_type(&value.type_ann, self.interfaces, self.generic_interfaces).ok().map(typed),
            Expr::Array(array) if array.elems.iter().flatten().all(|element| element.spread.is_none()) => {
                Some(HirExpr::ArrayLit(array.elems.iter().map(|element| match element {
                    None => HirExpr::Lit(HirLit::ArrayHole),
                    Some(element) => self.source_value_hint(&element.expr).unwrap_or_else(|| typed(HirType::Json)),
                }).collect()))
            }
            Expr::Object(object) if !object_literal_has_proto_setter(object) => {
                let mut fields = Vec::new();
                for property in &object.props {
                    let PropOrSpread::Prop(property) = property else { return None };
                    match property.as_ref() {
                        Prop::KeyValue(property) => fields.push((class_property_name(&property.key).ok()?, self.source_value_hint(&property.value).unwrap_or_else(|| typed(HirType::Json)))),
                        Prop::Shorthand(identifier) => fields.push((identifier.sym.to_string(), self.source_value_hint(&Expr::Ident(identifier.clone())).unwrap_or_else(|| typed(HirType::Json)))),
                        _ => return None,
                    }
                }
                Some(HirExpr::ObjectLit(fields))
            }
            Expr::Member(member) => {
                if let Some(ty) = self.source_contract_type_for_expression(expression) { return Some(source_typed(ty)); }
                let owner = self.source_value_hint(&member.obj)?;
                let ty = self.infer_expr_type(&owner).ok()?;
                match (&ty, &member.prop) {
                    (HirType::Object(fields), MemberProp::Ident(property)) => fields.iter().find(|(name, _)| name == property.sym.as_ref()).map(|(_, ty)| typed(ty.clone())),
                    (HirType::Array(element), MemberProp::Computed(_)) => Some(typed(*element.clone())),
                    (HirType::Json | HirType::JsValue, _) => Some(typed(HirType::Json)),
                    _ => None,
                }
            }
            Expr::New(_) if self.source_contract_type_for_expression(expression).is_some() => {
                self.source_contract_type_for_expression(expression).map(source_typed)
            }
            Expr::New(new_expr)
                if matches!(new_expr.callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "Date")
                    && !self.scope.contains_key(&self.resolve_binding("Date"))
                    && !self.signatures.contains_key(&self.resolve_binding("Date")) =>
            {
                Some(typed(HirType::Object(vec![
                    (
                        "__thaw_class_identity_\u{1e}Date".to_string(),
                        HirType::Bool,
                    ),
                    ("timestamp".to_string(), HirType::F64),
                ])))
            }
            _ => None,
        }
    }

    fn generic_call_argument_source_type(
        &self,
        source: Option<&Expr>,
        physical: &HirExpr,
        preserve_class_physical: bool,
    ) -> Result<HirType, String> {
        let physical_type = self.infer_expr_type(physical)?;
        if preserve_class_physical && contains_native_class(&physical_type) {
            return Ok(physical_type);
        }
        if physical_type != HirType::JsValue {
            if let Some(source) = source.filter(|source| has_dynamic_storage(source.span())) {
                if let Some(source_type) = self
                    .source_value_hint(source)
                    .and_then(|hint| self.infer_expr_type(&hint).ok())
                    .filter(|source_type| {
                        !(matches!(source_type, HirType::Json | HirType::Dynamic | HirType::JsValue)
                            || preserve_class_physical && contains_native_class(source_type))
                    })
                {
                    return Ok(source_type);
                }
            }
        }
        Ok(physical_type)
    }

    fn validate_source_assignable(&mut self, expected: &HirType, actual: &HirType) -> Result<(), String> {
        if matches!(actual, HirType::Json | HirType::JsValue | HirType::Dynamic) {
            return Ok(());
        }
        if let HirType::Union(members) = actual {
            for member in members {
                self.validate_source_assignable(expected, member)?;
            }
            return Ok(());
        }
        if let HirType::Union(members) = expected {
            if members.iter().any(|member| self.validate_source_assignable(member, actual).is_ok()) {
                return Ok(());
            }
            return Err("value does not satisfy any member of the declared union".to_string());
        }
        match (expected, actual) {
            (HirType::Optional(_), HirType::Undefined)
            | (HirType::Nullable(_), HirType::Null)
            | (HirType::Nullish(_), HirType::Null | HirType::Undefined) => return Ok(()),
            (HirType::Optional(expected), HirType::Optional(actual))
            | (HirType::Nullable(expected), HirType::Nullable(actual))
            | (HirType::Nullish(expected), HirType::Nullish(actual)) => {
                return self.validate_source_assignable(expected, actual);
            }
            (HirType::Nullish(expected), HirType::Optional(actual) | HirType::Nullable(actual)) => {
                return self.validate_source_assignable(expected, actual);
            }
            (HirType::Optional(expected), actual) => {
                if matches!(actual, HirType::Nullable(_) | HirType::Nullish(_)) {
                    return Err(format!("value has type {actual:?}, expected {expected:?} or undefined"));
                }
                return self.validate_source_assignable(expected, actual);
            }
            (HirType::Nullable(expected), actual) if !matches!(actual, HirType::Nullish(_) | HirType::Optional(_)) && *actual != HirType::Null => {
                return self.validate_source_assignable(expected, actual);
            }
            (HirType::Nullish(expected), actual) if !matches!(actual, HirType::Null | HirType::Undefined) => {
                return self.validate_source_assignable(expected, actual);
            }
            (HirType::Array(expected), HirType::Array(actual)) => {
                return self.validate_source_assignable(expected, actual);
            }
            (HirType::Array(expected), HirType::Tuple(actual)) => {
                for element in actual { self.validate_source_assignable(expected, element)?; }
                return Ok(());
            }
            (HirType::Tuple(expected), HirType::Tuple(actual)) => {
                let required = expected.iter().take_while(|ty| omitted_parameter_value(ty).is_err()).count();
                if actual.len() < required || actual.len() > expected.len() {
                    return Err(format!("value has tuple length {}, expected {}..={}", actual.len(), required, expected.len()));
                }
                for (expected, actual) in expected.iter().zip(actual) {
                    self.validate_source_assignable(expected, actual)?;
                }
                return Ok(());
            }
            (HirType::Object(expected), HirType::Object(actual)) => {
                if let Some(class) = class_name_from_type(&HirType::Object(expected.clone())) {
                    return if class_type_has_identity(&HirType::Object(actual.clone()), class) {
                        Ok(())
                    } else {
                        let actual_name = class_name_from_type(&HirType::Object(actual.clone()))
                            .map(|name| format!("class `{name}`"))
                            .unwrap_or_else(|| format!("value of type {actual:?}"));
                        Err(format!("{actual_name} does not satisfy nominal class `{class}`"))
                    };
                }
                let public = |name: &str| !name.starts_with("__thaw_class_identity_\u{1e}")
                    && !name.starts_with("__thaw_private_") && !is_hidden_accessor_field(name);
                for (name, expected_field) in expected.iter().filter(|(name, _)| public(name)) {
                    if let Some((_, actual_field)) = actual.iter().find(|(actual_name, _)| actual_name == name && public(actual_name)) {
                        self.validate_source_assignable(expected_field, actual_field)
                            .map_err(|error| format!("field `{name}`: {error}"))?;
                    } else if omitted_parameter_value(expected_field).is_err() {
                        let actual_class = HirType::Object(actual.clone());
                        if let Some(class) = class_name_from_type(&actual_class) {
                            if self.class_method_signature_compatible(class, name, expected_field, true) {
                                continue;
                            }
                            if self.class_method_signature_exists(class, name) {
                                return Err(format!("field `{name}` has an incompatible method signature"));
                            }
                        }
                        return Err(format!("missing required field `{name}`"));
                    }
                }
                return Ok(());
            }
            _ => {}
        }
        self.coerce_to_declared(expected, HirExpr::TypedClosure(
            actual.clone(), Box::new(HirExpr::Lit(HirLit::Undefined)),
        )).map(|_| ())
    }

    fn validate_storage_source_type(&mut self, expected: &HirType, source: HirExpr) -> Result<(), String> {
        let actual = self.infer_expr_type(&source)?;
        if matches!(actual, HirType::Json | HirType::JsValue | HirType::Dynamic) { return Ok(()); }
        if let HirType::Union(members) = expected {
            for member in members {
                if self.validate_storage_source_type(member, source.clone()).is_ok() {
                    return Ok(());
                }
            }
            return self.validate_source_assignable(expected, &actual);
        }
        if matches!(&source, HirExpr::ObjectLit(_) | HirExpr::ArrayLit(_)) {
            if let HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) = expected {
                return self.validate_storage_source_type(inner, source);
            }
        }
        if matches!((expected, &source), (HirType::Dictionary(_), HirExpr::ObjectLit(_))) {
            return self.coerce_to_declared(expected, source).map(|_| ());
        }
        if matches!((expected, &actual), (HirType::Tuple(_), HirType::Array(_))) {
            return self.coerce_to_declared(expected, source).map(|_| ());
        }
        if let (HirType::Object(expected_fields), HirExpr::ObjectLit(values)) = (expected, &source) {
            if class_name_from_type(expected).is_some() {
                return self.validate_source_assignable(expected, &actual);
            }
            for (name, expected_field) in expected_fields {
                if let Some((_, value)) = values.iter().find(|(field, _)| field == name) {
                    self.validate_storage_source_type(expected_field, value.clone())
                        .map_err(|error| format!("field `{name}`: {error}"))?;
                } else if omitted_parameter_value(expected_field).is_err() {
                    return Err(format!("missing required field `{name}`"));
                }
            }
            return Ok(());
        }
        if let (HirType::Array(element), HirExpr::ArrayLit(values)) = (expected, &source) {
            for value in values {
                if !matches!(value, HirExpr::Lit(HirLit::ArrayHole)) {
                    self.validate_storage_source_type(element, value.clone())?;
                }
            }
            return Ok(());
        }
        self.validate_source_assignable(expected, &actual)
    }

    fn validate_storage_source(&mut self, expression: &Expr, expected: &HirType) -> Result<(), String> {
        if matches!(expected, HirType::Json | HirType::JsValue | HirType::Dynamic | HirType::Void) { return Ok(()); }
        // A sink read of a maybe-uninitialised field is `undefined` until assigned, whatever its declared type.
        if matches!(peel_parens(expression), Expr::Member(member) if is_dynamic_read(member.span)) { return Ok(()); }
        let (expected_params, expected_return) = match expected {
            HirType::Function(params, ret)
            | HirType::CallableFunction(params, _, _, ret) => (params.as_slice(), ret.as_ref()),
            _ => (&[][..], expected),
        };
        let (actual_params, actual_return, is_async) = match expression {
            Expr::Arrow(arrow) => (
                arrow.params.iter().collect::<Vec<_>>(),
                arrow.return_type.as_deref().map(|annotation| annotation.type_ann.as_ref()),
                arrow.is_async,
            ),
            Expr::Fn(function) => (
                function.function.params.iter().map(|parameter| &parameter.pat).collect(),
                function.function.return_type.as_deref().map(|annotation| annotation.type_ann.as_ref()),
                function.function.is_async,
            ),
            _ => (Vec::new(), None, false),
        };
        if !expected_params.is_empty() || matches!(expected, HirType::Function(..) | HirType::CallableFunction(..)) {
            for (index, actual) in actual_params.iter().enumerate() {
                let is_rest = matches!(actual, Pat::Rest(_));
                let expected_params = if is_rest {
                    expected_params.get(index..).unwrap_or_default()
                } else {
                    expected_params.get(index..index + 1).unwrap_or_default()
                };
                if expected_params.is_empty() {
                    continue;
                }
                let annotation = match actual {
                    Pat::Ident(binding) => binding.type_ann.as_deref(),
                    Pat::Rest(rest) => rest.type_ann.as_deref(),
                    Pat::Array(array) => array.type_ann.as_deref(),
                    Pat::Object(object) => object.type_ann.as_deref(),
                    _ => None,
                };
                if let Some(annotation) = annotation {
                    let mut actual_type = lower_ts_type(&annotation.type_ann, self.interfaces, self.generic_interfaces)?;
                    if is_rest {
                        actual_type = match actual_type {
                            HirType::Array(element) => *element,
                            other => other,
                        };
                    }
                    for (offset, expected) in expected_params.iter().enumerate() {
                        if actual_type != HirType::Json
                            && !callback_param_compatible(expected, &actual_type)
                        {
                            return Err(format!(
                                "contextual callback parameter {} declares {actual_type:?}, expected {expected:?}",
                                index + offset + 1
                            ));
                        }
                    }
                }
            }
            if let Some(annotation) = actual_return {
                let mut actual = lower_ts_type(annotation, self.interfaces, self.generic_interfaces)?;
                let mut expected = expected_return.clone();
                if is_async {
                    if !matches!(expected, HirType::Promise(_)) {
                        expected = HirType::Promise(Box::new(expected));
                    }
                    if !matches!(actual, HirType::Promise(_)) {
                        actual = HirType::Promise(Box::new(actual));
                    }
                }
                if !callable_return_compatible(&expected, &actual) {
                    return Err(format!(
                        "contextual callback declares return type {actual:?}, expected {expected:?}"
                    ));
                }
            }
        }
        if let Some(source) = self.source_value_hint(expression) {
            self.validate_storage_source_type(expected, source)?;
        }
        Ok(())
    }

    fn validate_quickjs_function_value_arguments(
        &mut self,
        target: &Expr,
        arguments: &[swc_ecma_ast::ExprOrSpread],
    ) -> Result<bool, String> {
        if !is_quickjs_function_value_expression(target) {
            return Ok(false);
        }
        let Some(source) = self.source_value_hint(target) else {
            return Ok(true);
        };
        let source_type = self.infer_expr_type(&source)?;
        let params = match source_type {
            HirType::Function(params, _) => params,
            HirType::CallableFunction(params, _, rest, _) if rest.is_none() => params,
            _ => return Ok(true),
        };
        let mut parameter_index = 0;
        for argument in arguments {
            if argument.spread.is_some() {
                if let Expr::Array(array) = &*argument.expr {
                    if array.elems.iter().flatten().all(|element| element.spread.is_none()) {
                        for element in &array.elems {
                            if let Some(element) = element {
                                if let Some(expected) = params.get(parameter_index) {
                                    self.validate_storage_source(&element.expr, expected)?;
                                }
                            }
                            parameter_index += 1;
                        }
                    }
                } else if let Some(source) = self.source_value_hint(&argument.expr) {
                    match self.infer_expr_type(&source)? {
                        HirType::Tuple(elements) => {
                            for element in elements {
                                if let Some(expected) = params.get(parameter_index) {
                                    self.validate_storage_source_type(
                                        expected,
                                        HirExpr::TypedClosure(
                                            element,
                                            Box::new(HirExpr::Lit(HirLit::Undefined)),
                                        ),
                                    )?;
                                }
                                parameter_index += 1;
                            }
                        }
                        HirType::Array(element) => {
                            if let Some(expected) = params.get(parameter_index) {
                                self.validate_storage_source_type(
                                    expected,
                                    HirExpr::TypedClosure(
                                        *element,
                                        Box::new(HirExpr::Lit(HirLit::Undefined)),
                                    ),
                                )?;
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            if let Some(expected) = params.get(parameter_index) {
                self.validate_storage_source(&argument.expr, expected)?;
            }
            parameter_index += 1;
        }
        Ok(true)
    }

    fn validate_storage_call_source(&mut self, expression: &Expr) -> Result<(), String> {
        let expected = ACTIVE_STORAGE_PLAN.with(|active| {
            let active = active.borrow();
            let parameters = active.argument_parameters.get(&storage_key(expression.span()))?;
            let mut candidates = parameters.iter().filter(|parameter| !active.source_conflicts.contains(parameter)).filter_map(|parameter| active.source_types.get(parameter));
            let expected = candidates.next()?.clone();
            candidates.all(|candidate| *candidate == expected).then_some(expected)
        });
        if let Some(expected) = expected { self.validate_storage_source(expression, &expected)?; }
        Ok(())
    }
}

impl FnLowerer<'_> {
    fn remember_source_parameter(&mut self, name: &str, pattern: &Pat) {
        if let Some(source) = storage_source_type(pattern.span()) { self.source_scope.insert(name.to_string(), source); }
    }
    fn remember_source_return(&mut self, span: Span) {
        self.source_ret_type = if span.is_dummy() { None } else { ACTIVE_STORAGE_PLAN.with(|active| {
            let active = active.borrow(); let key = (span.lo.0, 0);
            (!active.source_conflicts.contains(&key)).then(|| active.source_types.get(&key).cloned()).flatten()
        }) };
    }
}

impl FnLowerer<'_> {
    fn adapt_native_ffi_result(&mut self, value: HirExpr, span: Span) -> Result<HirExpr, String> {
        let source = self.infer_expr_type(&value)?;
        if matches!(source, HirType::Json | HirType::JsValue) { return Ok(value); }
        let layout_changed = class_name_from_type(&source).and_then(|class| self.interfaces.get(class)).is_some_and(|target| *target != source);
        if !has_dynamic_storage(span) && !layout_changed { return Ok(value); }
        self.dynamic_native_ffi_result(value, &source)
    }

    fn dynamic_native_ffi_result(&mut self, value: HirExpr, source: &HirType) -> Result<HirExpr, String> {
        let name = format!("__thaw_ffi_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), source.clone());
        let current = HirExpr::Var(name.clone());
        let result = match source {
            HirType::Array(element) => {
                let item = format!("__thaw_ffi_element_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(item.clone(), *element.clone());
                let body = self.dynamic_native_ffi_result(HirExpr::Var(item.clone()), element)?;
                let callback = HirExpr::Lambda(Vec::new(), vec![HirParam { name: item, ty: *element.clone() }], HirType::Json, Box::new(body));
                let mapped = self.lower_array_map(current, source.clone(), *element.clone(), *element.clone(), callback, None)?;
                let json = self.wrap_native_value_as_json(mapped, HirType::Array(Box::new(HirType::Json)))?;
                self.retain_dynamic_storage_value(json)?
            }
            HirType::Tuple(elements) => {
                let mut values = Vec::new();
                for (index, element) in elements.iter().enumerate() {
                    let child = HirExpr::TypedIndex(Box::new(current.clone()), Box::new(HirExpr::Lit(HirLit::F64(index as f64))), element.clone());
                    values.push(self.dynamic_native_ffi_result(child, element)?);
                }
                let json = self.wrap_native_value_as_json(HirExpr::ArrayLit(values), HirType::Array(Box::new(HirType::Json)))?;
                self.retain_dynamic_storage_value(json)?
            }
            HirType::Object(fields) => {
                if let Some(target) = class_name_from_type(source).and_then(|class| self.interfaces.get(class)).cloned() {
                    let HirType::Object(target_fields) = &target else { unreachable!() };
                    let mut values = Vec::new();
                    for (field, expected) in target_fields {
                        let actual = fields.iter().find(|(name, _)| name == field).map(|(_, ty)| ty).ok_or("native return is missing an internal class field")?;
                        let child = HirExpr::PropAccess(Box::new(current.clone()), source.clone(), field.clone());
                        let child = if *expected == HirType::Json { self.dynamic_native_ffi_result(child, actual)? } else { self.coerce_to_declared(expected, child)? };
                        values.push((field.clone(), child));
                    }
                    let target_name = format!("__thaw_ffi_internal_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(target_name.clone(), target.clone());
                    let projected = self.coerce_to_declared_aliasing(&HirType::Json, HirExpr::Var(target_name.clone()))?;
                    let public_fields: Vec<_> = target_fields.iter().filter(|(field, _)|
                        !field.starts_with("__thaw_class_identity_\u{1e}") && !field.starts_with("__thaw_private_") && !is_hidden_accessor_field(field)).cloned().collect();
                    let plain_values = public_fields.iter().map(|(field, _)| (field.clone(), HirExpr::PropAccess(Box::new(HirExpr::Var(target_name.clone())), target.clone(), field.clone()))).collect();
                    let plain = self.wrap_native_value_as_json(HirExpr::ObjectLit(plain_values), HirType::Object(public_fields))?;
                    let plain = self.retain_dynamic_storage_value(plain)?;
                    let authentic = HirExpr::Call(Box::new(HirExpr::Var("__thaw_object_copy_class_identity".into())), vec![current, HirExpr::Var(target_name.clone())]);
                    HirExpr::Block(vec![
                        HirStmt::Let(target_name, target.clone(), HirExpr::TypedClosure(target, Box::new(HirExpr::ObjectLit(values)))),
                        HirStmt::Return(Some(HirExpr::Conditional(Box::new(authentic), Box::new(projected), Box::new(plain), HirType::Json))),
                    ])
                } else {
                    let mut values = Vec::new();
                    for (field, ty) in fields {
                        let child = HirExpr::PropAccess(Box::new(current.clone()), source.clone(), field.clone());
                        values.push((field.clone(), self.dynamic_native_ffi_result(child, ty)?));
                    }
                    let ty = HirType::Object(fields.iter().map(|(name, _)| (name.clone(), HirType::Json)).collect());
                    let json = self.wrap_native_value_as_json(HirExpr::ObjectLit(values), ty)?;
                    self.retain_dynamic_storage_value(json)?
                }
            }
            HirType::Optional(payload) => HirExpr::Conditional(
                Box::new(HirExpr::OptionalIsNone(Box::new(current.clone()), *payload.clone())),
                Box::new(self.coerce_to_declared(&HirType::Json, HirExpr::Lit(HirLit::Undefined))?),
                Box::new(self.dynamic_native_ffi_result(HirExpr::OptionalValue(Box::new(current), *payload.clone()), payload)?), HirType::Json),
            HirType::Nullable(payload) => HirExpr::Conditional(
                Box::new(HirExpr::NullableIsNone(Box::new(current.clone()), *payload.clone())),
                Box::new(self.coerce_to_declared(&HirType::Json, HirExpr::Lit(HirLit::Null))?),
                Box::new(self.dynamic_native_ffi_result(HirExpr::NullableValue(Box::new(current), *payload.clone()), payload)?), HirType::Json),
            HirType::JsValue => HirExpr::Call(Box::new(HirExpr::Var("__thaw_json_host_from_borrowed_dynamic".into())), vec![current]),
            _ => self.coerce_to_declared_aliasing(&HirType::Json, current)?,
        };
        self.wrap_call_argument_bindings(result, &[(name, source.clone(), value)])
    }
}

/// Collects the names JavaScript infers for anonymous function expressions
/// (NamedEvaluation): declarators, identifier assignments, property definitions,
/// class fields and parameter/destructuring defaults. Member assignments
/// (`this.f = () => 1`) infer nothing.
#[derive(Default)]
struct FunctionNames {
    names: HashMap<(u32, u32), String>,
}

impl FunctionNames {
    fn name(&mut self, expr: &Expr, name: String) {
        // Module flattening renames top-level bindings to `__thawmod<N>_<name>`.
        let name = match name.strip_prefix("__thawmod").and_then(|rest| rest.split_once('_')) {
            Some((digits, original)) if digits.bytes().all(|byte| byte.is_ascii_digit()) => original.to_string(),
            _ => name,
        };
        let mut inner = expr;
        loop {
            inner = match inner {
                Expr::Paren(paren) => &paren.expr,
                Expr::TsAs(cast) => &cast.expr,
                Expr::TsSatisfies(cast) => &cast.expr,
                Expr::TsNonNull(cast) => &cast.expr,
                Expr::TsTypeAssertion(cast) => &cast.expr,
                _ => break,
            };
        }
        match inner {
            Expr::Arrow(arrow) => {
                self.names.insert(storage_key(arrow.span), name);
            }
            Expr::Fn(function) if function.ident.is_none() => {
                self.names.insert(storage_key(function.function.span), name);
            }
            _ => {}
        }
    }

    fn property_name(key: &swc_ecma_ast::PropName) -> Option<String> {
        match key {
            swc_ecma_ast::PropName::Ident(ident) => Some(ident.sym.to_string()),
            swc_ecma_ast::PropName::Str(text) => Some(text.value.to_string_lossy().into_owned()),
            swc_ecma_ast::PropName::Num(number) if number.value.fract() == 0.0 && number.value.abs() < 1e15 => {
                Some(format!("{}", number.value as i64))
            }
            swc_ecma_ast::PropName::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(swc_ecma_ast::Lit::Str(text)) => Some(text.value.to_string_lossy().into_owned()),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Visit for FunctionNames {
    fn visit_var_declarator(&mut self, declarator: &swc_ecma_ast::VarDeclarator) {
        if let (swc_ecma_ast::Pat::Ident(binding), Some(init)) = (&declarator.name, &declarator.init) {
            self.name(init, binding.id.sym.to_string());
        }
        declarator.visit_children_with(self);
    }
    fn visit_assign_expr(&mut self, assign: &swc_ecma_ast::AssignExpr) {
        use swc_ecma_ast::AssignOp::*;
        if matches!(assign.op, Assign | AndAssign | OrAssign | NullishAssign) {
            if let swc_ecma_ast::AssignTarget::Simple(swc_ecma_ast::SimpleAssignTarget::Ident(target)) = &assign.left {
                self.name(&assign.right, target.id.sym.to_string());
            }
        }
        assign.visit_children_with(self);
    }
    fn visit_key_value_prop(&mut self, property: &swc_ecma_ast::KeyValueProp) {
        if let Some(name) = Self::property_name(&property.key) {
            self.name(&property.value, name);
        }
        property.visit_children_with(self);
    }
    fn visit_class_prop(&mut self, property: &swc_ecma_ast::ClassProp) {
        if let (Some(name), Some(value)) = (Self::property_name(&property.key), &property.value) {
            self.name(value, name);
        }
        property.visit_children_with(self);
    }
    fn visit_private_prop(&mut self, property: &swc_ecma_ast::PrivateProp) {
        if let Some(value) = &property.value {
            self.name(value, format!("#{}", property.key.name));
        }
        property.visit_children_with(self);
    }
    fn visit_assign_pat(&mut self, pattern: &swc_ecma_ast::AssignPat) {
        if let swc_ecma_ast::Pat::Ident(binding) = pattern.left.as_ref() {
            self.name(&pattern.right, binding.id.sym.to_string());
        }
        pattern.visit_children_with(self);
    }
}

/// The name a function expression at `span` prints with: its own, else the one JavaScript
/// infers from the surrounding definition. Only a program that can print a function value
/// (see `StoragePlan::function_names`) registers names.
fn function_expression_name(own: Option<&str>, span: Span) -> Option<String> {
    ACTIVE_STORAGE_PLAN.with(|active| {
        let active = active.borrow();
        if !active.names_functions {
            return None;
        }
        own.map(str::to_owned).or_else(|| active.function_names.get(&storage_key(span)).cloned())
    })
}

/// Syntactic facts about which names hold only primitives, so printing a primitive
/// does not count as exposing a class instance.
#[derive(Default)]
struct ValueKinds {
    /// Variables/parameters: `true` while every declaration is primitive-typed.
    variables: HashMap<String, bool>,
    /// Properties and methods (class fields, interface members, method returns).
    properties: HashMap<String, bool>,
    functions: HashMap<String, bool>,
    /// Functions returning a function: `true` while every such declaration's inner result is primitive.
    callable_returns: HashMap<String, bool>,
    instantiated: HashSet<String>,
    supers: HashMap<String, String>,
}

fn primitive_ts_type(ty: &TsType) -> bool {
    use swc_ecma_ast::TsKeywordTypeKind::*;
    match ty {
        TsType::TsKeywordType(keyword) => matches!(
            keyword.kind,
            TsNumberKeyword | TsStringKeyword | TsBooleanKeyword | TsBigIntKeyword | TsSymbolKeyword
                | TsNullKeyword | TsUndefinedKeyword | TsVoidKeyword | TsNeverKeyword
        ),
        TsType::TsLitType(_) => true,
        TsType::TsArrayType(array) => primitive_ts_type(&array.elem_type),
        TsType::TsParenthesizedType(inner) => primitive_ts_type(&inner.type_ann),
        TsType::TsUnionOrIntersectionType(swc_ecma_ast::TsUnionOrIntersectionType::TsUnionType(union)) => {
            union.types.iter().all(|member| primitive_ts_type(member))
        }
        _ => false,
    }
}

impl ValueKinds {
    fn note(table: &mut HashMap<String, bool>, name: String, primitive: bool) {
        let entry = table.entry(name).or_insert(true);
        *entry = *entry && primitive;
    }

    /// `Some(primitive result)` for a function type annotation (`(x: number) => number`).
    fn function_type_result(ty: &TsType) -> Option<bool> {
        match ty {
            TsType::TsFnOrConstructorType(swc_ecma_ast::TsFnOrConstructorType::TsFnType(function)) => {
                Some(primitive_ts_type(&function.type_ann.type_ann))
            }
            TsType::TsParenthesizedType(inner) => Self::function_type_result(&inner.type_ann),
            _ => None,
        }
    }

    fn annotated(annotation: Option<&swc_ecma_ast::TsTypeAnn>) -> bool {
        annotation.is_some_and(|annotation| primitive_ts_type(&annotation.type_ann))
    }

    fn primitive_literalish(expr: &Expr) -> bool {
        match expr {
            Expr::Paren(paren) => Self::primitive_literalish(&paren.expr),
            Expr::Lit(_) | Expr::Tpl(_) | Expr::Unary(_) | Expr::Update(_) => true,
            Expr::Bin(binary) => !matches!(
                binary.op,
                swc_ecma_ast::BinaryOp::LogicalOr
                    | swc_ecma_ast::BinaryOp::LogicalAnd
                    | swc_ecma_ast::BinaryOp::NullishCoalescing
            ),
            _ => false,
        }
    }

    /// Whether `expr` can evaluate to an object (a class instance, array of them, ...).
    fn may_carry_object(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Paren(paren) => self.may_carry_object(&paren.expr),
            Expr::TsAs(cast) => self.may_carry_object(&cast.expr),
            Expr::TsSatisfies(cast) => self.may_carry_object(&cast.expr),
            Expr::TsNonNull(cast) => self.may_carry_object(&cast.expr),
            Expr::TsTypeAssertion(cast) => self.may_carry_object(&cast.expr),
            Expr::Lit(_) | Expr::Tpl(_) | Expr::Unary(_) | Expr::Update(_) | Expr::Arrow(_) | Expr::Fn(_) => false,
            Expr::Bin(binary) => {
                matches!(
                    binary.op,
                    swc_ecma_ast::BinaryOp::LogicalOr
                        | swc_ecma_ast::BinaryOp::LogicalAnd
                        | swc_ecma_ast::BinaryOp::NullishCoalescing
                ) && (self.may_carry_object(&binary.left) || self.may_carry_object(&binary.right))
            }
            Expr::Cond(cond) => self.may_carry_object(&cond.cons) || self.may_carry_object(&cond.alt),
            Expr::Seq(sequence) => sequence.exprs.last().is_some_and(|last| self.may_carry_object(last)),
            Expr::Array(array) => array.elems.iter().flatten().any(|element| {
                element.spread.is_some() || self.may_carry_object(&element.expr)
            }),
            Expr::Object(object) => object.props.iter().any(|property| match property {
                swc_ecma_ast::PropOrSpread::Spread(_) => true,
                swc_ecma_ast::PropOrSpread::Prop(property) => match property.as_ref() {
                    swc_ecma_ast::Prop::KeyValue(pair) => self.may_carry_object(&pair.value),
                    swc_ecma_ast::Prop::Shorthand(ident) => self.variable_may_carry(ident.sym.as_ref()),
                    _ => true,
                },
            }),
            Expr::Ident(ident) => self.variable_may_carry(ident.sym.as_ref()),
            Expr::Member(member) => match &member.prop {
                swc_ecma_ast::MemberProp::Ident(name) => !self.property_is_primitive(name.sym.as_ref()),
                _ => true,
            },
            Expr::Call(call) => match &call.callee {
                swc_ecma_ast::Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(name) => {
                        !(matches!(name.sym.as_ref(), "Number" | "String" | "Boolean" | "parseInt" | "parseFloat"
                            | "isNaN" | "isFinite" | "BigInt")
                            || self.functions.get(name.sym.as_ref()) == Some(&true))
                    }
                    // `makeAdder()(1)`: the function a declaration returns yields a primitive.
                    Expr::Call(inner) => !matches!(&inner.callee, swc_ecma_ast::Callee::Expr(callee)
                        if matches!(callee.as_ref(), Expr::Ident(name)
                            if self.callable_returns.get(name.sym.as_ref()) == Some(&true))),
                    // `f.call(this, 1)` / `f.apply(this, [1])` of a primitive-returning function.
                    Expr::Member(member) if matches!((member.obj.as_ref(), &member.prop),
                        (Expr::Ident(object), swc_ecma_ast::MemberProp::Ident(method))
                            if matches!(method.sym.as_ref(), "call" | "apply")
                                && self.functions.get(object.sym.as_ref()) == Some(&true)) => false,
                    Expr::Member(member) => match (member.obj.as_ref(), &member.prop) {
                        (Expr::Ident(object), swc_ecma_ast::MemberProp::Ident(_))
                            if matches!(object.sym.as_ref(), "Math" | "Number" | "Date")
                                && !self.variables.contains_key(object.sym.as_ref()) => false,
                        (Expr::Ident(object), swc_ecma_ast::MemberProp::Ident(name))
                            if object.sym == *"JSON" && name.sym == *"stringify" => false,
                        (_, swc_ecma_ast::MemberProp::Ident(name)) => {
                            !(self.functions.get(name.sym.as_ref()) == Some(&true)
                                || (matches!(name.sym.as_ref(), "toString" | "toFixed" | "toUpperCase" | "toLowerCase"
                                    | "trim" | "join" | "charAt" | "padStart" | "padEnd" | "repeat" | "substring"
                                    | "indexOf" | "includes" | "startsWith" | "endsWith" | "has" | "test"
                                    | "localeCompare" | "toISOString" | "getTime")
                                    && !self.functions.contains_key(name.sym.as_ref())))
                        }
                        _ => true,
                    },
                    _ => true,
                },
                _ => true,
            },
            _ => true,
        }
    }

    fn variable_may_carry(&self, name: &str) -> bool {
        name != "undefined" && self.variables.get(name) != Some(&true)
    }

    fn property_is_primitive(&self, name: &str) -> bool {
        match self.properties.get(name) {
            Some(primitive) => *primitive,
            None => matches!(name, "length" | "size"),
        }
    }
}

impl Visit for ValueKinds {
    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
        // Declarations without an annotation are classified by their initializer
        // in `visit_var_declarator`; parameters/catch bindings with none are unknown.
        if binding.type_ann.is_some() {
            Self::note(&mut self.variables, binding.id.sym.to_string(), Self::annotated(binding.type_ann.as_deref()));
        } else {
            self.variables.entry(binding.id.sym.to_string()).or_insert(true);
        }
        binding.visit_children_with(self);
    }
    fn visit_var_declarator(&mut self, declarator: &swc_ecma_ast::VarDeclarator) {
        if let swc_ecma_ast::Pat::Ident(binding) = &declarator.name {
            if binding.type_ann.is_none() {
                let primitive = declarator.init.as_deref().is_some_and(Self::primitive_literalish);
                Self::note(&mut self.variables, binding.id.sym.to_string(), primitive);
            }
            // A variable holding a function: what calling it yields.
            let result = match (binding.type_ann.as_deref(), declarator.init.as_deref()) {
                (Some(annotation), _) => Self::function_type_result(&annotation.type_ann),
                (None, Some(Expr::Arrow(arrow))) => Some(Self::annotated(arrow.return_type.as_deref())),
                (None, Some(Expr::Fn(function))) => Some(Self::annotated(function.function.return_type.as_deref())),
                _ => None,
            };
            if let Some(primitive) = result {
                Self::note(&mut self.functions, binding.id.sym.to_string(), primitive);
            }
        }
        declarator.visit_children_with(self);
    }
    fn visit_pat(&mut self, pattern: &swc_ecma_ast::Pat) {
        // Destructured / rest / for-of bindings are unknown values.
        match pattern {
            swc_ecma_ast::Pat::Ident(_) => {}
            other => {
                struct Names<'a>(&'a mut HashMap<String, bool>);
                impl Visit for Names<'_> {
                    fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
                        ValueKinds::note(self.0, binding.id.sym.to_string(), false);
                    }
                }
                other.visit_with(&mut Names(&mut self.variables));
            }
        }
        pattern.visit_children_with(self);
    }
    fn visit_class_prop(&mut self, property: &ClassProp) {
        if let Ok(name) = class_property_name(&property.key) {
            let primitive = Self::annotated(property.type_ann.as_deref())
                || (property.type_ann.is_none() && property.value.as_deref().is_some_and(Self::primitive_literalish));
            Self::note(&mut self.properties, name, primitive);
        }
        property.visit_children_with(self);
    }
    fn visit_ts_property_signature(&mut self, signature: &swc_ecma_ast::TsPropertySignature) {
        if let Expr::Ident(name) = signature.key.as_ref() {
            Self::note(&mut self.properties, name.sym.to_string(), Self::annotated(signature.type_ann.as_deref()));
        }
        signature.visit_children_with(self);
    }
    fn visit_class_method(&mut self, method: &swc_ecma_ast::ClassMethod) {
        if let Ok(name) = class_property_name(&method.key) {
            Self::note(&mut self.functions, name.clone(), Self::annotated(method.function.return_type.as_deref()));
            if method.kind != swc_ecma_ast::MethodKind::Method {
                Self::note(&mut self.properties, name, Self::annotated(method.function.return_type.as_deref()));
            }
        }
        method.visit_children_with(self);
    }
    fn visit_fn_decl(&mut self, declaration: &swc_ecma_ast::FnDecl) {
        Self::note(
            &mut self.functions,
            declaration.ident.sym.to_string(),
            Self::annotated(declaration.function.return_type.as_deref()),
        );
        if let Some(primitive) = declaration
            .function
            .return_type
            .as_deref()
            .and_then(|annotation| Self::function_type_result(&annotation.type_ann))
        {
            Self::note(&mut self.callable_returns, declaration.ident.sym.to_string(), primitive);
        }
        declaration.visit_children_with(self);
    }
    fn visit_class_decl(&mut self, declaration: &ClassDecl) {
        if let Some(Expr::Ident(base)) = declaration.class.super_class.as_deref() {
            self.supers.insert(declaration.ident.sym.to_string(), base.sym.to_string());
        }
        declaration.visit_children_with(self);
    }
    fn visit_new_expr(&mut self, new: &swc_ecma_ast::NewExpr) {
        if let Expr::Ident(class) = new.callee.as_ref() {
            self.instantiated.insert(class.sym.to_string());
        }
        new.visit_children_with(self);
    }
}

/// Field names that some class may leave unset, computed with the same rule the class
/// layout uses (`maybe_uninit_fields_with`).
struct UninitNames<'a> {
    names: HashSet<String>,
    instantiated: &'a HashSet<String>,
    observed: &'a ObservedMembers,
}

impl Visit for UninitNames<'_> {
    fn visit_class_decl(&mut self, declaration: &ClassDecl) {
        let class = declaration.ident.sym.to_string();
        let observed = self.observed;
        let instantiated = self.instantiated;
        let fields = maybe_uninit_fields_with(&declaration.class, &|field| {
            instantiated.contains(&class) && (observed.objects || observed.names.contains(field))
        });
        self.names.extend(fields);
        declaration.visit_children_with(self);
    }
}
