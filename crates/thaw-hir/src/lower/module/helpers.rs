fn declaration_names_for_normalization(declaration: &Decl) -> Vec<String> {
    struct Collector(Vec<String>);
    impl Visit for Collector {
        fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
            self.0.push(binding.id.sym.to_string());
        }
    }
    let mut collector = Collector(Vec::new());
    declaration.visit_with(&mut collector);
    collector.0
}

/// One of the built-in error classes `Error`/etc. accept as a base even
/// though (unlike a real declared class) they have no entry of their own in
/// `declarations`/`interfaces` (see `module/classes.rs`'s synthetic
/// base-layout branch).
fn is_error_family_name(name: &str) -> bool {
    crate::is_error_family_name(name)
}

/// Standard-library constructors whose `.prototype` the realm exposes as a
/// real object. thaw models several of these only as call/new targets, so a
/// bare `Object`/`Array`/`Date`/... reference is otherwise an "unknown
/// variable"; `<name>.prototype` reads the live constructor off the realm
/// instead (see `crates/thaw-hir/src/lower/objects.rs`'s `prototype`
/// member-read branch) -- the shape most of test262's prototype tests use
/// (`Object.prototype.hasOwnProperty.call`, `Array.prototype.map.call`, ...).
fn is_builtin_prototype_owner(name: &str) -> bool {
    matches!(
        name,
        "Object"
            | "Function"
            | "Array"
            | "String"
            | "Number"
            | "Boolean"
            | "Symbol"
            | "BigInt"
            | "Date"
            | "RegExp"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "EvalError"
            | "URIError"
            | "AggregateError"
            | "SuppressedError"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "ArrayBuffer"
            | "SharedArrayBuffer"
            | "DataView"
            | "Int8Array"
            | "Uint8Array"
            | "Uint8ClampedArray"
            | "Int16Array"
            | "Uint16Array"
            | "Int32Array"
            | "Uint32Array"
            | "Float16Array"
            | "Float32Array"
            | "Float64Array"
            | "BigInt64Array"
            | "BigUint64Array"
            | "Promise"
            | "Iterator"
            | "AsyncIterator"
    )
}

/// Standard library / Web API type names thaw does not model natively. An
/// annotation naming one is treated as an opaque `JsValue` -- at runtime the
/// value is a real QuickJS object (thaw's platform globals provide these) --
/// rather than failing the build with the generic "unsupported type
/// reference" error. `.d.ts` files and user annotations reference
/// `URL`/`Response`/`AbortSignal`/... constantly.
fn is_opaque_global_type(name: &str) -> bool {
    matches!(
        name,
        // Web / WHATWG APIs.
        "URL"
            | "URLSearchParams"
            | "Headers"
            | "HeadersInit"
            | "Request"
            | "RequestInit"
            | "RequestInfo"
            | "RequestRedirect"
            | "Response"
            | "ResponseInit"
            | "ResponseType"
            | "BodyInit"
            | "Blob"
            | "BlobPart"
            | "File"
            | "FileList"
            | "FormData"
            | "FormDataEntryValue"
            | "Event"
            | "EventInit"
            | "EventTarget"
            | "CustomEvent"
            | "MessageEvent"
            | "MessageEventInit"
            | "MessagePort"
            | "MessageChannel"
            | "BroadcastChannel"
            | "CloseEvent"
            | "ErrorEvent"
            | "ProgressEvent"
            | "DOMException"
            | "AbortSignal"
            | "AbortController"
            | "ReadableStream"
            | "ReadableStreamDefaultReader"
            | "ReadableStreamReadResult"
            | "WritableStream"
            | "TransformStream"
            | "WebSocket"
            | "XMLHttpRequest"
            | "Worker"
            | "SharedWorker"
            | "Storage"
            | "Location"
            | "Navigator"
            | "Document"
            | "Window"
            | "ImageData"
            | "Performance"
            | "PerformanceEntry"
            | "DOMHighResTimeStamp"
            | "Crypto"
            | "CryptoKey"
            | "SubtleCrypto"
            | "CryptoKeyPair"
            | "Transferable"
            | "StructuredSerializeOptions"
            | "TextEncoder"
            | "TextDecoder"
            // Buffers / views outside the `Uint8Array`/`Buffer` bytes path.
            | "ArrayBuffer"
            | "ArrayBufferLike"
            | "ArrayBufferView"
            | "SharedArrayBuffer"
            | "DataView"
            | "BufferSource"
            // Non-`Uint8Array` typed arrays stay real QuickJS typed
            // arrays (`JsValue`): that keeps their per-element-width
            // coercion (Int8 wrap, Uint8Clamped clamp, Float32 rounding,
            // ...) exact, which a flat `Bytes`/`Array(F64)` model cannot
            // reproduce. `Uint8Array`/`Buffer` are the only spellings
            // routed to native `Bytes`.
            | "Int8Array"
            | "Uint8ClampedArray"
            | "Int16Array"
            | "Uint16Array"
            | "Int32Array"
            | "Uint32Array"
            | "Float16Array"
            | "Float32Array"
            | "Float64Array"
            | "BigInt64Array"
            | "BigUint64Array"
    )
}

/// Builds the HIR literal for a source string given its WTF-8 bytes: a
/// plain `HirLit::Str` when the text is valid UTF-8, or `HirLit::Wtf8`
/// when it contains a lone UTF-16 surrogate (which a Rust `String`
/// cannot hold). SWC's AST already carries the WTF-8 bytes, so this is
/// where a literal like `"\uD800"` stops being replaced with U+FFFD.
fn hir_string_literal_from_wtf8(bytes: &[u8]) -> HirLit {
    match std::str::from_utf8(bytes) {
        Ok(valid) => HirLit::Str(valid.to_string()),
        Err(_) => HirLit::Wtf8(bytes.to_vec()),
    }
}

/// An object-literal accessor's backing closure is stored as a hidden
/// `__thaw_getter_<name>`/`__thaw_setter_<name>` field so the read/write
/// paths can dispatch to it; it is never an own enumerable property, so
/// enumeration, JSON serialization and declared-type checks all skip it.
fn is_hidden_accessor_field(name: &str) -> bool {
    name.starts_with("__thaw_getter_") || name.starts_with("__thaw_setter_")
}

/// A class `#private` member is lowered to a field with this prefix; it is
/// storage only and is never an own property (keys, JSON, spread, clone).
const PRIVATE_SLOT_PREFIX: &str = "__thaw_private_";

fn ecmascript_field_order<T>(fields: &[(Symbol, T)]) -> Vec<usize> {
    let mut indices = fields
        .iter()
        .enumerate()
        .filter_map(|(index, (name, _))| {
            (!is_hidden_accessor_field(name)
                && !name.starts_with(PRIVATE_SLOT_PREFIX)
                && name != NATIVE_BASE_FIELD)
                .then_some((index, array_index_key(name)))
        })
        .collect::<Vec<_>>();
    indices.sort_by_key(|(index, array_index)| {
        (array_index.is_none(), array_index.unwrap_or(*index as u32))
    });
    indices.into_iter().map(|(index, _)| index).collect()
}

fn array_index_key(name: &str) -> Option<u32> {
    let index = name.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == name).then_some(index)
}

/// Whether an object type's identity chain (see `module/classes.rs`) makes
/// it an `Error`-family instance -- the built-ins themselves, or any user
/// class transitively `extends`ing one of them.
/// The shared physical prefix of every Error-family record: identity marker,
/// then `message` and `name`.
fn builtin_error_object_type() -> HirType {
    HirType::Object(vec![
        ("__thaw_class_identity_\u{1e}Error".to_string(), HirType::Bool),
        ("message".to_string(), HirType::Json),
        ("name".to_string(), HirType::Json),
    ])
}

fn object_type_is_error_family(ty: &HirType) -> bool {
    let HirType::Object(fields) = ty else {
        return false;
    };
    fields.first().is_some_and(|(marker, ty)| {
        *ty == HirType::Bool
            && marker
                .strip_prefix("__thaw_class_identity_\u{1e}")
                .is_some_and(|chain| chain.split('\u{1f}').any(is_error_family_name))
    })
}

fn literal_property_name(name: &PropName) -> Option<Symbol> {
    match name {
        PropName::Ident(name) => Some(name.sym.to_string()),
        PropName::Str(name) => Some(name.value.to_string_lossy().into_owned()),
        PropName::Num(name) => Some(number_property_name(name.value)),
        PropName::BigInt(name) => Some(name.value.to_string()),
        PropName::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Some(name.value.to_string_lossy().into_owned()),
            _ => None,
        },
    }
}

fn literal_expr_property_name(name: &Expr) -> Option<Symbol> {
    match name {
        Expr::Ident(name) => Some(name.sym.to_string()),
        Expr::Lit(Lit::Str(name)) => Some(name.value.to_string_lossy().into_owned()),
        Expr::Lit(Lit::Num(name)) => Some(number_property_name(name.value)),
        Expr::Lit(Lit::BigInt(name)) => Some(name.value.to_string()),
        _ => None,
    }
}

fn number_property_name(value: f64) -> Symbol {
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}

fn class_property_name(name: &PropName) -> Result<Symbol, String> {
    literal_property_name(name).ok_or_else(|| match name {
        PropName::Computed(_) => {
            "native class computed members require a string-literal name".into()
        }
        _ => "native class members require a static name".into(),
    })
}

/// Infers a class field's type from a self-contained literal initializer.
/// This runs before the class body is resolved, so identifiers, calls,
/// spreads, empty arrays, and mixed-element arrays still require an explicit
/// annotation rather than introducing a lowering-order dependency or guessing
/// a union layout.
fn infer_class_field_literal_type(expr: &Expr) -> Option<HirType> {
    match expr {
        Expr::Lit(Lit::Num(_)) => Some(HirType::F64),
        Expr::Lit(Lit::Str(_)) => Some(HirType::Str),
        Expr::Lit(Lit::Bool(_)) => Some(HirType::Bool),
        Expr::Paren(parenthesized) => infer_class_field_literal_type(&parenthesized.expr),
        Expr::Array(array) => {
            let elements = array
                .elems
                .iter()
                .map(|element| {
                    let element = element.as_ref()?;
                    element.spread.is_none().then_some(())?;
                    infer_class_field_literal_type(&element.expr)
                })
                .collect::<Option<Vec<_>>>()?;
            let first = elements.first()?.clone();
            elements
                .iter()
                .all(|element| element == &first)
                .then(|| HirType::Array(Box::new(first)))
        }
        Expr::Object(object) => {
            let mut fields = Vec::with_capacity(object.props.len());
            for property in &object.props {
                let PropOrSpread::Prop(property) = property else {
                    return None;
                };
                let Prop::KeyValue(property) = property.as_ref() else {
                    return None;
                };
                let name = class_property_name(&property.key).ok()?;
                if fields.iter().any(|(existing, _)| existing == &name) {
                    return None;
                }
                fields.push((
                    name,
                    infer_class_field_literal_type(&property.value)?,
                ));
            }
            Some(HirType::Object(fields))
        }
        _ => None,
    }
}

fn member_property_name(name: &MemberProp) -> Option<Symbol> {
    match name {
        MemberProp::Ident(name) => Some(name.sym.to_string()),
        MemberProp::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Some(name.value.to_string_lossy().into_owned()),
            _ => None,
        },
        _ => None,
    }
}

fn ordinary_optional_chain_expression(chain: &swc_ecma_ast::OptChainExpr) -> Expr {
    match chain.base.as_ref() {
        OptChainBase::Member(member) => Expr::Member(member.clone()),
        OptChainBase::Call(call) => Expr::Call(CallExpr::from(call.clone())),
    }
}

fn ordinary_optional_expression(expression: &Expr) -> Expr {
    match expression {
        Expr::OptChain(chain) => ordinary_optional_chain_expression(chain),
        expression => expression.clone(),
    }
}

fn class_constructor_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_constructor")
}

fn class_initializer_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_initialize")
}

fn pattern_is_omittable(pattern: &Pat) -> bool {
    matches!(pattern, Pat::Assign(_))
        || matches!(pattern, Pat::Ident(binding) if binding.id.optional)
}

fn source_omission_mask(patterns: &[Pat], receiver_count: usize) -> HirOptionalMask {
    HirOptionalMask::from_bools(
        &(0..receiver_count)
            .map(|_| false)
            .chain(patterns.iter().map(pattern_is_omittable))
            .collect::<Vec<_>>(),
    )
}

fn mask_with_offset(mask: &HirOptionalMask, offset: usize, len: usize) -> HirOptionalMask {
    HirOptionalMask::from_bools(
        &(0..len)
            .map(|index| index >= offset && mask.contains(index - offset))
            .collect::<Vec<_>>(),
    )
}

fn optional_parameter_type(ty: HirType) -> HirType {
    match ty {
        HirType::Optional(_) | HirType::Nullish(_) => ty,
        HirType::Nullable(payload) => HirType::Nullish(payload),
        other => HirType::Optional(Box::new(other)),
    }
}

fn optional_parameter_mask(optional: &[bool]) -> HirOptionalMask {
    HirOptionalMask::from_bools(optional)
}

fn is_optional_parameter(mask: &HirOptionalMask, index: usize) -> bool {
    mask.contains(index)
}

fn omitted_parameter_value(ty: &HirType) -> Result<HirExpr, String> {
    match ty {
        HirType::Json | HirType::Dynamic | HirType::JsValue => {
            Ok(HirExpr::Lit(HirLit::Undefined))
        }
        HirType::Optional(payload) => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
        HirType::Nullish(payload) => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
        HirType::Union(elements) => elements
            .iter()
            .position(|element| *element == HirType::Undefined)
            .map(|index| {
                HirExpr::UnionInject(
                    Box::new(HirExpr::Lit(HirLit::Undefined)),
                    index,
                    elements.clone(),
                )
            })
            .ok_or_else(|| {
                format!("omittable callable parameter needs an undefined-capable ABI type, got {ty:?}")
            }),
        other => Err(format!(
            "omittable callable parameter needs an undefined-capable ABI type, got {other:?}"
        )),
    }
}

#[cfg(test)]
mod source_omission_mask_tests {
    use super::{mask_with_offset, source_omission_mask};
    use crate::HirOptionalMask;
    use swc_ecma_ast::{Decl, ModuleItem, Stmt};

    #[test]
    fn source_masks_track_syntax_and_physical_receiver_offsets() {
        let module = thaw_parser::parse_typescript(
            "function f(this: number, required: number | undefined, optional?: number, defaulted: number = 1, ...rest: number[]): void {}",
        )
        .unwrap();
        let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = &module.body[0] else {
            panic!("expected function declaration")
        };
        let patterns = function
            .function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect::<Vec<_>>();
        let source = source_omission_mask(&patterns, 1);
        assert_eq!(source, HirOptionalMask::Inline(0b1100));

        let unbound = source.shifted(1);
        assert_eq!(unbound, HirOptionalMask::Inline(0b0110));

        let initializer = mask_with_offset(&source, 1, patterns.len() + 2);
        assert_eq!(initializer, HirOptionalMask::Inline(0b11000));
    }
}

fn native_rest_element(patterns: &[Pat], params: &[HirType]) -> Result<Option<HirType>, String> {
    if patterns
        .iter()
        .take(patterns.len().saturating_sub(1))
        .any(|pattern| matches!(pattern, Pat::Rest(_)))
    {
        return Err("native class rest parameter must be last".into());
    }
    if !patterns
        .last()
        .is_some_and(|pattern| matches!(pattern, Pat::Rest(_)))
    {
        return Ok(None);
    }
    match params.last() {
        Some(HirType::Array(element)) => Ok(Some(element.as_ref().clone())),
        // A fixed tuple rest parameter still occupies one native ABI slot.
        // `Dynamic` is only the packing marker; the call path validates the
        // completed tuple against this last signature parameter.
        Some(HirType::Tuple(_)) => Ok(Some(HirType::Dynamic)),
        Some(other) => Err(format!(
            "native class rest parameter needs an array annotation, got {other:?}"
        )),
        None => Err("native class rest parameter is missing its signature type".into()),
    }
}

fn native_rest_array(values: Vec<HirExpr>, element: &HirType) -> HirExpr {
    if values.is_empty() {
        HirExpr::ArrayAlloc(Box::new(HirExpr::Lit(HirLit::F64(0.0))), element.clone())
    } else {
        HirExpr::ArrayLit(values)
    }
}

fn class_method_symbol(class: &str, method: &str) -> Symbol {
    format!("__thaw_class_{class}_method_{method}")
}

fn unbound_class_method_symbol(method: &str) -> Symbol {
    format!("{method}__thaw_unbound")
}

fn class_static_method_symbol(class: &str, method: &str) -> Symbol {
    format!("__thaw_class_{class}_static_{method}")
}

fn class_static_field_symbol(class: &str, field: &str) -> Symbol {
    format!("__thaw_class_{class}_static_field_{field}")
}

fn is_class_static_field_symbol(symbol: &str) -> bool {
    symbol.starts_with("__thaw_class_") && symbol.contains("_static_field_")
}

fn class_getter_symbol(class: &str, property: &str, is_static: bool) -> Symbol {
    format!(
        "__thaw_class_{class}_{}_getter_{property}",
        if is_static { "static" } else { "instance" }
    )
}

fn class_setter_symbol(class: &str, property: &str, is_static: bool) -> Symbol {
    format!(
        "__thaw_class_{class}_{}_setter_{property}",
        if is_static { "static" } else { "instance" }
    )
}

fn class_member_symbol(class: &str, member: &swc_ecma_ast::ClassMethod) -> Result<Symbol, String> {
    let name = class_property_name(&member.key)?;
    Ok(match member.kind {
        MethodKind::Getter => class_getter_symbol(class, &name, member.is_static),
        MethodKind::Setter => class_setter_symbol(class, &name, member.is_static),
        MethodKind::Method if member.is_static => class_static_method_symbol(class, &name),
        MethodKind::Method => class_method_symbol(class, &name),
    })
}

fn super_property_name(property: &SuperProp) -> Result<Symbol, String> {
    match property {
        SuperProp::Ident(name) => Ok(name.sym.to_string()),
        SuperProp::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Ok(name.value.to_string_lossy().into_owned()),
            _ => Err("computed super properties require a string literal name".into()),
        },
    }
}

fn class_name_from_type(ty: &HirType) -> Option<&str> {
    let HirType::Object(fields) = ty else {
        return None;
    };
    fields.first().and_then(|(name, ty)| {
        (*ty == HirType::Bool)
            .then(|| name.strip_prefix("__thaw_class_identity_\u{1e}"))
            .flatten()
            .and_then(|identities| identities.split('\u{1f}').next())
    })
}

/// Name of the hidden field (always index 1, right after the identity marker)
/// holding the native collection of a class that `extends Map`/`Set`.
const NATIVE_BASE_FIELD: &str = "__thaw_base";

/// The native `Map`/`Set` type a class extending one of them wraps.
fn native_base_type(ty: &HirType) -> Option<&HirType> {
    let HirType::Object(fields) = ty else {
        return None;
    };
    fields
        .get(1)
        .filter(|(name, _)| name == NATIVE_BASE_FIELD)
        .map(|(_, ty)| ty)
}

/// `Map`/`Set`/`Array`: the builtin a class extending one of them inherits its
/// `Symbol.toStringTag` (`[object Map]`) from.
fn native_base_tag(ty: &HirType) -> Option<&'static str> {
    match native_base_type(ty)? {
        HirType::Map(..) => Some("Map"),
        HirType::Set(_) => Some("Set"),
        HirType::Array(_) => Some("Array"),
        _ => None,
    }
}

/// `receiver.__thaw_base`: the native collection behind a `Map`/`Set` subclass.
fn native_base_member(receiver: Expr) -> Expr {
    Expr::Member(MemberExpr {
        span: swc_common::DUMMY_SP,
        obj: Box::new(receiver),
        prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(
            NATIVE_BASE_FIELD.into(),
            swc_common::DUMMY_SP,
        )),
    })
}

/// Layout of `extends Map<K, V>` / `extends Set<T>`: the wrapped collection type.
fn native_collection_base_type(
    class: &str,
    base_name: &str,
    declaration: &ClassDecl,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Option<HirType>, String> {
    let arity = match base_name {
        "Map" => 2,
        "Set" | "Array" => 1,
        _ => return Ok(None),
    };
    let params = declaration
        .class
        .super_type_params
        .as_ref()
        .map(|params| params.params.as_slice())
        .unwrap_or_default();
    if params.len() != arity {
        return Err(format!(
            "class `{class}` needs explicit type arguments: `extends {base_name}<{}>`",
            if arity == 2 { "K, V" } else { "T" }
        ));
    }
    let mut lowered = Vec::with_capacity(arity);
    for param in params {
        lowered.push(Box::new(lower_ts_type(param, interfaces, generic_interfaces)?));
    }
    let mut lowered = lowered.into_iter();
    let first = lowered.next().expect("arity checked");
    let ty = match lowered.next() {
        Some(value) => HirType::Map(first, value),
        None if base_name == "Array" => HirType::Array(first),
        None => HirType::Set(first),
    };
    if let HirType::Map(key, _) | HirType::Set(key) = &ty {
        map_key_intrinsic_suffix(key)?;
    }
    Ok(Some(ty))
}

fn class_type_has_identity(ty: &HirType, expected: &str) -> bool {
    let HirType::Object(fields) = ty else {
        return false;
    };
    fields.first().is_some_and(|(name, ty)| {
        *ty == HirType::Bool
            && name
                .strip_prefix("__thaw_class_identity_\u{1e}")
                .is_some_and(|identities| identities.split('\u{1f}').any(|name| name == expected))
    })
}

fn class_constructor_param_pattern(parameter: &ParamOrTsParamProp) -> Pat {
    match parameter {
        ParamOrTsParamProp::Param(parameter) => parameter.pat.clone(),
        ParamOrTsParamProp::TsParamProp(property) => match &property.param {
            TsParamPropParam::Ident(binding) => Pat::Ident(binding.clone()),
            TsParamPropParam::Assign(assignment) => Pat::Assign(assignment.clone()),
        },
    }
}

fn parameter_property_binding(
    property: &swc_ecma_ast::TsParamProp,
) -> Result<&swc_ecma_ast::BindingIdent, String> {
    match &property.param {
        TsParamPropParam::Ident(binding) => Ok(binding),
        TsParamPropParam::Assign(assignment) => match assignment.left.as_ref() {
            Pat::Ident(binding) => Ok(binding),
            _ => Err("constructor parameter properties require identifier bindings".into()),
        },
    }
}
