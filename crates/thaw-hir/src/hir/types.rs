pub type Symbol = String;

#[derive(Clone, Debug, PartialEq)]
pub enum HirOptionalMask {
    Inline(u64),
    Extended(Box<Vec<u64>>),
}

impl Default for HirOptionalMask {
    fn default() -> Self {
        Self::Inline(0)
    }
}

impl HirOptionalMask {
    pub fn from_bools(optional: &[bool]) -> Self {
        if optional.len() <= u64::BITS as usize {
            let mask = optional
                .iter()
                .enumerate()
                .fold(0u64, |mask, (index, value)| {
                    mask | (u64::from(*value) << index)
                });
            return Self::Inline(mask);
        }
        let mut words = vec![0; optional.len().div_ceil(u64::BITS as usize)];
        for (index, value) in optional.iter().enumerate() {
            if *value {
                words[index / u64::BITS as usize] |= 1u64 << (index % u64::BITS as usize);
            }
        }
        while words.last() == Some(&0) {
            words.pop();
        }
        Self::Extended(Box::new(words))
    }

    pub fn insert(&mut self, index: usize) {
        match self {
            Self::Inline(mask) if index < u64::BITS as usize => *mask |= 1u64 << index,
            Self::Inline(mask) => {
                let mut words = vec![0; index / u64::BITS as usize + 1];
                words[0] = *mask;
                words[index / u64::BITS as usize] |= 1u64 << (index % u64::BITS as usize);
                *self = Self::Extended(Box::new(words));
            }
            Self::Extended(words) => {
                words.resize(words.len().max(index / u64::BITS as usize + 1), 0);
                words[index / u64::BITS as usize] |= 1u64 << (index % u64::BITS as usize);
            }
        }
    }

    pub fn offset_by(&self, offset: usize) -> Self {
        let mut shifted = Self::default();
        let mut next = self.first_at_or_after(0);
        while let Some(index) = next {
            shifted.insert(index + offset);
            next = self.first_at_or_after(index + 1);
        }
        shifted
    }

    pub fn contains(&self, index: usize) -> bool {
        match self {
            Self::Inline(mask) => index < u64::BITS as usize && mask & (1u64 << index) != 0,
            Self::Extended(words) => words
                .get(index / u64::BITS as usize)
                .is_some_and(|word| word & (1u64 << (index % u64::BITS as usize)) != 0),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Inline(mask) => *mask == 0,
            Self::Extended(words) => words.is_empty(),
        }
    }

    pub fn count(&self) -> u32 {
        match self {
            Self::Inline(mask) => mask.count_ones(),
            Self::Extended(words) => words.iter().map(|word| word.count_ones()).sum(),
        }
    }

    pub fn shifted(&self, offset: usize) -> Self {
        if let Self::Inline(mask) = self {
            return Self::Inline(
                u32::try_from(offset)
                    .ok()
                    .and_then(|offset| mask.checked_shr(offset))
                    .unwrap_or(0),
            );
        }
        let Self::Extended(words) = self else {
            unreachable!()
        };
        let bit_len = words.len() * u64::BITS as usize;
        let optional = (offset..bit_len)
            .map(|index| self.contains(index))
            .collect::<Vec<_>>();
        Self::from_bools(&optional)
    }

    pub fn first_at_or_after(&self, start: usize) -> Option<usize> {
        let bit_len = match self {
            Self::Inline(_) => u64::BITS as usize,
            Self::Extended(words) => words.len() * u64::BITS as usize,
        };
        (start..bit_len).find(|index| self.contains(*index))
    }
}

#[cfg(test)]
mod optional_mask_tests {
    use super::HirOptionalMask;

    #[test]
    fn omission_bits_keep_identity_across_word_boundaries() {
        let mut mask = HirOptionalMask::default();
        mask.insert(0);
        mask.insert(64);
        assert!(mask.contains(0));
        assert!(mask.contains(64));
        assert!(!mask.contains(63));
        let initializer = mask.offset_by(1);
        assert!(initializer.contains(1));
        assert!(initializer.contains(65));
        assert!(!initializer.contains(64));
    }

    #[test]
    fn optional_masks_scale_beyond_one_machine_word() {
        let mut optional = vec![false; 131];
        optional[1] = true;
        optional[64] = true;
        optional[130] = true;
        let mask = HirOptionalMask::from_bools(&optional);

        assert!(mask.contains(1));
        assert!(mask.contains(64));
        assert!(mask.contains(130));
        assert!(!mask.contains(63));
        assert_eq!(mask.count(), 3);
        assert_eq!(mask.first_at_or_after(2), Some(64));

        let shifted = mask.shifted(64);
        assert!(shifted.contains(0));
        assert!(shifted.contains(66));
        assert!(!shifted.contains(1));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirType {
    F64,
    I64,
    Bool,
    Undefined,
    Null,
    Void,
    Str,
    Symbol,
    /// Compile-time generic specialization marker; erased to `Str` in HIR bodies.
    StrLiteral(Symbol),
    Json,
    /// A runtime-keyed JSON object whose values share one native type.
    Dictionary(Box<HirType>),
    /// Opaque callable/object value retained by the embedded JavaScript realm.
    JsValue,
    Promise(Box<HirType>),
    Array(Box<HirType>),
    /// A byte buffer (`Buffer` / `Uint8Array`). Physically an
    /// `Array(F64)` -- same `[len][payload]` heap block, same element
    /// load/store, same `.length` -- and every codegen and inference
    /// site that handles `Array(F64)` handles this identically (grep
    /// `HirType::Bytes`). The separate identity is only so method
    /// dispatch can tell `buf.toString("utf8")` (decode) from an array's
    /// `.toString()` (comma-join). Elements are logically `u8`.
    Bytes,
    /// A native hash-table-backed `Map<K, V>`. `K` must be `F64` or `Str`
    /// (SameValueZero-equal numbers or content-equal strings) -- there is
    /// no reference-identity hashing for object/array keys, since nothing
    /// in the runtime gives heap values a stable identity token to hash.
    Map(Box<HirType>, Box<HirType>),
    /// A non-enumerable weak-keyed map. It shares the native table layout
    /// with `Map`, but remains distinct so WeakMap-only API restrictions are
    /// enforced during lowering.
    WeakMap(Box<HirType>, Box<HirType>),
    /// A native hash-table-backed `Set<T>`, sharing the same `K`
    /// restriction as `Map` (`F64` or `Str`).
    Set(Box<HirType>),
    /// A non-enumerable weak-keyed set; see `WeakMap`.
    WeakSet(Box<HirType>),
    Tuple(Vec<HirType>),
    Object(Vec<(Symbol, HirType)>),
    Function(Vec<HirType>, Box<HirType>),
    /// Native callable with a separate physical `this` parameter. Its value
    /// is still one closure pointer; `signature` describes visible values
    /// (Function or CallableFunction), never the receiver ABI slot.
    FunctionWithThis(Box<HirType>, Box<HirType>),
    // A callable whose fixed prefix can contain omittable slots and can be
    // followed by a packed native rest array. The bit mask is parallel to the
    // fixed parameters and records logical optional/default positions.
    CallableFunction(
        Vec<HirType>,
        HirOptionalMask,
        Option<Box<HirType>>,
        Box<HirType>,
    ),
    Union(Vec<HirType>),
    /// A native `T | undefined` value represented as an explicit presence
    /// tag plus a payload. This avoids sentinel collisions with NaN/pointers.
    Optional(Box<HirType>),
    /// A native `T | null` value with the same tagged physical layout as an
    /// optional, but a distinct absence kind for equality/typeof/display.
    Nullable(Box<HirType>),
    /// A native `T | null | undefined` value. Its tag distinguishes a
    /// payload, `null`, and `undefined` without relying on sentinels.
    Nullish(Box<HirType>),
    /// Type inference failed / not yet supported for this expression ->
    /// falls back to QuickJS-NG at runtime (see design doc section 3.1).
    Dynamic,
}

/// The ordinary value stored in a `catch` binding. Its member order is also
/// the compact exception tag understood by synchronous and split async
/// catches. Keep the original native owner/Json pointer or live JS handle;
/// display text is a separate channel and must never become identity.
pub fn caught_exception_carrier_type() -> HirType {
    HirType::Union(vec![
        HirType::F64,
        HirType::I64,
        HirType::Bool,
        HirType::Str,
        HirType::Undefined,
        HirType::Null,
        HirType::Object(Vec::new()),
        HirType::Json,
        HirType::JsValue,
    ])
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirLit {
    F64(f64),
    I64(i64),
    Str(String),
    /// A string literal that contains at least one lone UTF-16 surrogate,
    /// stored as WTF-8 bytes (UTF-8 extended to encode unpaired
    /// surrogates as their 3-byte sequence). A Rust `String` cannot hold
    /// a lone surrogate, so such literals need their own representation;
    /// every other string literal (and every identifier / property name /
    /// key) stays `Str`. At codegen time both become the same
    /// NUL-terminated byte buffer; the runtime string functions decode
    /// WTF-8 (of which UTF-8 is a subset), so a `Wtf8` value flows through
    /// `length`/`charCodeAt`/`slice`/... with its surrogates intact.
    Wtf8(Vec<u8>),
    Bool(bool),
    Undefined,
    ArrayHole,
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitOr,
    BitXor,
    BitAnd,
    LShift,
    RShift,
    ZeroFillRShift,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEqEq,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirParam {
    pub name: Symbol,
    pub ty: HirType,
}

/// Unambiguous private runtime descriptor for an ordered native object layout.
/// A complete expected descriptor is a byte prefix of a wider allocation's
/// descriptor only when every preceding field name and HIR type matches.
pub fn native_object_layout_token(fields: &[(Symbol, HirType)]) -> String {
    // The token passes through a static C string ABI. Hex encoding avoids a
    // user property name containing NUL truncating the trusted layout test.
    let mut token = String::new();
    for (name, ty) in fields {
        let type_name = format!("{ty:?}");
        for part in [name.as_bytes(), type_name.as_bytes()] {
            token.push_str(&format!("{}:", part.len()));
            for byte in part {
                token.push_str(&format!("{byte:02x}"));
            }
        }
    }
    token
}
