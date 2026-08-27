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
    Json,
    /// A runtime-keyed JSON object whose values share one native type.
    Dictionary(Box<HirType>),
    /// Opaque callable/object value retained by the embedded JavaScript realm.
    JsValue,
    Promise(Box<HirType>),
    Array(Box<HirType>),
    Tuple(Vec<HirType>),
    Object(Vec<(Symbol, HirType)>),
    Function(Vec<HirType>, Box<HirType>),
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

#[derive(Debug, Clone, PartialEq)]
pub enum HirLit {
    F64(f64),
    Str(String),
    Bool(bool),
    Undefined,
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
