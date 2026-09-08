fn regex_object_type() -> HirType {
    HirType::Object(vec![
        ("source".to_string(), HirType::Str),
        ("flags".to_string(), HirType::Str),
        ("lastIndex".to_string(), HirType::F64),
    ])
}

/// `Date` is a fixed native object with a single millisecond-since-epoch
/// `timestamp` field (reusing the existing object machinery, like
/// `regex_object_type`, rather than a new value representation).
/// `thaw-runtime`'s calendar math is UTC-only -- there is no host timezone
/// database, so "local" `Date` methods alias their UTC counterparts.
fn date_object_type() -> HirType {
    HirType::Object(vec![("timestamp".to_string(), HirType::F64)])
}

/// `Map`/`Set`'s key/element type selects which family of native
/// `__thaw_map_*` intrinsics to call -- `"num"` (`SameValueZero`-equal
/// numbers), `"str"` (content-hashed strings), or `"ref"` (hashed and
/// compared by its own pointer value -- reference identity, exactly like
/// JavaScript's `SameValueZero` degenerates to `===` for non-primitive
/// keys). `Optional`/`Nullable`/`Nullish`/`Union` don't fit `"ref"`:
/// they're inline tagged structs, not a single pointer, so there's no
/// stable identity word to hash. `Function`/`CallableFunction` don't
/// either, for a different reason: referencing the same top-level named
/// function as a value builds a fresh closure-ABI wrapper each time
/// (confirmed empirically -- `f === f` is observably `false` here), so
/// there is no stable identity to key by even though the value is
/// pointer-shaped. Any other key type is rejected at `Map<K, V>`/
/// `Set<T>` resolution time (`type_resolution.rs`) and at
/// `new Map<K, V>()`/`new Set<T>()` construction time, so this should
/// never actually fail once a `Map`/`Set` value exists, but a lowering
/// bug elsewhere producing one anyway shouldn't panic.
fn map_key_intrinsic_suffix(key_type: &HirType) -> Result<&'static str, String> {
    match key_type {
        HirType::F64 => Ok("num"),
        HirType::Str => Ok("str"),
        HirType::Array(_)
        | HirType::Tuple(_)
        | HirType::Object(_)
        | HirType::Json
        | HirType::Dictionary(_)
        | HirType::Promise(_)
        | HirType::Map(_, _)
        | HirType::Set(_) => Ok("ref"),
        other => Err(format!(
            "Map/Set keys must be `number`, `string`, or a reference type \
             (object, array, ...), got {other:?}"
        )),
    }
}

/// `WeakMap`/`WeakSet` reuse `map_key_intrinsic_suffix`'s own type
/// classification, but only the `"ref"` family is a valid key for them --
/// matching the specification, which requires a `WeakMap`/`WeakSet`
/// key/element to be an object (or similar reference type), never a
/// `number` or `string`.
fn weak_key_intrinsic_suffix(key_type: &HirType) -> Result<&'static str, String> {
    match map_key_intrinsic_suffix(key_type) {
        Ok("ref") => Ok("ref"),
        _ => Err(format!(
            "WeakMap/WeakSet keys must be a reference type (object, array, ...), got {key_type:?}"
        )),
    }
}

/// Chooses which `__thaw_map_{num,str}_get_*` variant decodes a `Map`
/// value of `value_type` correctly, and whether the raw call result needs
/// wrapping in `HirExpr::TypedClosure` to recover a pointer-shaped type
/// the intrinsic name alone can't carry (unlike `F64`/`Bool`, where the
/// result type is always the same regardless of context).
fn map_value_get_suffix(value_type: &HirType) -> Result<(&'static str, bool), String> {
    match value_type {
        HirType::F64 => Ok(("f64", false)),
        HirType::Bool | HirType::Undefined | HirType::Null => Ok(("bool", false)),
        HirType::I64 | HirType::JsValue => Err(format!(
            "Map/Set values of type {value_type:?} are not supported"
        )),
        _ => Ok(("ptr", true)),
    }
}

