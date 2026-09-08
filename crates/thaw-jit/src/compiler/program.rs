#[repr(C)]
pub struct ThawJitResult {
    pub value: f64,
    pub error: *const c_char,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NumericOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum CompareOp {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

impl CompareOp {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "lt" => Some(Self::Less),
            "lte" => Some(Self::LessEqual),
            "gt" => Some(Self::Greater),
            "gte" => Some(Self::GreaterEqual),
            "eq" => Some(Self::Equal),
            "ne" => Some(Self::NotEqual),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum UnaryMath {
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Cbrt,
    Ceil,
    Clz32,
    Cos,
    Cosh,
    Exp,
    Expm1,
    Floor,
    Fround,
    Log,
    Log1p,
    Log2,
    Log10,
    Round,
    Sign,
    Sin,
    Sinh,
    SquareRoot,
    Tan,
    Tanh,
    Truncate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BitwiseOp {
    And,
    Or,
    ShiftLeft,
    ShiftRight,
    ShiftRightUnsigned,
    Xor,
}

impl BitwiseOp {
    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn function(self) -> extern "C" fn(f64, f64) -> f64 {
        match self {
            Self::And => bit_and,
            Self::Or => bit_or,
            Self::ShiftLeft => shift_left,
            Self::ShiftRight => shift_right,
            Self::ShiftRightUnsigned => shift_right_unsigned,
            Self::Xor => bit_xor,
        }
    }
}

impl UnaryMath {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "acos" => Some(Self::Acos),
            "acosh" => Some(Self::Acosh),
            "asin" => Some(Self::Asin),
            "asinh" => Some(Self::Asinh),
            "atan" => Some(Self::Atan),
            "atanh" => Some(Self::Atanh),
            "cbrt" => Some(Self::Cbrt),
            "ceil" => Some(Self::Ceil),
            "clz32" => Some(Self::Clz32),
            "cos" => Some(Self::Cos),
            "cosh" => Some(Self::Cosh),
            "exp" => Some(Self::Exp),
            "expm1" => Some(Self::Expm1),
            "floor" => Some(Self::Floor),
            "fround" => Some(Self::Fround),
            "log" => Some(Self::Log),
            "log1p" => Some(Self::Log1p),
            "log2" => Some(Self::Log2),
            "log10" => Some(Self::Log10),
            "round" => Some(Self::Round),
            "sign" => Some(Self::Sign),
            "sin" => Some(Self::Sin),
            "sinh" => Some(Self::Sinh),
            "sqrt" => Some(Self::SquareRoot),
            "tan" => Some(Self::Tan),
            "tanh" => Some(Self::Tanh),
            "trunc" => Some(Self::Truncate),
            _ => None,
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn function(self) -> extern "C" fn(f64) -> f64 {
        match self {
            Self::Acos => acos_number,
            Self::Acosh => acosh_number,
            Self::Asin => asin_number,
            Self::Asinh => asinh_number,
            Self::Atan => atan_number,
            Self::Atanh => atanh_number,
            Self::Cbrt => cbrt_number,
            Self::Ceil => ceil_number,
            Self::Clz32 => clz32_number,
            Self::Cos => cos_number,
            Self::Cosh => cosh_number,
            Self::Exp => exp_number,
            Self::Expm1 => expm1_number,
            Self::Floor => floor_number,
            Self::Fround => fround_number,
            Self::Log => log_number,
            Self::Log1p => log1p_number,
            Self::Log2 => log2_number,
            Self::Log10 => log10_number,
            Self::Round => round_number,
            Self::Sign => sign_number,
            Self::Sin => sin_number,
            Self::Sinh => sinh_number,
            Self::SquareRoot => unreachable!("square root emits SSE2 directly"),
            Self::Tan => tan_number,
            Self::Tanh => tanh_number,
            Self::Truncate => truncate_number,
        }
    }
}

impl NumericOp {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Subtract),
            "mul" => Some(Self::Multiply),
            "div" => Some(Self::Divide),
            _ => None,
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn opcode(self) -> u8 {
        match self {
            Self::Add => 0x58,
            Self::Subtract => 0x5c,
            Self::Multiply => 0x59,
            Self::Divide => 0x5e,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NumericReduceOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,
    Minimum,
    Maximum,
}

impl NumericReduceOp {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Subtract),
            "mul" => Some(Self::Multiply),
            "div" => Some(Self::Divide),
            "rem" => Some(Self::Remainder),
            "pow" => Some(Self::Power),
            "min" => Some(Self::Minimum),
            "max" => Some(Self::Maximum),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NumericValue {
    Argument(u8),
    DynamicArgument(u8),
    Constant(f64),
    Operation(NumericOp),
    Compare(CompareOp),
    Bitwise(BitwiseOp),
    BitNot,
    Absolute,
    Negate,
    Maximum,
    Minimum,
    Atan2,
    Hypot,
    Imul,
    IsFinite,
    IsInteger,
    IsNaN,
    IsSafeInteger,
    NumberSameValue,
    StringSameValue,
    ReferenceSameValue,
    TypeOfNumber,
    TypeOfBoolean,
    TypeOfString,
    TypeOfObject,
    TypeOfDynamic,
    DynamicToBoolean,
    StringCompare,
    StringCharAt,
    StringCharCodeAt,
    StringAt,
    StringCodePointAt,
    StringConcat,
    NumberToString,
    BooleanToString,
    StringToNumber,
    DynamicToString,
    DynamicToNumber,
    DynamicAdd,
    DynamicCompare(u8),
    TagNumber,
    TagString,
    TagBoolean,
    TagAggregate(u8),
    DynamicTag,
    UntagNumber,
    UntagString,
    UntagBoolean,
    UntagNumberArray,
    UntagBooleanArray,
    UntagStringArray,
    UntagArray,
    UntagNumberDictionary,
    UntagBooleanDictionary,
    UntagStringDictionary,
    UntagDictionary,
    UntagObject,
    UntagTuple,
    ObjectField(u8, u16),
    OptionalObjectField(u8, u16),
    OptionalTupleField(u8, u16),
    NullishObjectField(u8, u16),
    NullishTupleField(u8, u16),
    FixedObjectNew(u16),
    FixedObjectSet(u8, u16),
    TaggedObjectNumberUpdate(u8, u16),
    TaggedObjectNumberAssign(u8, u16),
    FixedTupleNew(u16),
    FixedTupleSet(u8, u16),
    FixedWideTupleNew(u16),
    FixedWideTupleSet(u8, u16, u8),
    ExcludeNumber,
    ExcludeString,
    ExcludeBoolean,
    ExcludeArray,
    ExcludeObject,
    GlobalGet,
    GlobalInit,
    GlobalSet,
    CallableEntryGet,
    CallableEntrySet,
    ParseFloat,
    ParseInt,
    NumberToFixed,
    NumberToPrecision,
    NumberToRadixString,
    NumberToExponential,
    NumberToExponentialShortest,
    StringConstant(*const c_char),
    StringEndsWith,
    StringEndsWithAt,
    StringIncludes,
    StringIncludesAt,
    StringIsWellFormed,
    StringIndexOf,
    StringIndexOfAt,
    StringLastIndexOf,
    StringLastIndexOfAt,
    StringLength,
    ArrayLength,
    DynamicArrayLength,
    IsArray,
    IsNotArray,
    DynamicIsArray,
    NumberArrayAt,
    BoolArrayAt,
    StringArrayAt,
    DynamicArrayAt,
    NumberArrayGet,
    BoolArrayGet,
    StringArrayGet,
    NumberDictionaryGet,
    BoolDictionaryGet,
    StringDictionaryGet,
    NumberDictionarySet,
    NumberDictionaryPostSet,
    StringDictionarySet,
    BoolDictionarySet,
    DictionaryDelete,
    DictionaryHasOwn,
    DictionaryIn,
    DictionaryKeys,
    NumberDictionaryValues,
    BoolDictionaryValues,
    StringDictionaryValues,
    NumberDictionaryEntries,
    BoolDictionaryEntries,
    StringDictionaryEntries,
    DictionaryFromNumberEntries,
    DictionaryFromBoolEntries,
    DictionaryFromStringEntries,
    DictionaryAssign,
    DictionaryLength,
    DictionaryKeyAt,
    EmptyDictionary,
    DictionaryAppend(u8),
    DictionaryStaticAppend(u8, *const c_char),
    NumberArrayIncludes,
    BoolArrayIncludes,
    StringArrayIncludes,
    DynamicArrayIncludes,
    NumberArrayIndexOf,
    BoolArrayIndexOf,
    StringArrayIndexOf,
    DynamicArrayIndexOf,
    NumberArrayLastIndexOf,
    BoolArrayLastIndexOf,
    StringArrayLastIndexOf,
    DynamicArrayLastIndexOf,
    NumberArrayJoin,
    BoolArrayJoin,
    StringArrayJoin,
    DynamicArrayJoin,
    DynamicArraySlice,
    ArraySlice,
    DynamicArrayConcat,
    ArrayConcat,
    NumberArrayAppend,
    StringArrayAppend,
    BoolArrayAppend,
    DynamicArrayAppend,
    ArrayToReversed,
    ArrayReverse,
    DynamicArrayToReversed,
    DynamicArrayReverse,
    NumberArrayToSorted,
    StringArrayToSorted,
    BoolArrayToSorted,
    DynamicArrayToSorted,
    NumberArraySort,
    NumberArrayToSortedBy(bool),
    NumberArraySortBy(bool),
    StringArrayToSortedDescending,
    StringArraySortDescending,
    StringArraySort,
    BoolArraySort,
    DynamicArraySort,
    NumberArrayFill,
    StringArrayFill,
    BoolArrayFill,
    DynamicArrayFill,
    ArrayCopyWithin,
    DynamicArrayCopyWithin,
    DynamicArrayWith,
    NumberArrayPush,
    StringArrayPush,
    BoolArrayPush,
    DynamicArrayPush,
    NumberArrayUnshift,
    StringArrayUnshift,
    BoolArrayUnshift,
    DynamicArrayUnshift,
    NumberArraySet,
    NumberArrayPostSet,
    StringArraySet,
    BoolArraySet,
    DynamicArraySet,
    AggregateLocalSet(u8, u8),
    AggregateLocalArrayInsert(u8, u8, bool),
    ArrayValue,
    MutableArrayHandle,
    EmptyArray,
    NumberArrayMin,
    NumberArrayMax,
    NumberArrayHypot,
    NumberArrayReduce(NumericReduceOp, bool, bool),
    NumberArrayJitReduce(bool, bool, bool),
    NumberArrayQuantifier(CompareOp, bool),
    NumberArrayFind(CompareOp, u8),
    NumberArrayFilter(CompareOp),
    PrimitiveArrayTruthy(u8, u8),
    DynamicArrayTruthy(u8),
    PrimitiveArrayCompare(u8, CompareOp, u8),
    DynamicArrayCompare(u8, u8),
    PrimitiveArrayMap(u8, u8),
    PrimitiveArrayConvert(u8, u8),
    DynamicArrayConvert(u8),
    DynamicArrayMapIdentity,
    DynamicArrayJitMap(u8, bool),
    DynamicArrayJitScan(u8, bool),
    DynamicArrayJitReduce(bool, bool, bool),
    NumberArrayMap(NumericReduceOp, bool),
    PrimitiveArrayJitMap(u8, u8, bool),
    PrimitiveArrayJitScan(u8, u8, bool),
    NumberArrayIndexMap(NumericReduceOp, bool),
    NumberArraySelectMap(CompareOp, u8),
    NumberArrayBranchMap(u16),
    NumberArrayUnaryMap(bool),
    NumberArrayMathMap(UnaryMath),
    NumberArrayPop,
    StringArrayPop,
    BoolArrayPop,
    DynamicArrayPop,
    NumberArrayShift,
    StringArrayShift,
    BoolArrayShift,
    DynamicArrayShift,
    ArraySplice,
    ArrayToSpliced,
    DynamicArraySplice,
    DynamicArrayToSpliced,
    NumberArrayWith,
    StringArrayWith,
    BoolArrayWith,
    StringTruthy,
    StringPadEnd,
    StringPadStart,
    StringStartsWith,
    StringStartsWithAt,
    StringRepeat,
    StringNormalize,
    StringSplit,
    StringToArray,
    StringFromCharCode,
    StringFromCodePoint,
    StringReplace,
    StringReplaceAll,
    StringSlice,
    StringSliceRange,
    StringSubstring,
    StringSubstringRange,
    StringToLowerCase,
    StringToWellFormed,
    StringToUpperCase,
    StringTrim,
    StringTrimEnd,
    StringTrimStart,
    Power,
    UnaryMath(UnaryMath),
    Remainder,
    Select,
    ShortCircuit(bool),
    ConditionalStart,
    PresentConditionalStart,
    ConditionalAlternate,
    ShortCircuitEnd,
    Absent,
    Null,
    PreserveAbsent,
    AsBoolean,
    BooleanNot,
    StrictMismatch(bool),
    Drop,
    DropUnder,
    Duplicate,
    DuplicatePair,
    LocalGet(u8),
    LocalSet(u8),
    LoopStart,
    LoopWhile,
    LoopContinuePoint,
    LoopBreak(u8),
    LoopContinue(u8),
    LoopEnd,
    GuardStart,
    GuardAlternate,
    GuardEnd,
    SwitchStart,
    SwitchCaseStart,
    SwitchCaseBody,
    SwitchDefault,
    SwitchBreak,
    SwitchEnd,
    TryStart,
    TaggedTryStart,
    CatchStart,
    TryEnd,
    ResultStart,
    ResultReturn(u8),
    ResultEnd,
    Throw,
    TaggedThrow(u8),
    CheckError,
    UncaughtNumberThrow,
    UncaughtBooleanThrow,
    UncaughtStringThrow,
    UncaughtNumberArrayThrow,
    UncaughtBooleanArrayThrow,
    UncaughtStringArrayThrow,
    UncaughtDictionaryThrow,
    EarlyReturn,
    MathRandom,
    DateNow,
    PerformanceNow,
    ProcessPid,
    ProcessPpid,
    MissingCallable,
    Recur(u8),
}

struct NumericProgram(Vec<NumericValue>);

impl NumericProgram {
    fn returns_tagged_array(&self) -> bool {
        matches!(
            self.0.last(),
            Some(
                NumericValue::StringSplit
                    | NumericValue::StringToArray
                    | NumericValue::ArraySlice
                    | NumericValue::ArrayConcat
                    | NumericValue::NumberArrayAppend
                    | NumericValue::StringArrayAppend
                    | NumericValue::BoolArrayAppend
                    | NumericValue::ArrayToReversed
                    | NumericValue::ArrayReverse
                    | NumericValue::NumberArrayToSorted
                    | NumericValue::StringArrayToSorted
                    | NumericValue::BoolArrayToSorted
                    | NumericValue::NumberArraySort
                    | NumericValue::StringArraySort
                    | NumericValue::BoolArraySort
                    | NumericValue::PrimitiveArrayJitMap(_, _, _)
                    | NumericValue::PrimitiveArrayJitScan(_, 6, _)
                    | NumericValue::StringArrayToSortedDescending
                    | NumericValue::StringArraySortDescending
                    | NumericValue::NumberArrayFill
                    | NumericValue::StringArrayFill
                    | NumericValue::BoolArrayFill
                    | NumericValue::ArrayCopyWithin
                    | NumericValue::ArraySplice
                    | NumericValue::ArrayToSpliced
                    | NumericValue::ArrayValue
                    | NumericValue::NumberArrayWith
                    | NumericValue::StringArrayWith
                    | NumericValue::BoolArrayWith
            )
        )
    }

    fn parse(symbol: &str) -> Option<Self> {
        if let Some(encoded) = symbol.strip_prefix("expr:") {
            let encoded = encoded.split_once(':')?.0;
            let values = encoded
                .split(',')
                .map(|token| match token {
                    "x" => Some(NumericValue::Argument(0)),
                    "y" => Some(NumericValue::Argument(1)),
                    "+" => Some(NumericValue::Operation(NumericOp::Add)),
                    "-" => Some(NumericValue::Operation(NumericOp::Subtract)),
                    "*" => Some(NumericValue::Operation(NumericOp::Multiply)),
                    "/" => Some(NumericValue::Operation(NumericOp::Divide)),
                    "<" => Some(NumericValue::Compare(CompareOp::Less)),
                    "<=" => Some(NumericValue::Compare(CompareOp::LessEqual)),
                    ">" => Some(NumericValue::Compare(CompareOp::Greater)),
                    ">=" => Some(NumericValue::Compare(CompareOp::GreaterEqual)),
                    "==" => Some(NumericValue::Compare(CompareOp::Equal)),
                    "!=" => Some(NumericValue::Compare(CompareOp::NotEqual)),
                    "band" => Some(NumericValue::Bitwise(BitwiseOp::And)),
                    "bor" => Some(NumericValue::Bitwise(BitwiseOp::Or)),
                    "bxor" => Some(NumericValue::Bitwise(BitwiseOp::Xor)),
                    "shl" => Some(NumericValue::Bitwise(BitwiseOp::ShiftLeft)),
                    "shr" => Some(NumericValue::Bitwise(BitwiseOp::ShiftRight)),
                    "ushr" => Some(NumericValue::Bitwise(BitwiseOp::ShiftRightUnsigned)),
                    "bnot" => Some(NumericValue::BitNot),
                    "abs" => Some(NumericValue::Absolute),
                    "neg" => Some(NumericValue::Negate),
                    "max" => Some(NumericValue::Maximum),
                    "min" => Some(NumericValue::Minimum),
                    "atan2" => Some(NumericValue::Atan2),
                    "hypot" => Some(NumericValue::Hypot),
                    "imul" => Some(NumericValue::Imul),
                    "isfinite" => Some(NumericValue::IsFinite),
                    "isinteger" => Some(NumericValue::IsInteger),
                    "isnan" => Some(NumericValue::IsNaN),
                    "issafeinteger" => Some(NumericValue::IsSafeInteger),
                    "numsame" => Some(NumericValue::NumberSameValue),
                    "strsame" => Some(NumericValue::StringSameValue),
                    "refsame" => Some(NumericValue::ReferenceSameValue),
                    "typeofnumber" => Some(NumericValue::TypeOfNumber),
                    "typeofboolean" => Some(NumericValue::TypeOfBoolean),
                    "typeofstring" => Some(NumericValue::TypeOfString),
                    "typeofobject" => Some(NumericValue::TypeOfObject),
                    "typeofdynamic" => Some(NumericValue::TypeOfDynamic),
                    "dynbool" => Some(NumericValue::DynamicToBoolean),
                    "strcmp" => Some(NumericValue::StringCompare),
                    "charat" => Some(NumericValue::StringCharAt),
                    "charcodeat" => Some(NumericValue::StringCharCodeAt),
                    "at" => Some(NumericValue::StringAt),
                    "codepointat" => Some(NumericValue::StringCodePointAt),
                    "concat" => Some(NumericValue::StringConcat),
                    "numstr" => Some(NumericValue::NumberToString),
                    "boolstr" => Some(NumericValue::BooleanToString),
                    "strnum" => Some(NumericValue::StringToNumber),
                    "dynstr" => Some(NumericValue::DynamicToString),
                    "dynnum" => Some(NumericValue::DynamicToNumber),
                    "dynadd" => Some(NumericValue::DynamicAdd),
                    "dynlt" => Some(NumericValue::DynamicCompare(0)),
                    "dynlte" => Some(NumericValue::DynamicCompare(1)),
                    "dyngt" => Some(NumericValue::DynamicCompare(2)),
                    "dyngte" => Some(NumericValue::DynamicCompare(3)),
                    "dyneq" => Some(NumericValue::DynamicCompare(4)),
                    "dynne" => Some(NumericValue::DynamicCompare(5)),
                    "dynseq" => Some(NumericValue::DynamicCompare(6)),
                    "dynsne" => Some(NumericValue::DynamicCompare(7)),
                    "tagnum" => Some(NumericValue::TagNumber),
                    "tagstr" => Some(NumericValue::TagString),
                    "tagbool" => Some(NumericValue::TagBoolean),
                    "tagrn" => Some(NumericValue::TagAggregate(0)),
                    "tagrb" => Some(NumericValue::TagAggregate(1)),
                    "tagrs" => Some(NumericValue::TagAggregate(2)),
                    "tagdn" => Some(NumericValue::TagAggregate(3)),
                    "tagdb" => Some(NumericValue::TagAggregate(4)),
                    "tagds" => Some(NumericValue::TagAggregate(5)),
                    "tagobject" => Some(NumericValue::TagAggregate(6)),
                    "tagtuple" => Some(NumericValue::TagAggregate(7)),
                    "tagkind" => Some(NumericValue::DynamicTag),
                    "untagnum" => Some(NumericValue::UntagNumber),
                    "untagstr" => Some(NumericValue::UntagString),
                    "untagbool" => Some(NumericValue::UntagBoolean),
                    "untagrn" => Some(NumericValue::UntagNumberArray),
                    "untagrb" => Some(NumericValue::UntagBooleanArray),
                    "untagrs" => Some(NumericValue::UntagStringArray),
                    "untagarray" | "untagarrayn" | "untagarrayb" | "untagarrays" => {
                        Some(NumericValue::UntagArray)
                    }
                    "untagdn" => Some(NumericValue::UntagNumberDictionary),
                    "untagdb" => Some(NumericValue::UntagBooleanDictionary),
                    "untagds" => Some(NumericValue::UntagStringDictionary),
                    "untagdictionary" => Some(NumericValue::UntagDictionary),
                    "untagobject" => Some(NumericValue::UntagObject),
                    "untagtuple" => Some(NumericValue::UntagTuple),
                    "notnum" => Some(NumericValue::ExcludeNumber),
                    "notstr" => Some(NumericValue::ExcludeString),
                    "notbool" => Some(NumericValue::ExcludeBoolean),
                    "notarray" => Some(NumericValue::ExcludeArray),
                    "notobject" => Some(NumericValue::ExcludeObject),
                    "globalget" => Some(NumericValue::GlobalGet),
                    "globalinit" => Some(NumericValue::GlobalInit),
                    "globalset" => Some(NumericValue::GlobalSet),
                    "callableget" => Some(NumericValue::CallableEntryGet),
                    "callableset" => Some(NumericValue::CallableEntrySet),
                    "parsefloat" => Some(NumericValue::ParseFloat),
                    "parseint" => Some(NumericValue::ParseInt),
                    "tofixed" => Some(NumericValue::NumberToFixed),
                    "toprecision" => Some(NumericValue::NumberToPrecision),
                    "toradix" => Some(NumericValue::NumberToRadixString),
                    "toexponential" => Some(NumericValue::NumberToExponential),
                    "toexponential0" => Some(NumericValue::NumberToExponentialShortest),
                    "endswith" => Some(NumericValue::StringEndsWith),
                    "endswith2" => Some(NumericValue::StringEndsWithAt),
                    "includes" => Some(NumericValue::StringIncludes),
                    "includes2" => Some(NumericValue::StringIncludesAt),
                    "iswellformed" => Some(NumericValue::StringIsWellFormed),
                    "indexof" => Some(NumericValue::StringIndexOf),
                    "indexof2" => Some(NumericValue::StringIndexOfAt),
                    "lastindexof" => Some(NumericValue::StringLastIndexOf),
                    "lastindexof2" => Some(NumericValue::StringLastIndexOfAt),
                    "strlen" => Some(NumericValue::StringLength),
                    "arraylen" => Some(NumericValue::ArrayLength),
                    "dynarraylen" => Some(NumericValue::DynamicArrayLength),
                    "isarray" => Some(NumericValue::IsArray),
                    "isnotarray" => Some(NumericValue::IsNotArray),
                    "dynisarray" => Some(NumericValue::DynamicIsArray),
                    "rnat" => Some(NumericValue::NumberArrayAt),
                    "rbat" => Some(NumericValue::BoolArrayAt),
                    "rsat" => Some(NumericValue::StringArrayAt),
                    "dynarrayat" => Some(NumericValue::DynamicArrayAt),
                    "rnget" => Some(NumericValue::NumberArrayGet),
                    "rbget" => Some(NumericValue::BoolArrayGet),
                    "rsget" => Some(NumericValue::StringArrayGet),
                    "raget" => Some(NumericValue::StringArrayGet),
                    "roget" => Some(NumericValue::StringArrayGet),
                    "ragetrn" | "ragetrb" | "ragetrs" | "rogetdn" | "rogetdb" | "rogetds" => {
                        Some(NumericValue::StringArrayGet)
                    }
                    "dnget" => Some(NumericValue::NumberDictionaryGet),
                    "dbget" => Some(NumericValue::BoolDictionaryGet),
                    "dsget" => Some(NumericValue::StringDictionaryGet),
                    "dnset" => Some(NumericValue::NumberDictionarySet),
                    "dnpostset" => Some(NumericValue::NumberDictionaryPostSet),
                    "dsset" => Some(NumericValue::StringDictionarySet),
                    "dbset" => Some(NumericValue::BoolDictionarySet),
                    "ddelete" => Some(NumericValue::DictionaryDelete),
                    "dhasown" => Some(NumericValue::DictionaryHasOwn),
                    "din" => Some(NumericValue::DictionaryIn),
                    "dkeys" => Some(NumericValue::DictionaryKeys),
                    "dnvalues" => Some(NumericValue::NumberDictionaryValues),
                    "dbvalues" => Some(NumericValue::BoolDictionaryValues),
                    "dsvalues" => Some(NumericValue::StringDictionaryValues),
                    "dnentries" => Some(NumericValue::NumberDictionaryEntries),
                    "dbentries" => Some(NumericValue::BoolDictionaryEntries),
                    "dsentries" => Some(NumericValue::StringDictionaryEntries),
                    "dnfromentries" => Some(NumericValue::DictionaryFromNumberEntries),
                    "dbfromentries" => Some(NumericValue::DictionaryFromBoolEntries),
                    "dsfromentries" => Some(NumericValue::DictionaryFromStringEntries),
                    "dassign" => Some(NumericValue::DictionaryAssign),
                    "dlen" => Some(NumericValue::DictionaryLength),
                    "dkeyat" => Some(NumericValue::DictionaryKeyAt),
                    "dnempty" | "dbempty" | "dsempty" => Some(NumericValue::EmptyDictionary),
                    "dnappend" => Some(NumericValue::DictionaryAppend(0)),
                    "dbappend" => Some(NumericValue::DictionaryAppend(1)),
                    "dsappend" => Some(NumericValue::DictionaryAppend(2)),
                    "rnincludes" => Some(NumericValue::NumberArrayIncludes),
                    "rbincludes" => Some(NumericValue::BoolArrayIncludes),
                    "rsincludes" => Some(NumericValue::StringArrayIncludes),
                    "dynarrayincludes" => Some(NumericValue::DynamicArrayIncludes),
                    "rnindexof" => Some(NumericValue::NumberArrayIndexOf),
                    "rbindexof" => Some(NumericValue::BoolArrayIndexOf),
                    "rsindexof" => Some(NumericValue::StringArrayIndexOf),
                    "dynarrayindexof" => Some(NumericValue::DynamicArrayIndexOf),
                    "rnlastindexof" => Some(NumericValue::NumberArrayLastIndexOf),
                    "rblastindexof" => Some(NumericValue::BoolArrayLastIndexOf),
                    "rslastindexof" => Some(NumericValue::StringArrayLastIndexOf),
                    "dynarraylastindexof" => Some(NumericValue::DynamicArrayLastIndexOf),
                    "rnjoin" => Some(NumericValue::NumberArrayJoin),
                    "rbjoin" => Some(NumericValue::BoolArrayJoin),
                    "rsjoin" => Some(NumericValue::StringArrayJoin),
                    "dynarrayjoin" => Some(NumericValue::DynamicArrayJoin),
                    "dynarrayslice" => Some(NumericValue::DynamicArraySlice),
                    "arrayslice" => Some(NumericValue::ArraySlice),
                    "dynarrayconcat" => Some(NumericValue::DynamicArrayConcat),
                    "arrayconcat" => Some(NumericValue::ArrayConcat),
                    "rnappend" => Some(NumericValue::NumberArrayAppend),
                    "rsappend" => Some(NumericValue::StringArrayAppend),
                    "rbappend" => Some(NumericValue::BoolArrayAppend),
                    "dynarrayappend" => Some(NumericValue::DynamicArrayAppend),
                    "captureappend" => Some(NumericValue::NumberArrayAppend),
                    "arrayreversed" => Some(NumericValue::ArrayToReversed),
                    "arrayreverse" => Some(NumericValue::ArrayReverse),
                    "dynarrayreversed" => Some(NumericValue::DynamicArrayToReversed),
                    "dynarrayreverse" => Some(NumericValue::DynamicArrayReverse),
                    "rnsorted" => Some(NumericValue::NumberArrayToSorted),
                    "rssorted" => Some(NumericValue::StringArrayToSorted),
                    "rbsorted" => Some(NumericValue::BoolArrayToSorted),
                    "dynarraysorted" => Some(NumericValue::DynamicArrayToSorted),
                    "rnsort" => Some(NumericValue::NumberArraySort),
                    "rnsortedasc" => Some(NumericValue::NumberArrayToSortedBy(false)),
                    "rnsorteddesc" => Some(NumericValue::NumberArrayToSortedBy(true)),
                    "rnsortasc" => Some(NumericValue::NumberArraySortBy(false)),
                    "rnsortdesc" => Some(NumericValue::NumberArraySortBy(true)),
                    "rssorteddesc" => Some(NumericValue::StringArrayToSortedDescending),
                    "rssortdesc" => Some(NumericValue::StringArraySortDescending),
                    "rssort" => Some(NumericValue::StringArraySort),
                    "rbsort" => Some(NumericValue::BoolArraySort),
                    "dynarraysort" => Some(NumericValue::DynamicArraySort),
                    "rnfill" => Some(NumericValue::NumberArrayFill),
                    "rsfill" => Some(NumericValue::StringArrayFill),
                    "rbfill" => Some(NumericValue::BoolArrayFill),
                    "dynarrayfill" => Some(NumericValue::DynamicArrayFill),
                    "arraycopywithin" => Some(NumericValue::ArrayCopyWithin),
                    "dynarraycopywithin" => Some(NumericValue::DynamicArrayCopyWithin),
                    "dynarraywith" => Some(NumericValue::DynamicArrayWith),
                    "arraysplice" => Some(NumericValue::ArraySplice),
                    "arraytospliced" => Some(NumericValue::ArrayToSpliced),
                    "dynarraysplice" => Some(NumericValue::DynamicArraySplice),
                    "dynarraytospliced" => Some(NumericValue::DynamicArrayToSpliced),
                    "rnpush" => Some(NumericValue::NumberArrayPush),
                    "rspush" => Some(NumericValue::StringArrayPush),
                    "rbpush" => Some(NumericValue::BoolArrayPush),
                    "dynarraypush" => Some(NumericValue::DynamicArrayPush),
                    "rnunshift" => Some(NumericValue::NumberArrayUnshift),
                    "rsunshift" => Some(NumericValue::StringArrayUnshift),
                    "rbunshift" => Some(NumericValue::BoolArrayUnshift),
                    "dynarrayunshift" => Some(NumericValue::DynamicArrayUnshift),
                    "rnset" => Some(NumericValue::NumberArraySet),
                    "rnpostset" => Some(NumericValue::NumberArrayPostSet),
                    "rsset" => Some(NumericValue::StringArraySet),
                    "rbset" => Some(NumericValue::BoolArraySet),
                    "dynarrayset" => Some(NumericValue::DynamicArraySet),
                    "dynarraysometruthy" => Some(NumericValue::DynamicArrayTruthy(0)),
                    "dynarrayeverytruthy" => Some(NumericValue::DynamicArrayTruthy(1)),
                    "dynarrayfindtruthy" => Some(NumericValue::DynamicArrayTruthy(2)),
                    "dynarrayfindindextruthy" => Some(NumericValue::DynamicArrayTruthy(3)),
                    "dynarrayfindlasttruthy" => Some(NumericValue::DynamicArrayTruthy(4)),
                    "dynarrayfindlastindextruthy" => Some(NumericValue::DynamicArrayTruthy(5)),
                    "dynarrayfiltertruthy" => Some(NumericValue::DynamicArrayTruthy(6)),
                    "dynarraymaptonumber" => Some(NumericValue::DynamicArrayConvert(0)),
                    "dynarraymaptoboolean" => Some(NumericValue::DynamicArrayConvert(1)),
                    "dynarraymaptostring" => Some(NumericValue::DynamicArrayConvert(2)),
                    "dynarraymapidentity" => Some(NumericValue::DynamicArrayMapIdentity),
                    _ if token.starts_with("dynarraymapjit") => {
                        let (target, captured) = match token.strip_prefix("dynarraymapjit")? {
                            "n" => (0, false),
                            "b" => (1, false),
                            "s" => (2, false),
                            "nc" => (0, true),
                            "bc" => (1, true),
                            "sc" => (2, true),
                            _ => return None,
                        };
                        Some(NumericValue::DynamicArrayJitMap(target, captured))
                    }
                    "dynarrayreducejit" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, false, false))
                    }
                    "dynarrayreducejit0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, false, false))
                    }
                    "dynarrayreducerightjit" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, true, false))
                    }
                    "dynarrayreducerightjit0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, true, false))
                    }
                    "dynarrayreducejitc" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, false, true))
                    }
                    "dynarrayreducejitc0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, false, true))
                    }
                    "dynarrayreducerightjitc" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, true, true))
                    }
                    "dynarrayreducerightjitc0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, true, true))
                    }
                    _ if token.starts_with("dynarray") && token.contains("jit") => {
                        let suffix = token.strip_prefix("dynarray")?;
                        let (mode, captured) = match suffix {
                            "somejit" => (0, false),
                            "everyjit" => (1, false),
                            "findjit" => (2, false),
                            "findindexjit" => (3, false),
                            "findlastjit" => (4, false),
                            "findlastindexjit" => (5, false),
                            "filterjit" => (6, false),
                            "somejitc" => (0, true),
                            "everyjitc" => (1, true),
                            "findjitc" => (2, true),
                            "findindexjitc" => (3, true),
                            "findlastjitc" => (4, true),
                            "findlastindexjitc" => (5, true),
                            "filterjitc" => (6, true),
                            _ => return None,
                        };
                        Some(NumericValue::DynamicArrayJitScan(mode, captured))
                    }
                    _ if token.strip_prefix("dynarray").is_some_and(|operation| {
                        [
                            "findlastindex",
                            "findlast",
                            "findindex",
                            "filter",
                            "every",
                            "some",
                            "find",
                        ]
                        .iter()
                        .any(|method| operation.starts_with(method))
                    }) =>
                    {
                        let operation = token.strip_prefix("dynarray")?;
                        let (operation, mode) = [
                            ("findlastindex", 5),
                            ("findlast", 4),
                            ("findindex", 3),
                            ("filter", 6),
                            ("every", 1),
                            ("some", 0),
                            ("find", 2),
                        ]
                        .into_iter()
                        .find_map(|(method, mode)| {
                            operation
                                .strip_prefix(method)
                                .map(|operation| (operation, mode))
                        })?;
                        let operation = match operation {
                            "lt" => 0,
                            "lte" => 1,
                            "gt" => 2,
                            "gte" => 3,
                            "eq" => 4,
                            "ne" => 5,
                            "seq" => 6,
                            "sne" => 7,
                            _ => return None,
                        };
                        Some(NumericValue::DynamicArrayCompare(operation, mode))
                    }
                    "arrayvalue" => Some(NumericValue::ArrayValue),
                    "arrayhandle" => Some(NumericValue::MutableArrayHandle),
                    "arrayempty" => Some(NumericValue::EmptyArray),
                    "rnmin" => Some(NumericValue::NumberArrayMin),
                    "rnmax" => Some(NumericValue::NumberArrayMax),
                    "rnhypot" => Some(NumericValue::NumberArrayHypot),
                    "rnreducejit" => Some(NumericValue::NumberArrayJitReduce(true, false, false)),
                    "rnreducejit0" => Some(NumericValue::NumberArrayJitReduce(false, false, false)),
                    "rnreducerightjit" => {
                        Some(NumericValue::NumberArrayJitReduce(true, true, false))
                    }
                    "rnreducerightjit0" => {
                        Some(NumericValue::NumberArrayJitReduce(false, true, false))
                    }
                    "rnreducejitc" => Some(NumericValue::NumberArrayJitReduce(true, false, true)),
                    "rnreducejitc0" => Some(NumericValue::NumberArrayJitReduce(false, false, true)),
                    "rnreducerightjitc" => {
                        Some(NumericValue::NumberArrayJitReduce(true, true, true))
                    }
                    "rnreducerightjitc0" => {
                        Some(NumericValue::NumberArrayJitReduce(false, true, true))
                    }
                    "rnpop" => Some(NumericValue::NumberArrayPop),
                    "rspop" => Some(NumericValue::StringArrayPop),
                    "rbpop" => Some(NumericValue::BoolArrayPop),
                    "dynarraypop" => Some(NumericValue::DynamicArrayPop),
                    "rnshift" => Some(NumericValue::NumberArrayShift),
                    "rsshift" => Some(NumericValue::StringArrayShift),
                    "rbshift" => Some(NumericValue::BoolArrayShift),
                    "dynarrayshift" => Some(NumericValue::DynamicArrayShift),
                    "drop" => Some(NumericValue::Drop),
                    "nip" => Some(NumericValue::DropUnder),
                    "dup" => Some(NumericValue::Duplicate),
                    "dup2" => Some(NumericValue::DuplicatePair),
                    "loop" => Some(NumericValue::LoopStart),
                    "while" => Some(NumericValue::LoopWhile),
                    "looptail" => Some(NumericValue::LoopContinuePoint),
                    "break" => Some(NumericValue::LoopBreak(0)),
                    value if value.starts_with("break") => value
                        .strip_prefix("break")?
                        .parse::<u8>()
                        .ok()
                        .map(NumericValue::LoopBreak),
                    "continue" => Some(NumericValue::LoopContinue(0)),
                    value if value.starts_with("continue") => value
                        .strip_prefix("continue")?
                        .parse::<u8>()
                        .ok()
                        .map(NumericValue::LoopContinue),
                    "loopend" => Some(NumericValue::LoopEnd),
                    "guard" => Some(NumericValue::GuardStart),
                    "guardelse" => Some(NumericValue::GuardAlternate),
                    "guardend" => Some(NumericValue::GuardEnd),
                    "switch" => Some(NumericValue::SwitchStart),
                    "case" => Some(NumericValue::SwitchCaseStart),
                    "casebody" => Some(NumericValue::SwitchCaseBody),
                    "default" => Some(NumericValue::SwitchDefault),
                    "switchbreak" => Some(NumericValue::SwitchBreak),
                    "switchend" => Some(NumericValue::SwitchEnd),
                    "trystart" => Some(NumericValue::TryStart),
                    "trystarttag" => Some(NumericValue::TaggedTryStart),
                    "catch" => Some(NumericValue::CatchStart),
                    "tryend" => Some(NumericValue::TryEnd),
                    "resultstart" => Some(NumericValue::ResultStart),
                    "resultreturn" => Some(NumericValue::ResultReturn(0)),
                    value if value.starts_with("resultreturn") => value
                        .strip_prefix("resultreturn")?
                        .parse::<u8>()
                        .ok()
                        .filter(|count| (1..=8).contains(count))
                        .map(NumericValue::ResultReturn),
                    "resultend" => Some(NumericValue::ResultEnd),
                    "throw" => Some(NumericValue::Throw),
                    value if value.starts_with("throwtag") => value
                        .strip_prefix("throwtag")?
                        .parse::<u8>()
                        .ok()
                        .filter(|tag| *tag <= 8)
                        .map(NumericValue::TaggedThrow),
                    "checkerror" => Some(NumericValue::CheckError),
                    "throwoutn" => Some(NumericValue::UncaughtNumberThrow),
                    "throwoutb" => Some(NumericValue::UncaughtBooleanThrow),
                    "throwouts" => Some(NumericValue::UncaughtStringThrow),
                    "throwoutrn" => Some(NumericValue::UncaughtNumberArrayThrow),
                    "throwoutrb" => Some(NumericValue::UncaughtBooleanArrayThrow),
                    "throwoutrs" => Some(NumericValue::UncaughtStringArrayThrow),
                    "throwoutd" => Some(NumericValue::UncaughtDictionaryThrow),
                    "return" => Some(NumericValue::EarlyReturn),
                    "random" => Some(NumericValue::MathRandom),
                    "datenow" => Some(NumericValue::DateNow),
                    "performancenow" => Some(NumericValue::PerformanceNow),
                    "processpid" => Some(NumericValue::ProcessPid),
                    "processppid" => Some(NumericValue::ProcessPpid),
                    "missingcalln" | "missingcallb" | "missingcalls" | "missingcalldyn"
                    | "missingcalla" | "missingcalld" => Some(NumericValue::MissingCallable),
                    "rnwith" => Some(NumericValue::NumberArrayWith),
                    "rswith" => Some(NumericValue::StringArrayWith),
                    "rbwith" => Some(NumericValue::BoolArrayWith),
                    "strbool" => Some(NumericValue::StringTruthy),
                    "padend" => Some(NumericValue::StringPadEnd),
                    "padstart" => Some(NumericValue::StringPadStart),
                    "startswith" => Some(NumericValue::StringStartsWith),
                    "startswith2" => Some(NumericValue::StringStartsWithAt),
                    "repeat" => Some(NumericValue::StringRepeat),
                    "normalize" => Some(NumericValue::StringNormalize),
                    "split" => Some(NumericValue::StringSplit),
                    "strarray" => Some(NumericValue::StringToArray),
                    "fromcharcode" => Some(NumericValue::StringFromCharCode),
                    "fromcodepoint" => Some(NumericValue::StringFromCodePoint),
                    "replace" => Some(NumericValue::StringReplace),
                    "replaceall" => Some(NumericValue::StringReplaceAll),
                    "slice" => Some(NumericValue::StringSlice),
                    "slice2" => Some(NumericValue::StringSliceRange),
                    "substring" => Some(NumericValue::StringSubstring),
                    "substring2" => Some(NumericValue::StringSubstringRange),
                    "tolowercase" => Some(NumericValue::StringToLowerCase),
                    "towellformed" => Some(NumericValue::StringToWellFormed),
                    "touppercase" => Some(NumericValue::StringToUpperCase),
                    "trim" => Some(NumericValue::StringTrim),
                    "trimend" => Some(NumericValue::StringTrimEnd),
                    "trimstart" => Some(NumericValue::StringTrimStart),
                    "pow" => Some(NumericValue::Power),
                    "%" => Some(NumericValue::Remainder),
                    "?" => Some(NumericValue::Select),
                    "&&" => Some(NumericValue::ShortCircuit(true)),
                    "||" => Some(NumericValue::ShortCircuit(false)),
                    "if" => Some(NumericValue::ConditionalStart),
                    "ifpresent" => Some(NumericValue::PresentConditionalStart),
                    "else" => Some(NumericValue::ConditionalAlternate),
                    "end" => Some(NumericValue::ShortCircuitEnd),
                    "absentn" | "absentb" | "absents" | "absentdyn" | "absenta" | "absentd" => {
                        Some(NumericValue::Absent)
                    }
                    "keepabsentn" | "keepabsentb" | "keepabsents" | "keepabsenta" => {
                        Some(NumericValue::PreserveAbsent)
                    }
                    "nulln" | "nullb" | "nulls" | "nulla" | "nulld" => Some(NumericValue::Null),
                    "asbool" => Some(NumericValue::AsBoolean),
                    "boolnot" => Some(NumericValue::BooleanNot),
                    "strictfalse" => Some(NumericValue::StrictMismatch(false)),
                    "stricttrue" => Some(NumericValue::StrictMismatch(true)),
                    value => UnaryMath::parse(value)
                        .map(NumericValue::UnaryMath)
                        .or_else(|| {
                            value.strip_prefix("rnreduce").and_then(|operation| {
                                let (operation, from_right) = operation
                                    .strip_prefix("right")
                                    .map_or((operation, false), |operation| (operation, true));
                                let (operation, has_initial) = operation
                                    .strip_suffix('0')
                                    .map_or((operation, true), |operation| (operation, false));
                                NumericReduceOp::parse(operation).map(|operation| {
                                    NumericValue::NumberArrayReduce(
                                        operation,
                                        has_initial,
                                        from_right,
                                    )
                                })
                            })
                        })
                        .or_else(|| {
                            let (operation, every) = value
                                .strip_prefix("rnsome")
                                .map(|operation| (operation, false))
                                .or_else(|| {
                                    value
                                        .strip_prefix("rnevery")
                                        .map(|operation| (operation, true))
                                })?;
                            let operation = match operation {
                                "lt" => CompareOp::Less,
                                "lte" => CompareOp::LessEqual,
                                "gt" => CompareOp::Greater,
                                "gte" => CompareOp::GreaterEqual,
                                "eq" => CompareOp::Equal,
                                "ne" => CompareOp::NotEqual,
                                _ => return None,
                            };
                            Some(NumericValue::NumberArrayQuantifier(operation, every))
                        })
                        .or_else(|| {
                            let (operation, mode) = value
                                .strip_prefix("rnfindlastindex")
                                .map(|operation| (operation, 3))
                                .or_else(|| {
                                    value
                                        .strip_prefix("rnfindlast")
                                        .map(|operation| (operation, 2))
                                })
                                .or_else(|| {
                                    value
                                        .strip_prefix("rnfindindex")
                                        .map(|operation| (operation, 1))
                                })
                                .or_else(|| {
                                    value.strip_prefix("rnfind").map(|operation| (operation, 0))
                                })?;
                            let operation = match operation {
                                "lt" => CompareOp::Less,
                                "lte" => CompareOp::LessEqual,
                                "gt" => CompareOp::Greater,
                                "gte" => CompareOp::GreaterEqual,
                                "eq" => CompareOp::Equal,
                                "ne" => CompareOp::NotEqual,
                                _ => return None,
                            };
                            Some(NumericValue::NumberArrayFind(operation, mode))
                        })
                        .or_else(|| {
                            let operation = match value.strip_prefix("rnfilter")? {
                                "lt" => CompareOp::Less,
                                "lte" => CompareOp::LessEqual,
                                "gt" => CompareOp::Greater,
                                "gte" => CompareOp::GreaterEqual,
                                "eq" => CompareOp::Equal,
                                "ne" => CompareOp::NotEqual,
                                _ => return None,
                            };
                            Some(NumericValue::NumberArrayFilter(operation))
                        })
                        .or_else(|| {
                            let (kind, operation) = value
                                .strip_prefix("rn")
                                .map(|operation| (0, operation))
                                .or_else(|| {
                                    value.strip_prefix("rb").map(|operation| (1, operation))
                                })
                                .or_else(|| {
                                    value.strip_prefix("rs").map(|operation| (2, operation))
                                })?;
                            let mode = match operation {
                                "sometruthy" => 0,
                                "everytruthy" => 1,
                                "findtruthy" => 2,
                                "findindextruthy" => 3,
                                "findlasttruthy" => 4,
                                "findlastindextruthy" => 5,
                                "filtertruthy" => 6,
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayTruthy(kind, mode))
                        })
                        .or_else(|| {
                            let (kind, operation) = value
                                .strip_prefix("rb")
                                .map(|operation| (1, operation))
                                .or_else(|| {
                                    value.strip_prefix("rs").map(|operation| (2, operation))
                                })?;
                            let (operation, mode) = operation
                                .strip_prefix("some")
                                .map(|operation| (operation, 0))
                                .or_else(|| {
                                    operation
                                        .strip_prefix("every")
                                        .map(|operation| (operation, 1))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("findlastindex")
                                        .map(|operation| (operation, 5))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("findlast")
                                        .map(|operation| (operation, 4))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("findindex")
                                        .map(|operation| (operation, 3))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("find")
                                        .map(|operation| (operation, 2))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("filter")
                                        .map(|operation| (operation, 6))
                                })?;
                            Some(NumericValue::PrimitiveArrayCompare(
                                kind,
                                CompareOp::parse(operation)?,
                                mode,
                            ))
                        })
                        .or_else(|| {
                            let (kind, operation) = value
                                .strip_prefix("rbmap")
                                .map(|operation| (1, operation))
                                .or_else(|| {
                                    value.strip_prefix("rsmap").map(|operation| (2, operation))
                                })?;
                            let operation = match operation {
                                "identity" => 0,
                                "not" if kind == 1 => 1,
                                "tolowercase" if kind == 2 => 2,
                                "touppercase" if kind == 2 => 3,
                                "trim" if kind == 2 => 4,
                                "trimstart" if kind == 2 => 5,
                                "trimend" if kind == 2 => 6,
                                "length" if kind == 2 => 7,
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayMap(kind, operation))
                        })
                        .or_else(|| {
                            let (source, target) = value
                                .strip_prefix("rnmapto")
                                .map(|target| (0, target))
                                .or_else(|| value.strip_prefix("rbmapto").map(|target| (1, target)))
                                .or_else(|| {
                                    value.strip_prefix("rsmapto").map(|target| (2, target))
                                })?;
                            Some(NumericValue::PrimitiveArrayConvert(
                                source,
                                match target {
                                    "number" => 0,
                                    "boolean" => 1,
                                    "string" => 2,
                                    _ => return None,
                                },
                            ))
                        })
                        .or_else(|| {
                            let encoded =
                                u16::from_str_radix(value.strip_prefix("rnmapbranch")?, 16).ok()?;
                            let operation = encoded & 7;
                            let true_branch = (encoded >> 4) & 15;
                            let false_branch = (encoded >> 8) & 15;
                            (operation <= 5 && true_branch <= 13 && false_branch <= 13)
                                .then_some(NumericValue::NumberArrayBranchMap(encoded))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("rnmapselect")?;
                            let (operation, mode) = encoded.split_at(encoded.len().checked_sub(1)?);
                            Some(NumericValue::NumberArraySelectMap(
                                CompareOp::parse(operation)?,
                                mode.parse::<u8>().ok().filter(|mode| *mode < 4)?,
                            ))
                        })
                        .or_else(|| {
                            let operation = value.strip_prefix("rnmapindex")?;
                            let (operation, reverse) = NumericReduceOp::parse(operation)
                                .map(|operation| (operation, false))
                                .or_else(|| {
                                    NumericReduceOp::parse(operation.strip_prefix('r')?)
                                        .map(|operation| (operation, true))
                                })?;
                            (!matches!(
                                operation,
                                NumericReduceOp::Minimum | NumericReduceOp::Maximum
                            ))
                            .then_some(NumericValue::NumberArrayIndexMap(operation, reverse))
                        })
                        .or_else(|| {
                            Some(NumericValue::NumberArrayUnaryMap(match value {
                                "rnmapneg" => false,
                                "rnmapabs" => true,
                                _ => return None,
                            }))
                        })
                        .or(match value {
                            "rnmapjit" => Some(NumericValue::PrimitiveArrayJitMap(0, 0, false)),
                            "rnmapjitc" => Some(NumericValue::PrimitiveArrayJitMap(0, 0, true)),
                            _ => None,
                        })
                        .or_else(|| {
                            let (source, suffix) = ["rn", "rb", "rs"].iter().enumerate().find_map(
                                |(source, prefix)| {
                                    value
                                        .strip_prefix(prefix)
                                        .map(|suffix| (source as u8, suffix))
                                },
                            )?;
                            let suffix = suffix.strip_prefix("mapjit")?;
                            let (target, captured) = match suffix {
                                "n" => (0, false),
                                "b" => (1, false),
                                "s" => (2, false),
                                "nc" => (0, true),
                                "bc" => (1, true),
                                "sc" => (2, true),
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayJitMap(source, target, captured))
                        })
                        .or_else(|| {
                            let (kind, suffix) = ["rn", "rb", "rs"].iter().enumerate().find_map(
                                |(kind, prefix)| {
                                    value
                                        .strip_prefix(prefix)
                                        .map(|suffix| (kind as u8, suffix))
                                },
                            )?;
                            let (mode, captured) = match suffix {
                                "somejit" => (0, false),
                                "everyjit" => (1, false),
                                "findjit" => (2, false),
                                "findindexjit" => (3, false),
                                "findlastjit" => (4, false),
                                "findlastindexjit" => (5, false),
                                "filterjit" => (6, false),
                                "somejitc" => (0, true),
                                "everyjitc" => (1, true),
                                "findjitc" => (2, true),
                                "findindexjitc" => (3, true),
                                "findlastjitc" => (4, true),
                                "findlastindexjitc" => (5, true),
                                "filterjitc" => (6, true),
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayJitScan(kind, mode, captured))
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("rnmap")
                                .and_then(UnaryMath::parse)
                                .map(NumericValue::NumberArrayMathMap)
                        })
                        .or_else(|| {
                            let operation = value.strip_prefix("rnmap")?;
                            let (operation, reverse) = NumericReduceOp::parse(operation)
                                .map(|operation| (operation, false))
                                .or_else(|| {
                                    NumericReduceOp::parse(operation.strip_prefix('r')?)
                                        .map(|operation| (operation, true))
                                })?;
                            (!matches!(
                                operation,
                                NumericReduceOp::Minimum | NumericReduceOp::Maximum
                            ))
                            .then_some(NumericValue::NumberArrayMap(operation, reverse))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("recur")?;
                            matches!(encoded.as_bytes().first(), Some(b'n' | b'b' | b's'))
                                .then_some(encoded.get(1..)?)?
                                .parse::<u8>()
                                .ok()
                                .filter(|arity| (1..=8).contains(arity))
                                .map(NumericValue::Recur)
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix('u')?;
                            let digits = encoded.bytes().take_while(u8::is_ascii_digit).count();
                            let (index, kinds) = encoded.split_at(digits);
                            (!kinds.is_empty()
                                && kinds.bytes().all(|kind| {
                                    matches!(
                                        kind,
                                        b'n' | b'b'
                                            | b's'
                                            | b'N'
                                            | b'B'
                                            | b'S'
                                            | b'D'
                                            | b'E'
                                            | b'F'
                                            | b'O'
                                            | b'T'
                                            | b'X'
                                            | b'Y'
                                            | b'Z'
                                    )
                                }))
                            .then_some(index)?
                            .parse::<u8>()
                            .ok()
                            .filter(|index| *index < 15)
                            .map(NumericValue::DynamicArgument)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("objnew")?
                                .parse::<u16>()
                                .ok()
                                .filter(|size| *size > 0)
                                .map(NumericValue::FixedObjectNew)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("tupneww")?
                                .parse::<u16>()
                                .ok()
                                .map(NumericValue::FixedWideTupleNew)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("tupnew")?
                                .parse::<u16>()
                                .ok()
                                .map(NumericValue::FixedTupleNew)
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("tupnull")?;
                            let (kind, index) = if let Some(index) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, index)
                            } else {
                                let (kind, index) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    index,
                                )
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::NullishTupleField(kind, index))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("tupopt")?;
                            let (kind, index) = if let Some(index) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, index)
                            } else {
                                let (kind, index) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    index,
                                )
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::OptionalTupleField(kind, index))
                        })
                        .or_else(|| {
                            let (mode, encoded) = value
                                .strip_prefix("tupsetnull")
                                .map(|encoded| (2, encoded))
                                .or_else(|| {
                                    value.strip_prefix("tupsetopt").map(|encoded| (1, encoded))
                                })
                                .or_else(|| {
                                    value.strip_prefix("tupsetw").map(|encoded| (0, encoded))
                                })?;
                            let (kind, index) = encoded.split_at(1);
                            let kind = match kind {
                                "n" => 0,
                                "b" => 1,
                                "s" | "p" | "o" => 2,
                                "u" => 3,
                                _ => return None,
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::FixedWideTupleSet(kind, index, mode))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("tupset")?;
                            let (kind, index) = encoded.split_at(1);
                            let kind = match kind {
                                "n" => 0,
                                "b" => 1,
                                "s" | "p" | "o" => 2,
                                _ => return None,
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::FixedTupleSet(kind, index))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objca")?;
                            let semantic = match encoded.as_bytes().first()? {
                                b'o' => 0,
                                b'l' => 1,
                                b'n' => 2,
                                _ => return None,
                            };
                            let operation = match encoded.as_bytes().get(1)? {
                                b'a' => 0,
                                b's' => 1,
                                b'm' => 2,
                                b'd' => 3,
                                b'r' => 4,
                                b'l' => 5,
                                b'h' => 6,
                                b'u' => 7,
                                b'o' => 8,
                                b'x' => 9,
                                b'b' => 10,
                                b'p' => 11,
                                _ => return None,
                            };
                            encoded.get(2..)?.parse::<u16>().ok().map(|offset| {
                                NumericValue::TaggedObjectNumberAssign(
                                    semantic * 12 + operation,
                                    offset,
                                )
                            })
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objup")?;
                            let semantic = match encoded.as_bytes().first()? {
                                b'o' => 0,
                                b'l' => 1,
                                b'n' => 2,
                                _ => return None,
                            };
                            let decrement = match encoded.as_bytes().get(1)? {
                                b'i' => 0,
                                b'd' => 2,
                                _ => return None,
                            };
                            let postfix = match encoded.as_bytes().get(2)? {
                                b'p' => 0,
                                b'o' => 1,
                                _ => return None,
                            };
                            encoded.get(3..)?.parse::<u16>().ok().map(|offset| {
                                NumericValue::TaggedObjectNumberUpdate(
                                    semantic * 4 + decrement + postfix,
                                    offset,
                                )
                            })
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objset")?;
                            let (kind, offset) = encoded.split_at(1);
                            let kind = match kind {
                                "n" => 0,
                                "b" => 1,
                                "s" | "a" | "o" => 2,
                                "u" => 3,
                                _ => return None,
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::FixedObjectSet(kind, offset))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objnull")?;
                            let (kind, offset) = if let Some(offset) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, offset)
                            } else {
                                let (kind, offset) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    offset,
                                )
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::NullishObjectField(kind, offset))
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("objnullablen")?
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::OptionalObjectField(0, offset))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objopt")?;
                            let (kind, offset) = if let Some(offset) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, offset)
                            } else {
                                let (kind, offset) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "a" | "o" | "t" | "d" => 2,
                                        _ => return None,
                                    },
                                    offset,
                                )
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::OptionalObjectField(kind, offset))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("obj")?;
                            let (kind, offset) = if let Some(offset) = encoded.strip_prefix("rn") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("rb") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("rs") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("dn") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("db") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("ds") {
                                (2, offset)
                            } else {
                                let (kind, offset) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    offset,
                                )
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::ObjectField(kind, offset))
                        })
                        .or_else(|| {
                            value
                                .strip_prefix('a')
                                .or_else(|| value.strip_prefix('b'))
                                .or_else(|| value.strip_prefix('s'))
                                .or_else(|| value.strip_prefix("rn"))
                                .or_else(|| value.strip_prefix("rb"))
                                .or_else(|| value.strip_prefix("rs"))
                                .or_else(|| value.strip_prefix("dn"))
                                .or_else(|| value.strip_prefix("db"))
                                .or_else(|| value.strip_prefix("ds"))
                                .or_else(|| value.strip_prefix("en"))
                                .or_else(|| value.strip_prefix("eb"))
                                .or_else(|| value.strip_prefix("es"))
                                .and_then(|index| index.parse::<u8>().ok())
                                .filter(|index| *index < 16)
                                .map(NumericValue::Argument)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("setl")
                                .and_then(|index| index.parse::<u8>().ok())
                                .filter(|index| *index < 8)
                                .map(NumericValue::LocalSet)
                        })
                        .or_else(|| {
                            ["dnput", "dbput", "dsput"].iter().enumerate().find_map(
                                |(kind, prefix)| {
                                    value.strip_prefix(prefix).and_then(|key| {
                                        intern_string(key).map(|key| {
                                            NumericValue::DictionaryStaticAppend(kind as u8, key)
                                        })
                                    })
                                },
                            )
                        })
                        .or_else(|| {
                            let (kind, index) =
                                ["rnlset", "rblset", "rslset", "dnlset", "dblset", "dslset"]
                                    .iter()
                                    .enumerate()
                                    .find_map(|(kind, prefix)| {
                                        value.strip_prefix(prefix).map(|index| (kind as u8, index))
                                    })?;
                            index
                                .parse::<u8>()
                                .ok()
                                .filter(|index| *index < 8)
                                .map(|index| NumericValue::AggregateLocalSet(kind, index))
                        })
                        .or_else(|| {
                            let (kind, index, unshift) = ["rn", "rb", "rs"]
                                .iter()
                                .enumerate()
                                .find_map(|(kind, prefix)| {
                                    value
                                        .strip_prefix(&format!("{prefix}lpush"))
                                        .map(|index| (kind as u8, index, false))
                                        .or_else(|| {
                                            value
                                                .strip_prefix(&format!("{prefix}lunshift"))
                                                .map(|index| (kind as u8, index, true))
                                        })
                                })?;
                            index
                                .parse::<u8>()
                                .ok()
                                .filter(|index| *index < 8)
                                .map(|index| {
                                    NumericValue::AggregateLocalArrayInsert(kind, index, unshift)
                                })
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("rnl")
                                .or_else(|| value.strip_prefix("rbl"))
                                .or_else(|| value.strip_prefix("rsl"))
                                .or_else(|| value.strip_prefix("dnl"))
                                .or_else(|| value.strip_prefix("dbl"))
                                .or_else(|| value.strip_prefix("dsl"))
                                .or_else(|| value.strip_prefix("ln"))
                                .or_else(|| value.strip_prefix("lb"))
                                .or_else(|| value.strip_prefix("ls"))
                                .or_else(|| value.strip_prefix("ld"))
                                .or_else(|| value.strip_prefix('l'))
                                .and_then(|index| index.parse::<u8>().ok())
                                .filter(|index| *index < 8)
                                .map(NumericValue::LocalGet)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix('t')
                                .and_then(intern_string)
                                .map(NumericValue::StringConstant)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix('c')
                                .filter(|bits| bits.len() == 16)
                                .and_then(|bits| u64::from_str_radix(bits, 16).ok())
                                .map(|bits| NumericValue::Constant(f64::from_bits(bits)))
                        }),
                })
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty() && values.len() <= 256).then_some(Self(values))
        } else {
            let operation = symbol.split_once(':').map_or(symbol, |pair| pair.0);
            Some(Self(vec![
                NumericValue::Argument(0),
                NumericValue::Argument(1),
                NumericValue::Operation(NumericOp::parse(operation)?),
            ]))
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn machine_code(&self) -> Option<Vec<u8>> {
        let mut code = Vec::with_capacity(self.0.len() * 12 + 8);
        let mut depth = 0u8;
        let mut branches = Vec::new();
        let mut loops = Vec::new();
        let mut guards = Vec::new();
        let mut switches = Vec::new();
        let mut tries = Vec::new();
        let mut catches = Vec::new();
        let mut results = Vec::new();
        for value in &self.0 {
            match value {
                NumericValue::Argument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    depth += 1;
                }
                NumericValue::DynamicArgument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    code.extend_from_slice(&[
                        0xf2,
                        0x0f,
                        0x10,
                        0x47 | ((depth + 1) << 3),
                        (index + 1) * 8,
                    ]);
                    emit_binary_call(&mut code, dynamic_from_parts as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::Constant(value) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&value.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    depth += 1;
                }
                NumericValue::StringConstant(value) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(*value as usize as u64).to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    depth += 1;
                }
                NumericValue::Operation(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let right = depth - 1;
                    let left = depth - 2;
                    code.extend_from_slice(&[
                        0xf2,
                        0x0f,
                        operation.opcode(),
                        0xc0 | (left << 3) | right,
                    ]);
                    depth -= 1;
                }
                NumericValue::Compare(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let right = depth - 1;
                    let left = depth - 2;
                    emit_compare(&mut code, left, right, *operation);
                    depth -= 1;
                }
                NumericValue::Bitwise(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(
                        &mut code,
                        operation.function() as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::BitNot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, bit_not as *const () as u64, depth - 1);
                }
                NumericValue::Absolute => {
                    if depth == 0 {
                        return None;
                    }
                    let value = depth - 1;
                    emit_bit_operation(&mut code, value, 0xf0);
                }
                NumericValue::Negate => {
                    if depth == 0 {
                        return None;
                    }
                    let value = depth - 1;
                    emit_bit_operation(&mut code, value, 0xf8);
                }
                NumericValue::Remainder => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, fmod as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Minimum | NumericValue::Maximum => {
                    if depth < 2 {
                        return None;
                    }
                    let function = if matches!(value, NumericValue::Minimum) {
                        minimum
                    } else {
                        maximum
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Power => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, power as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Atan2 | NumericValue::Hypot | NumericValue::Imul => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::Atan2 => atan2_number,
                        NumericValue::Hypot => hypot_number,
                        NumericValue::Imul => imul_number,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::IsFinite
                | NumericValue::IsInteger
                | NumericValue::IsNaN
                | NumericValue::IsSafeInteger => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::IsFinite => number_is_finite,
                        NumericValue::IsInteger => number_is_integer,
                        NumericValue::IsNaN => number_is_nan,
                        NumericValue::IsSafeInteger => number_is_safe_integer,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringCompare => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, string_compare as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberSameValue
                | NumericValue::StringSameValue
                | NumericValue::ReferenceSameValue => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberSameValue => number_same_value,
                        NumericValue::StringSameValue => string_same_value,
                        NumericValue::ReferenceSameValue => reference_same_value,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::TypeOfNumber
                | NumericValue::TypeOfBoolean
                | NumericValue::TypeOfString
                | NumericValue::TypeOfObject
                | NumericValue::TypeOfDynamic => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::TypeOfNumber => type_of_number,
                        NumericValue::TypeOfBoolean => type_of_boolean,
                        NumericValue::TypeOfString => type_of_string,
                        NumericValue::TypeOfObject => type_of_object,
                        NumericValue::TypeOfDynamic => type_of_dynamic,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::DynamicToBoolean => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dynamic_to_boolean as *const () as u64, depth - 1);
                }
                NumericValue::DynamicAdd | NumericValue::DynamicCompare(_) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::DynamicAdd => dynamic_add,
                        NumericValue::DynamicCompare(operation) => [
                            dynamic_less,
                            dynamic_less_equal,
                            dynamic_greater,
                            dynamic_greater_equal,
                            dynamic_equal,
                            dynamic_not_equal,
                            dynamic_strict_equal,
                            dynamic_strict_not_equal,
                        ][usize::from(*operation)],
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringCharAt
                | NumericValue::StringCharCodeAt
                | NumericValue::StringAt
                | NumericValue::StringCodePointAt => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringCharAt => string_char_at,
                        NumericValue::StringCharCodeAt => string_char_code_at,
                        NumericValue::StringAt => string_at,
                        NumericValue::StringCodePointAt => string_code_point_at,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, string_concat as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberToString
                | NumericValue::BooleanToString
                | NumericValue::StringToNumber
                | NumericValue::DynamicToString
                | NumericValue::DynamicToNumber
                | NumericValue::TagNumber
                | NumericValue::TagString
                | NumericValue::TagBoolean
                | NumericValue::TagAggregate(_)
                | NumericValue::DynamicTag
                | NumericValue::UntagNumber
                | NumericValue::UntagString
                | NumericValue::UntagBoolean
                | NumericValue::UntagNumberArray
                | NumericValue::UntagBooleanArray
                | NumericValue::UntagStringArray
                | NumericValue::UntagArray
                | NumericValue::UntagNumberDictionary
                | NumericValue::UntagBooleanDictionary
                | NumericValue::UntagStringDictionary
                | NumericValue::UntagDictionary
                | NumericValue::UntagObject
                | NumericValue::UntagTuple
                | NumericValue::ParseFloat
                | NumericValue::NumberToExponentialShortest => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberToString => number_to_string,
                        NumericValue::BooleanToString => boolean_to_string,
                        NumericValue::StringToNumber => string_to_number,
                        NumericValue::DynamicToString => dynamic_to_string,
                        NumericValue::DynamicToNumber => dynamic_to_number,
                        NumericValue::TagNumber => tag_number,
                        NumericValue::TagString => tag_string,
                        NumericValue::TagBoolean => tag_boolean,
                        NumericValue::TagAggregate(kind) => [
                            tag_number_array,
                            tag_boolean_array,
                            tag_string_array,
                            tag_number_dictionary,
                            tag_boolean_dictionary,
                            tag_string_dictionary,
                            tag_object,
                            tag_tuple,
                        ][*kind as usize],
                        NumericValue::DynamicTag => dynamic_tag,
                        NumericValue::UntagNumber => untag_number,
                        NumericValue::UntagString => untag_string,
                        NumericValue::UntagBoolean => untag_boolean,
                        NumericValue::UntagNumberArray => untag_number_array,
                        NumericValue::UntagBooleanArray => untag_boolean_array,
                        NumericValue::UntagStringArray => untag_string_array,
                        NumericValue::UntagArray => untag_array,
                        NumericValue::UntagNumberDictionary => untag_number_dictionary,
                        NumericValue::UntagBooleanDictionary => untag_boolean_dictionary,
                        NumericValue::UntagStringDictionary => untag_string_dictionary,
                        NumericValue::UntagDictionary => untag_dictionary,
                        NumericValue::UntagObject => untag_object,
                        NumericValue::UntagTuple => untag_tuple,
                        NumericValue::ParseFloat => parse_float,
                        NumericValue::NumberToExponentialShortest => number_to_exponential_shortest,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::ObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_number_field,
                        object_boolean_field,
                        object_string_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::OptionalObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_optional_number_field,
                        object_optional_boolean_field,
                        object_optional_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NullishObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_nullish_number_field,
                        object_nullish_boolean_field,
                        object_nullish_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::OptionalTupleField(kind, index) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        optional_tuple_number_field,
                        optional_tuple_boolean_field,
                        optional_tuple_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NullishTupleField(kind, index) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        nullish_tuple_number_field,
                        nullish_tuple_boolean_field,
                        nullish_tuple_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::FixedObjectNew(size) => {
                    if depth == 8 {
                        return None;
                    }
                    let size = f64::from(*size);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&size.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_object_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedObjectSet(kind, offset) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        fixed_object_set_number,
                        fixed_object_set_boolean,
                        fixed_object_set_string,
                        fixed_object_set_byte,
                    ][usize::from(*kind)];
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::TaggedObjectNumberUpdate(mode, offset) => {
                    if depth == 0 || depth > 6 {
                        return None;
                    }
                    for (index, value) in [f64::from(*offset), f64::from(*mode)]
                        .into_iter()
                        .enumerate()
                    {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&value.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | ((depth + index as u8) << 3),
                        ]);
                    }
                    emit_ternary_call(
                        &mut code,
                        tagged_object_number_update as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::TaggedObjectNumberAssign(mode, offset) => {
                    if !(2..=6).contains(&depth) {
                        return None;
                    }
                    for (index, value) in [f64::from(*offset), f64::from(*mode)]
                        .into_iter()
                        .enumerate()
                    {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&value.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | ((depth + index as u8) << 3),
                        ]);
                    }
                    emit_quaternary_call(
                        &mut code,
                        tagged_object_number_assign as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::FixedTupleNew(length) => {
                    if depth == 8 {
                        return None;
                    }
                    let length = f64::from(*length);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&length.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_tuple_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedWideTupleNew(length) => {
                    if depth == 8 {
                        return None;
                    }
                    let length = f64::from(*length);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&length.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_wide_tuple_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedTupleSet(kind, index) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        fixed_tuple_set_number,
                        fixed_tuple_set_boolean,
                        fixed_tuple_set_pointer,
                    ][usize::from(*kind)];
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::FixedWideTupleSet(kind, index, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let functions = if *mode == 1 {
                        [
                            fixed_wide_tuple_set_optional_number,
                            fixed_wide_tuple_set_optional_boolean,
                            fixed_wide_tuple_set_optional_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    } else if *mode == 2 {
                        [
                            fixed_wide_tuple_set_nullish_number,
                            fixed_wide_tuple_set_nullish_boolean,
                            fixed_wide_tuple_set_nullish_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    } else {
                        [
                            fixed_wide_tuple_set_number,
                            fixed_wide_tuple_set_boolean,
                            fixed_wide_tuple_set_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    };
                    emit_ternary_call(
                        &mut code,
                        functions[usize::from(*kind)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::ExcludeNumber
                | NumericValue::ExcludeString
                | NumericValue::ExcludeBoolean
                | NumericValue::ExcludeArray
                | NumericValue::ExcludeObject => {}
                NumericValue::GlobalGet => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, global_get as *const () as u64, depth - 1);
                }
                NumericValue::GlobalSet => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, global_set as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::GlobalInit => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, global_init as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::CallableEntryGet => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, callable_entry_get as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::CallableEntrySet => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, callable_entry_set as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::ParseInt
                | NumericValue::NumberToFixed
                | NumericValue::NumberToPrecision
                | NumericValue::NumberToRadixString
                | NumericValue::NumberToExponential => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::ParseInt => parse_int,
                        NumericValue::NumberToFixed => number_to_fixed,
                        NumericValue::NumberToPrecision => number_to_precision,
                        NumericValue::NumberToRadixString => number_to_radix_string,
                        NumericValue::NumberToExponential => number_to_exponential,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringStartsWith
                | NumericValue::StringEndsWith
                | NumericValue::StringIncludes
                | NumericValue::StringIndexOf
                | NumericValue::StringLastIndexOf
                | NumericValue::StringRepeat
                | NumericValue::StringNormalize
                | NumericValue::StringSlice
                | NumericValue::StringSubstring => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringStartsWith => string_starts_with,
                        NumericValue::StringEndsWith => string_ends_with,
                        NumericValue::StringIncludes => string_includes,
                        NumericValue::StringIndexOf => string_index_of,
                        NumericValue::StringLastIndexOf => string_last_index_of,
                        NumericValue::StringRepeat => string_repeat,
                        NumericValue::StringNormalize => string_normalize,
                        NumericValue::StringSlice => string_slice,
                        NumericValue::StringSubstring => string_substring,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringSliceRange
                | NumericValue::StringSubstringRange
                | NumericValue::StringPadStart
                | NumericValue::StringPadEnd
                | NumericValue::StringStartsWithAt
                | NumericValue::StringEndsWithAt
                | NumericValue::StringIncludesAt
                | NumericValue::StringIndexOfAt
                | NumericValue::StringLastIndexOfAt
                | NumericValue::StringReplace
                | NumericValue::StringReplaceAll => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringSliceRange => string_slice_range,
                        NumericValue::StringSubstringRange => string_substring_range,
                        NumericValue::StringPadStart => string_pad_start,
                        NumericValue::StringPadEnd => string_pad_end,
                        NumericValue::StringStartsWithAt => string_starts_with_at,
                        NumericValue::StringEndsWithAt => string_ends_with_at,
                        NumericValue::StringIncludesAt => string_includes_at,
                        NumericValue::StringIndexOfAt => string_index_of_at,
                        NumericValue::StringLastIndexOfAt => string_last_index_of_at,
                        NumericValue::StringReplace => string_replace_first,
                        NumericValue::StringReplaceAll => string_replace_all,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringSplit => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, string_split as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringToArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_to_array as *const () as u64, depth - 1);
                }
                NumericValue::StringFromCharCode | NumericValue::StringFromCodePoint => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringFromCharCode => string_from_char_code,
                        NumericValue::StringFromCodePoint => string_from_code_point,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayMin | NumericValue::NumberArrayMax => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayMin => array_min,
                        NumericValue::NumberArrayMax => array_max,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayHypot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_hypot as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayReduce(operation, has_initial, from_right) => {
                    if depth < 2 {
                        return None;
                    }
                    type ReduceFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [ReduceFn; 4] = match operation {
                        NumericReduceOp::Add => [
                            number_array_reduce_add,
                            number_array_reduce_add_first,
                            number_array_reduce_add_right,
                            number_array_reduce_add_last,
                        ],
                        NumericReduceOp::Subtract => [
                            number_array_reduce_subtract,
                            number_array_reduce_subtract_first,
                            number_array_reduce_subtract_right,
                            number_array_reduce_subtract_last,
                        ],
                        NumericReduceOp::Multiply => [
                            number_array_reduce_multiply,
                            number_array_reduce_multiply_first,
                            number_array_reduce_multiply_right,
                            number_array_reduce_multiply_last,
                        ],
                        NumericReduceOp::Divide => [
                            number_array_reduce_divide,
                            number_array_reduce_divide_first,
                            number_array_reduce_divide_right,
                            number_array_reduce_divide_last,
                        ],
                        NumericReduceOp::Remainder => [
                            number_array_reduce_remainder,
                            number_array_reduce_remainder_first,
                            number_array_reduce_remainder_right,
                            number_array_reduce_remainder_last,
                        ],
                        NumericReduceOp::Power => [
                            number_array_reduce_power,
                            number_array_reduce_power_first,
                            number_array_reduce_power_right,
                            number_array_reduce_power_last,
                        ],
                        NumericReduceOp::Minimum => [
                            number_array_reduce_minimum,
                            number_array_reduce_minimum_first,
                            number_array_reduce_minimum_right,
                            number_array_reduce_minimum_last,
                        ],
                        NumericReduceOp::Maximum => [
                            number_array_reduce_maximum,
                            number_array_reduce_maximum_first,
                            number_array_reduce_maximum_right,
                            number_array_reduce_maximum_last,
                        ],
                    };
                    let function = functions[*from_right as usize * 2 + usize::from(!*has_initial)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayJitReduce(has_initial, from_right, captured) => {
                    if *captured {
                        if depth < 4 {
                            return None;
                        }
                        let function = match (has_initial, from_right) {
                            (true, false) => number_array_jit_reduce_initial_captured,
                            (false, false) => number_array_jit_reduce_first_captured,
                            (true, true) => number_array_jit_reduce_right_initial_captured,
                            (false, true) => number_array_jit_reduce_right_last_captured,
                        };
                        emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                        depth -= 3;
                    } else {
                        if depth < 3 {
                            return None;
                        }
                        let function = match (has_initial, from_right) {
                            (true, false) => number_array_jit_reduce_initial,
                            (false, false) => number_array_jit_reduce_first,
                            (true, true) => number_array_jit_reduce_right_initial,
                            (false, true) => number_array_jit_reduce_right_last,
                        };
                        emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                        depth -= 2;
                    }
                }
                NumericValue::NumberArrayQuantifier(operation, every) => {
                    if depth < 2 {
                        return None;
                    }
                    type QuantifierFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [QuantifierFn; 2] = match operation {
                        CompareOp::Less => [number_array_some_lt, number_array_every_lt],
                        CompareOp::LessEqual => [number_array_some_lte, number_array_every_lte],
                        CompareOp::Greater => [number_array_some_gt, number_array_every_gt],
                        CompareOp::GreaterEqual => [number_array_some_gte, number_array_every_gte],
                        CompareOp::Equal => [number_array_some_eq, number_array_every_eq],
                        CompareOp::NotEqual => [number_array_some_ne, number_array_every_ne],
                    };
                    emit_binary_call(
                        &mut code,
                        functions[usize::from(*every)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayFind(operation, mode) => {
                    if depth < 2 {
                        return None;
                    }
                    type FindFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [FindFn; 4] = match operation {
                        CompareOp::Less => [
                            number_array_find_lt,
                            number_array_find_index_lt,
                            number_array_find_last_lt,
                            number_array_find_last_index_lt,
                        ],
                        CompareOp::LessEqual => [
                            number_array_find_lte,
                            number_array_find_index_lte,
                            number_array_find_last_lte,
                            number_array_find_last_index_lte,
                        ],
                        CompareOp::Greater => [
                            number_array_find_gt,
                            number_array_find_index_gt,
                            number_array_find_last_gt,
                            number_array_find_last_index_gt,
                        ],
                        CompareOp::GreaterEqual => [
                            number_array_find_gte,
                            number_array_find_index_gte,
                            number_array_find_last_gte,
                            number_array_find_last_index_gte,
                        ],
                        CompareOp::Equal => [
                            number_array_find_eq,
                            number_array_find_index_eq,
                            number_array_find_last_eq,
                            number_array_find_last_index_eq,
                        ],
                        CompareOp::NotEqual => [
                            number_array_find_ne,
                            number_array_find_index_ne,
                            number_array_find_last_ne,
                            number_array_find_last_index_ne,
                        ],
                    };
                    emit_binary_call(
                        &mut code,
                        functions[*mode as usize] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayFilter(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match operation {
                        CompareOp::Less => number_array_filter_lt,
                        CompareOp::LessEqual => number_array_filter_lte,
                        CompareOp::Greater => number_array_filter_gt,
                        CompareOp::GreaterEqual => number_array_filter_gte,
                        CompareOp::Equal => number_array_filter_eq,
                        CompareOp::NotEqual => number_array_filter_ne,
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayTruthy(kind, mode) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(kind * 8 + mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_truthy as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayTruthy(mode) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        dynamic_array_truthy as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::PrimitiveArrayCompare(kind, operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(kind * 64 + mode * 8 + *operation as u8)
                            .to_bits()
                            .to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        primitive_array_compare as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::DynamicArrayCompare(operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(mode * 8 + operation).to_bits().to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        dynamic_array_compare as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayMap(kind, operation) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(kind * 8 + operation).to_bits().to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_map as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::PrimitiveArrayConvert(source, target) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(source * 4 + target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_convert as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayConvert(target) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        dynamic_array_convert as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayMapIdentity => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        dynamic_array_map_identity as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayJitMap(target, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            dynamic_array_jit_map_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            dynamic_array_jit_map as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::DynamicArrayJitScan(mode, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            dynamic_array_jit_scan_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            dynamic_array_jit_scan as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::DynamicArrayJitReduce(has_initial, from_right, captured) => {
                    if *captured {
                        if depth < 4 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            if !*has_initial && *from_right {
                                dynamic_array_jit_reduce_right_unseeded_captured
                            } else if !*has_initial {
                                dynamic_array_jit_reduce_left_unseeded_captured
                            } else if *from_right {
                                dynamic_array_jit_reduce_right_captured
                            } else {
                                dynamic_array_jit_reduce_left_captured
                            } as *const () as u64,
                            depth - 4,
                        );
                        depth -= 3;
                    } else {
                        if depth < 3 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            if !*has_initial && *from_right {
                                dynamic_array_jit_reduce_right_unseeded
                            } else if !*has_initial {
                                dynamic_array_jit_reduce_left_unseeded
                            } else if *from_right {
                                dynamic_array_jit_reduce_right
                            } else {
                                dynamic_array_jit_reduce_left
                            } as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    }
                }
                NumericValue::NumberArrayMap(operation, reverse) => {
                    if depth < 2 {
                        return None;
                    }
                    let functions = match operation {
                        NumericReduceOp::Add => {
                            [number_array_map_add, number_array_map_add_reverse]
                        }
                        NumericReduceOp::Subtract => {
                            [number_array_map_subtract, number_array_map_subtract_reverse]
                        }
                        NumericReduceOp::Multiply => {
                            [number_array_map_multiply, number_array_map_multiply_reverse]
                        }
                        NumericReduceOp::Divide => {
                            [number_array_map_divide, number_array_map_divide_reverse]
                        }
                        NumericReduceOp::Remainder => [
                            number_array_map_remainder,
                            number_array_map_remainder_reverse,
                        ],
                        NumericReduceOp::Power => {
                            [number_array_map_power, number_array_map_power_reverse]
                        }
                        NumericReduceOp::Minimum | NumericReduceOp::Maximum => return None,
                    };
                    emit_binary_call(
                        &mut code,
                        functions[usize::from(*reverse)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayJitMap(source, target, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(source * 4 + target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            primitive_array_jit_map_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            primitive_array_jit_map as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::PrimitiveArrayJitScan(kind, mode, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(kind * 8 + mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            primitive_array_jit_scan_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            primitive_array_jit_scan as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::NumberArrayIndexMap(operation, reverse) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let operation = match operation {
                        NumericReduceOp::Add => 0,
                        NumericReduceOp::Subtract => 1,
                        NumericReduceOp::Multiply => 2,
                        NumericReduceOp::Divide => 3,
                        NumericReduceOp::Remainder => 4,
                        NumericReduceOp::Power => 5,
                        NumericReduceOp::Minimum | NumericReduceOp::Maximum => return None,
                    } + if *reverse { 8 } else { 0 };
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(operation).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        number_array_index_map as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::NumberArraySelectMap(operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(*operation as u8 + mode * 8)
                            .to_bits()
                            .to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        number_array_select_map as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayBranchMap(encoded) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*encoded).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        number_array_branch_map as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayUnaryMap(absolute) => {
                    if depth == 0 {
                        return None;
                    }
                    let function = if *absolute {
                        number_array_map_absolute
                    } else {
                        number_array_map_negate
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayMathMap(operation) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*operation as u8).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        number_array_map_math as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::ArraySlice => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, array_slice as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::DynamicArraySlice => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(
                        &mut code,
                        dynamic_array_slice as *const () as u64,
                        depth - 3,
                    );
                    depth -= 2;
                }
                NumericValue::ArrayConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, array_concat as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DynamicArrayConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(
                        &mut code,
                        dynamic_array_concat as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayAppend
                | NumericValue::StringArrayAppend
                | NumericValue::BoolArrayAppend
                | NumericValue::DynamicArrayAppend => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayAppend => number_array_append,
                        NumericValue::StringArrayAppend => string_array_append,
                        NumericValue::BoolArrayAppend => bool_array_append,
                        NumericValue::DynamicArrayAppend => dynamic_array_append,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::ArrayToReversed => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_to_reversed as *const () as u64, depth - 1);
                }
                NumericValue::ArrayReverse => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_reverse as *const () as u64, depth - 1);
                }
                NumericValue::DynamicArrayToReversed | NumericValue::DynamicArrayReverse => {
                    if depth == 0 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayReverse {
                        dynamic_array_reverse_in_place
                    } else {
                        dynamic_array_to_reversed
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayToSorted
                | NumericValue::StringArrayToSorted
                | NumericValue::BoolArrayToSorted
                | NumericValue::DynamicArrayToSorted => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayToSorted => number_array_to_sorted,
                        NumericValue::StringArrayToSorted => string_array_to_sorted,
                        NumericValue::BoolArrayToSorted => bool_array_to_sorted,
                        NumericValue::DynamicArrayToSorted => dynamic_array_to_sorted,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArraySort
                | NumericValue::StringArraySort
                | NumericValue::BoolArraySort
                | NumericValue::DynamicArraySort => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArraySort => number_array_sort,
                        NumericValue::StringArraySort => string_array_sort,
                        NumericValue::BoolArraySort => bool_array_sort,
                        NumericValue::DynamicArraySort => dynamic_array_sort_in_place,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayToSortedBy(descending)
                | NumericValue::NumberArraySortBy(descending) => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match (value, descending) {
                        (NumericValue::NumberArrayToSortedBy(_), false) => {
                            number_array_to_sorted_ascending
                        }
                        (NumericValue::NumberArrayToSortedBy(_), true) => {
                            number_array_to_sorted_descending
                        }
                        (NumericValue::NumberArraySortBy(_), false) => number_array_sort_ascending,
                        (NumericValue::NumberArraySortBy(_), true) => number_array_sort_descending,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringArrayToSortedDescending
                | NumericValue::StringArraySortDescending => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringArrayToSortedDescending => {
                            string_array_to_sorted_descending
                        }
                        NumericValue::StringArraySortDescending => string_array_sort_descending,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayFill
                | NumericValue::StringArrayFill
                | NumericValue::BoolArrayFill
                | NumericValue::DynamicArrayFill => {
                    if depth < 4 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayFill => number_array_fill,
                        NumericValue::StringArrayFill => string_array_fill,
                        NumericValue::BoolArrayFill => bool_array_fill,
                        NumericValue::DynamicArrayFill => dynamic_array_fill,
                        _ => unreachable!(),
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArrayCopyWithin | NumericValue::DynamicArrayCopyWithin => {
                    if depth < 4 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayCopyWithin {
                        dynamic_array_copy_within
                    } else {
                        array_copy_within
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArraySplice => {
                    if depth < 4 {
                        return None;
                    }
                    emit_quaternary_call(&mut code, array_splice as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArrayToSpliced => {
                    if depth < 4 {
                        return None;
                    }
                    emit_quaternary_call(
                        &mut code,
                        array_to_spliced as *const () as u64,
                        depth - 4,
                    );
                    depth -= 3;
                }
                NumericValue::DynamicArraySplice | NumericValue::DynamicArrayToSpliced => {
                    if depth < 4 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayToSpliced {
                        dynamic_array_to_spliced
                    } else {
                        dynamic_array_splice_in_place
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::NumberArrayPush
                | NumericValue::StringArrayPush
                | NumericValue::BoolArrayPush
                | NumericValue::DynamicArrayPush => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayPush => number_array_push,
                        NumericValue::StringArrayPush => string_array_push,
                        NumericValue::BoolArrayPush => bool_array_push,
                        NumericValue::DynamicArrayPush => dynamic_array_push,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayUnshift
                | NumericValue::StringArrayUnshift
                | NumericValue::BoolArrayUnshift
                | NumericValue::DynamicArrayUnshift => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayUnshift => number_array_unshift,
                        NumericValue::StringArrayUnshift => string_array_unshift,
                        NumericValue::BoolArrayUnshift => bool_array_unshift,
                        NumericValue::DynamicArrayUnshift => dynamic_array_unshift,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArraySet
                | NumericValue::StringArraySet
                | NumericValue::BoolArraySet
                | NumericValue::DynamicArraySet => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArraySet => number_array_set,
                        NumericValue::StringArraySet => string_array_set,
                        NumericValue::BoolArraySet => bool_array_set,
                        NumericValue::DynamicArraySet => dynamic_array_set,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::AggregateLocalSet(kind, index) => {
                    if depth < 2 || *index >= depth - 2 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_array_set,
                        1 => bool_array_set,
                        2 => string_array_set,
                        3 => number_dictionary_set,
                        4 => bool_dictionary_set,
                        5 => string_dictionary_set,
                        _ => return None,
                    };
                    let destination = depth - 2;
                    emit_spill(&mut code, destination);
                    emit_move(&mut code, 0, *index);
                    emit_move(&mut code, 1, destination);
                    emit_move(&mut code, 2, destination + 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, destination, 0);
                    emit_restore(&mut code, destination);
                    depth -= 1;
                }
                NumericValue::AggregateLocalArrayInsert(kind, index, unshift) => {
                    if depth == 0 || *index >= depth - 1 {
                        return None;
                    }
                    let function = match (kind, unshift) {
                        (0, false) => number_array_push,
                        (1, false) => bool_array_push,
                        (2, false) => string_array_push,
                        (0, true) => number_array_unshift,
                        (1, true) => bool_array_unshift,
                        (2, true) => string_array_unshift,
                        _ => return None,
                    };
                    let destination = depth - 1;
                    emit_spill(&mut code, destination);
                    emit_move(&mut code, 0, *index);
                    emit_move(&mut code, 1, destination);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, destination, 0);
                    emit_restore(&mut code, destination);
                }
                NumericValue::NumberArrayPostSet => {
                    if depth < 4 {
                        return None;
                    }
                    let left = depth - 4;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, left);
                    emit_move(&mut code, 1, left + 1);
                    emit_move(&mut code, 2, left + 3);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(number_array_set as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    emit_move(&mut code, left, left + 2);
                    depth -= 3;
                }
                NumericValue::ArrayValue => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_value as *const () as u64, depth - 1);
                }
                NumericValue::MutableArrayHandle => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        mutable_array_handle as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::EmptyArray => {
                    if depth > 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(empty_array as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::NumberArrayPop
                | NumericValue::StringArrayPop
                | NumericValue::BoolArrayPop
                | NumericValue::DynamicArrayPop
                | NumericValue::NumberArrayShift
                | NumericValue::StringArrayShift
                | NumericValue::BoolArrayShift
                | NumericValue::DynamicArrayShift => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayPop => number_array_pop,
                        NumericValue::StringArrayPop => string_array_pop,
                        NumericValue::BoolArrayPop => bool_array_pop,
                        NumericValue::DynamicArrayPop => dynamic_array_pop,
                        NumericValue::NumberArrayShift => number_array_shift,
                        NumericValue::StringArrayShift => string_array_shift,
                        NumericValue::BoolArrayShift => bool_array_shift,
                        NumericValue::DynamicArrayShift => dynamic_array_shift,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::Drop => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                NumericValue::DropUnder => {
                    if depth < 2 {
                        return None;
                    }
                    emit_move(&mut code, depth - 2, depth - 1);
                    depth -= 1;
                }
                NumericValue::Duplicate => {
                    if !(1..=7).contains(&depth) {
                        return None;
                    }
                    emit_move(&mut code, depth, depth - 1);
                    depth += 1;
                }
                NumericValue::DuplicatePair => {
                    if !(2..=6).contains(&depth) {
                        return None;
                    }
                    emit_move(&mut code, depth, depth - 2);
                    emit_move(&mut code, depth + 1, depth - 1);
                    depth += 2;
                }
                NumericValue::LocalGet(index) => {
                    if *index >= depth || depth == 8 {
                        return None;
                    }
                    emit_move(&mut code, depth, *index);
                    depth += 1;
                }
                NumericValue::LocalSet(index) => {
                    if depth == 0 || *index >= depth - 1 {
                        return None;
                    }
                    emit_move(&mut code, *index, depth - 1);
                    depth -= 1;
                }
                NumericValue::LoopStart => loops.push(LoopPatch {
                    start: code.len(),
                    continue_target: None,
                    base_depth: depth,
                    condition_exits: Vec::new(),
                    continues: Vec::new(),
                    breaks: Vec::new(),
                }),
                NumericValue::LoopWhile => {
                    let loop_patch = loops.last_mut()?;
                    if depth != loop_patch.base_depth + 1 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    loop_patch
                        .condition_exits
                        .extend([parity, emit_near_jump(&mut code, 0x84)]);
                    depth -= 1;
                }
                NumericValue::LoopContinuePoint => {
                    let loop_patch = loops.last_mut()?;
                    if depth != loop_patch.base_depth || loop_patch.continue_target.is_some() {
                        return None;
                    }
                    for jump in loop_patch.continues.drain(..) {
                        patch_near_jump(&mut code, jump)?;
                    }
                    loop_patch.continue_target = Some(code.len());
                }
                NumericValue::LoopBreak(target_depth) => {
                    let target = loops
                        .len()
                        .checked_sub(usize::from(*target_depth).checked_add(1)?)?;
                    let loop_patch = loops.get_mut(target)?;
                    if depth < loop_patch.base_depth {
                        return None;
                    }
                    loop_patch.breaks.push(emit_unconditional_jump(&mut code));
                }
                NumericValue::LoopContinue(target_depth) => {
                    let target = loops
                        .len()
                        .checked_sub(usize::from(*target_depth).checked_add(1)?)?;
                    let loop_patch = loops.get_mut(target)?;
                    if depth < loop_patch.base_depth {
                        return None;
                    }
                    if let Some(target) = loop_patch.continue_target {
                        emit_backward_jump(&mut code, target)?;
                    } else {
                        loop_patch
                            .continues
                            .push(emit_unconditional_jump(&mut code));
                    }
                }
                NumericValue::LoopEnd => {
                    let loop_patch = loops.pop()?;
                    if depth != loop_patch.base_depth
                        || loop_patch.continue_target.is_none()
                        || !loop_patch.continues.is_empty()
                    {
                        return None;
                    }
                    emit_backward_jump(&mut code, loop_patch.start)?;
                    for exit in loop_patch
                        .condition_exits
                        .into_iter()
                        .chain(loop_patch.breaks)
                    {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::GuardStart => {
                    if depth == 0 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    let zero = emit_near_jump(&mut code, 0x84);
                    depth -= 1;
                    guards.push((depth, vec![parity, zero], Vec::new(), false));
                }
                NumericValue::GuardAlternate => {
                    let (base_depth, false_exits, end_exits, has_alternate) = guards.last_mut()?;
                    if *has_alternate || depth != *base_depth {
                        return None;
                    }
                    end_exits.push(emit_unconditional_jump(&mut code));
                    for exit in false_exits.drain(..) {
                        patch_near_jump(&mut code, exit)?;
                    }
                    *has_alternate = true;
                }
                NumericValue::GuardEnd => {
                    let (base_depth, false_exits, end_exits, _) = guards.pop()?;
                    if depth != base_depth {
                        return None;
                    }
                    for exit in false_exits.into_iter().chain(end_exits) {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::SwitchStart => {
                    if depth == 0 {
                        return None;
                    }
                    switches.push(SwitchPatch {
                        base_depth: depth,
                        next_case: Vec::new(),
                        fallthrough: None,
                        breaks: Vec::new(),
                        has_case: false,
                        has_default: false,
                        default_body: None,
                    });
                }
                NumericValue::SwitchCaseStart => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth {
                        return None;
                    }
                    if switch.has_case {
                        switch.fallthrough = Some(emit_unconditional_jump(&mut code));
                    }
                    for next in switch.next_case.drain(..) {
                        patch_near_jump(&mut code, next)?;
                    }
                    switch.has_case = true;
                }
                NumericValue::SwitchCaseBody => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth + 1 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    let mismatch = emit_near_jump(&mut code, 0x84);
                    switch.next_case.extend([parity, mismatch]);
                    if let Some(fallthrough) = switch.fallthrough.take() {
                        patch_near_jump(&mut code, fallthrough)?;
                    }
                    depth -= 1;
                }
                NumericValue::SwitchDefault => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth || switch.has_default {
                        return None;
                    }
                    let fallthrough = switch.has_case.then(|| emit_unconditional_jump(&mut code));
                    for mismatch in switch.next_case.drain(..) {
                        patch_near_jump(&mut code, mismatch)?;
                    }
                    switch.next_case.push(emit_unconditional_jump(&mut code));
                    let body = code.len();
                    if let Some(fallthrough) = fallthrough {
                        patch_near_jump(&mut code, fallthrough)?;
                    }
                    switch.has_default = true;
                    switch.default_body = Some(body);
                }
                NumericValue::SwitchBreak => {
                    let switch = switches.last_mut()?;
                    if depth < switch.base_depth {
                        return None;
                    }
                    switch.breaks.push(emit_unconditional_jump(&mut code));
                }
                NumericValue::SwitchEnd => {
                    let switch = switches.pop()?;
                    if depth != switch.base_depth {
                        return None;
                    }
                    let unmatched = switch.default_body.unwrap_or(code.len());
                    for exit in switch.next_case {
                        patch_jump_to(&mut code, exit, unmatched)?;
                    }
                    for exit in switch.breaks {
                        patch_near_jump(&mut code, exit)?;
                    }
                    depth -= 1;
                }
                NumericValue::TryStart | NumericValue::TaggedTryStart => tries.push(TryPatch {
                    base_depth: depth,
                    throws: Vec::new(),
                    tagged: matches!(value, NumericValue::TaggedTryStart),
                }),
                NumericValue::Throw => {
                    let exception = tries.last_mut()?;
                    if exception.tagged || depth <= exception.base_depth {
                        return None;
                    }
                    emit_move(&mut code, exception.base_depth, depth - 1);
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    depth -= 1;
                }
                NumericValue::TaggedThrow(tag) => {
                    let exception = tries.last_mut()?;
                    if !exception.tagged
                        || depth <= exception.base_depth
                        || exception.base_depth >= 7
                    {
                        return None;
                    }
                    emit_move(&mut code, exception.base_depth + 1, depth - 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*tag).to_bits().to_le_bytes());
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x6e,
                        0xc0 | (exception.base_depth << 3),
                    ]);
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    depth -= 1;
                }
                NumericValue::CheckError => {
                    let exception = tries.last_mut()?;
                    if exception.tagged && exception.base_depth >= 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(take_call_error as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0, 0x66, 0x48, 0x0f, 0x7e, 0xc0]);
                    emit_restore(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0x85, 0xc0]);
                    let no_error = emit_near_jump(&mut code, 0x84);
                    let value_slot = exception.base_depth + u8::from(exception.tagged);
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (value_slot << 3)]);
                    if exception.tagged {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&2.0_f64.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | (exception.base_depth << 3),
                        ]);
                    }
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    patch_near_jump(&mut code, no_error)?;
                }
                NumericValue::UncaughtNumberThrow
                | NumericValue::UncaughtBooleanThrow
                | NumericValue::UncaughtStringThrow
                | NumericValue::UncaughtNumberArrayThrow
                | NumericValue::UncaughtBooleanArrayThrow
                | NumericValue::UncaughtStringArrayThrow
                | NumericValue::UncaughtDictionaryThrow => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::UncaughtNumberThrow => uncaught_number_throw,
                        NumericValue::UncaughtBooleanThrow => uncaught_boolean_throw,
                        NumericValue::UncaughtStringThrow => uncaught_string_throw,
                        NumericValue::UncaughtNumberArrayThrow => uncaught_number_array_throw,
                        NumericValue::UncaughtBooleanArrayThrow => uncaught_boolean_array_throw,
                        NumericValue::UncaughtStringArrayThrow => uncaught_string_array_throw,
                        NumericValue::UncaughtDictionaryThrow => uncaught_dictionary_throw,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                    code.push(0xc3);
                    depth -= 1;
                }
                NumericValue::CatchStart => {
                    let exception = tries.pop()?;
                    if depth < exception.base_depth || exception.throws.is_empty() {
                        return None;
                    }
                    let result_depth = depth - exception.base_depth;
                    let normal_exit = emit_unconditional_jump(&mut code);
                    for jump in exception.throws {
                        patch_near_jump(&mut code, jump)?;
                    }
                    catches.push((exception.base_depth, normal_exit, result_depth));
                    depth = exception.base_depth + if exception.tagged { 2 } else { 1 };
                }
                NumericValue::TryEnd => {
                    let (base_depth, normal_exit, result_depth) = catches.pop()?;
                    if depth != base_depth + result_depth {
                        return None;
                    }
                    patch_near_jump(&mut code, normal_exit)?;
                }
                NumericValue::ResultStart => results.push(ResultPatch {
                    base_depth: depth,
                    result_depth: None,
                    exits: Vec::new(),
                }),
                NumericValue::ResultReturn(count) => {
                    let continuation_depth = loops
                        .last()
                        .map(|loop_patch| loop_patch.base_depth)
                        .into_iter()
                        .chain(guards.last().map(|guard| guard.0))
                        .chain(switches.last().map(|switch| switch.base_depth))
                        .max();
                    let result = results.last_mut()?;
                    if depth <= result.base_depth {
                        return None;
                    }
                    let result_depth = if *count == 0 {
                        depth - result.base_depth
                    } else {
                        *count
                    };
                    if depth < result.base_depth + result_depth {
                        return None;
                    }
                    if result
                        .result_depth
                        .replace(result_depth)
                        .is_some_and(|expected| expected != result_depth)
                    {
                        return None;
                    }
                    let source = depth - result_depth;
                    for offset in 0..result_depth {
                        emit_move(&mut code, result.base_depth + offset, source + offset);
                    }
                    result.exits.push(emit_unconditional_jump(&mut code));
                    depth = continuation_depth
                        .unwrap_or(result.base_depth)
                        .max(result.base_depth);
                }
                NumericValue::ResultEnd => {
                    let result = results.pop()?;
                    if depth != result.base_depth + result.result_depth? {
                        return None;
                    }
                    for exit in result.exits {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::EarlyReturn => {
                    let loop_patch = loops.last()?;
                    if depth <= loop_patch.base_depth {
                        return None;
                    }
                    emit_move(&mut code, 0, depth - 1);
                    code.push(0xc3);
                    depth -= 1;
                }
                NumericValue::MathRandom
                | NumericValue::DateNow
                | NumericValue::PerformanceNow
                | NumericValue::ProcessPid
                | NumericValue::ProcessPpid
                | NumericValue::MissingCallable
                | NumericValue::Absent
                | NumericValue::Null
                | NumericValue::PreserveAbsent => {
                    if depth > 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    let function = match value {
                        NumericValue::MathRandom => math_random,
                        NumericValue::DateNow => date_now,
                        NumericValue::PerformanceNow => performance_now,
                        NumericValue::ProcessPid => process_pid,
                        NumericValue::ProcessPpid => process_ppid,
                        NumericValue::MissingCallable => missing_callable,
                        NumericValue::Absent => absent_value,
                        NumericValue::Null => null_value,
                        NumericValue::PreserveAbsent => preserve_absent_value,
                        _ => unreachable!(),
                    };
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::NumberArrayWith
                | NumericValue::StringArrayWith
                | NumericValue::BoolArrayWith
                | NumericValue::DynamicArrayWith => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayWith => number_array_with,
                        NumericValue::StringArrayWith => string_array_with,
                        NumericValue::BoolArrayWith => bool_array_with,
                        NumericValue::DynamicArrayWith => dynamic_array_with,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringToLowerCase
                | NumericValue::StringToUpperCase
                | NumericValue::StringTrim
                | NumericValue::StringTrimStart
                | NumericValue::StringTrimEnd => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringToLowerCase => string_to_lower_case,
                        NumericValue::StringToUpperCase => string_to_upper_case,
                        NumericValue::StringTrim => string_trim,
                        NumericValue::StringTrimStart => string_trim_start,
                        NumericValue::StringTrimEnd => string_trim_end,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_length as *const () as u64, depth - 1);
                }
                NumericValue::ArrayLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_length as *const () as u64, depth - 1);
                }
                NumericValue::DynamicArrayLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        dynamic_array_length as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::IsArray | NumericValue::IsNotArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        if *value == NumericValue::IsArray {
                            is_array
                        } else {
                            is_not_array
                        } as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicIsArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dynamic_is_array as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayAt
                | NumericValue::BoolArrayAt
                | NumericValue::StringArrayAt
                | NumericValue::DynamicArrayAt
                | NumericValue::NumberArrayGet
                | NumericValue::BoolArrayGet
                | NumericValue::StringArrayGet => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayAt => number_array_at,
                        NumericValue::BoolArrayAt => bool_array_at,
                        NumericValue::StringArrayAt => string_array_at,
                        NumericValue::DynamicArrayAt => dynamic_array_at,
                        NumericValue::NumberArrayGet => number_array_get,
                        NumericValue::BoolArrayGet => bool_array_get,
                        NumericValue::StringArrayGet => string_array_get,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberDictionaryGet
                | NumericValue::BoolDictionaryGet
                | NumericValue::StringDictionaryGet
                | NumericValue::DictionaryDelete
                | NumericValue::DictionaryHasOwn
                | NumericValue::DictionaryIn
                | NumericValue::DictionaryAssign => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberDictionaryGet => number_dictionary_get,
                        NumericValue::BoolDictionaryGet => bool_dictionary_get,
                        NumericValue::StringDictionaryGet => string_dictionary_get,
                        NumericValue::DictionaryDelete => dictionary_delete,
                        NumericValue::DictionaryHasOwn => dictionary_has_own,
                        NumericValue::DictionaryIn => dictionary_in,
                        NumericValue::DictionaryAssign => dictionary_assign,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DictionaryKeys
                | NumericValue::NumberDictionaryValues
                | NumericValue::BoolDictionaryValues
                | NumericValue::StringDictionaryValues
                | NumericValue::NumberDictionaryEntries
                | NumericValue::BoolDictionaryEntries
                | NumericValue::StringDictionaryEntries
                | NumericValue::DictionaryFromNumberEntries
                | NumericValue::DictionaryFromBoolEntries
                | NumericValue::DictionaryFromStringEntries => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::DictionaryKeys => dictionary_keys,
                        NumericValue::NumberDictionaryValues => number_dictionary_values,
                        NumericValue::BoolDictionaryValues => bool_dictionary_values,
                        NumericValue::StringDictionaryValues => string_dictionary_values,
                        NumericValue::NumberDictionaryEntries => number_dictionary_entries,
                        NumericValue::BoolDictionaryEntries => bool_dictionary_entries,
                        NumericValue::StringDictionaryEntries => string_dictionary_entries,
                        NumericValue::DictionaryFromNumberEntries => dictionary_from_number_entries,
                        NumericValue::DictionaryFromBoolEntries => dictionary_from_bool_entries,
                        NumericValue::DictionaryFromStringEntries => dictionary_from_string_entries,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberDictionarySet
                | NumericValue::StringDictionarySet
                | NumericValue::BoolDictionarySet => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberDictionarySet => number_dictionary_set,
                        NumericValue::StringDictionarySet => string_dictionary_set,
                        NumericValue::BoolDictionarySet => bool_dictionary_set,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::EmptyDictionary => {
                    if depth == 8 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(empty_dictionary as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::DictionaryLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dictionary_length as *const () as u64, depth - 1);
                }
                NumericValue::DictionaryKeyAt => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, dictionary_key_at as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DictionaryAppend(kind) => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_dictionary_set,
                        1 => bool_dictionary_set,
                        2 => string_dictionary_set,
                        _ => return None,
                    };
                    let object = depth - 3;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, object);
                    emit_move(&mut code, 1, object + 1);
                    emit_move(&mut code, 2, object + 2);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    depth -= 2;
                }
                NumericValue::DictionaryStaticAppend(kind, key) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_dictionary_set,
                        1 => bool_dictionary_set,
                        2 => string_dictionary_set,
                        _ => return None,
                    };
                    let object = depth - 2;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, object);
                    emit_move(&mut code, 2, object + 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(*key as usize as u64).to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc8]);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    depth -= 1;
                }
                NumericValue::NumberDictionaryPostSet => {
                    if depth < 4 {
                        return None;
                    }
                    let left = depth - 4;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, left);
                    emit_move(&mut code, 1, left + 1);
                    emit_move(&mut code, 2, left + 3);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &(number_dictionary_set as *const () as u64).to_le_bytes(),
                    );
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    emit_move(&mut code, left, left + 2);
                    depth -= 3;
                }
                NumericValue::NumberArrayJoin
                | NumericValue::BoolArrayJoin
                | NumericValue::StringArrayJoin
                | NumericValue::DynamicArrayJoin => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayJoin => number_array_join,
                        NumericValue::BoolArrayJoin => bool_array_join,
                        NumericValue::StringArrayJoin => string_array_join,
                        NumericValue::DynamicArrayJoin => dynamic_array_join,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayIncludes
                | NumericValue::BoolArrayIncludes
                | NumericValue::StringArrayIncludes
                | NumericValue::DynamicArrayIncludes
                | NumericValue::NumberArrayIndexOf
                | NumericValue::BoolArrayIndexOf
                | NumericValue::StringArrayIndexOf
                | NumericValue::DynamicArrayIndexOf
                | NumericValue::NumberArrayLastIndexOf
                | NumericValue::BoolArrayLastIndexOf
                | NumericValue::StringArrayLastIndexOf
                | NumericValue::DynamicArrayLastIndexOf => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayIncludes => number_array_includes,
                        NumericValue::BoolArrayIncludes => bool_array_includes,
                        NumericValue::StringArrayIncludes => string_array_includes,
                        NumericValue::DynamicArrayIncludes => dynamic_array_includes,
                        NumericValue::NumberArrayIndexOf => number_array_index_of,
                        NumericValue::BoolArrayIndexOf => bool_array_index_of,
                        NumericValue::StringArrayIndexOf => string_array_index_of,
                        NumericValue::DynamicArrayIndexOf => dynamic_array_index_of,
                        NumericValue::NumberArrayLastIndexOf => number_array_last_index_of,
                        NumericValue::BoolArrayLastIndexOf => bool_array_last_index_of,
                        NumericValue::StringArrayLastIndexOf => string_array_last_index_of,
                        NumericValue::DynamicArrayLastIndexOf => dynamic_array_last_index_of,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringTruthy => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_truthy as *const () as u64, depth - 1);
                }
                NumericValue::StringIsWellFormed => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        string_is_well_formed as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::StringToWellFormed => {
                    if depth == 0 {
                        return None;
                    }
                }
                NumericValue::UnaryMath(operation) => {
                    if depth == 0 {
                        return None;
                    }
                    if *operation == UnaryMath::SquareRoot {
                        let register = depth - 1;
                        code.extend_from_slice(&[
                            0xf2,
                            0x0f,
                            0x51,
                            0xc0 | (register << 3) | register,
                        ]);
                    } else {
                        emit_unary_call(
                            &mut code,
                            operation.function() as *const () as u64,
                            depth - 1,
                        );
                    }
                }
                NumericValue::Select => {
                    if depth < 3 {
                        return None;
                    }
                    let condition = depth - 3;
                    let consequent = depth - 2;
                    let alternate = depth - 1;
                    // JavaScript ToBoolean treats NaN and both signed zeroes as false.
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                        0x7a,
                        0x13,
                    ]);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                        0x74,
                        0x06,
                    ]);
                    emit_move(&mut code, condition, consequent);
                    code.extend_from_slice(&[0xeb, 0x04]);
                    emit_move(&mut code, condition, alternate);
                    depth -= 2;
                }
                NumericValue::ShortCircuit(and) => {
                    if depth < 2 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    let truth = emit_near_jump(&mut code, if *and { 0x84 } else { 0x85 });
                    let mut exits = vec![truth];
                    if *and {
                        exits.push(parity);
                    } else {
                        patch_near_jump(&mut code, parity)?;
                    }
                    depth -= 2;
                    branches.push((depth, exits, false, Some(1)));
                }
                NumericValue::ConditionalStart => {
                    if depth == 0 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    let zero = emit_near_jump(&mut code, 0x84);
                    depth -= 1;
                    branches.push((depth, vec![parity, zero], true, None));
                }
                NumericValue::PresentConditionalStart => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(take_present as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    code.extend_from_slice(&[0x66, 0x0f, 0x2e, 0xc0 | (depth << 3) | depth]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (depth << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    let absent = emit_near_jump(&mut code, 0x84);
                    branches.push((depth - 1, vec![parity, absent], true, None));
                }
                NumericValue::ConditionalAlternate => {
                    let (base_depth, exits, awaits_alternate, result_depth) =
                        branches.last_mut()?;
                    if !*awaits_alternate || depth <= *base_depth {
                        return None;
                    }
                    *result_depth = Some(depth - *base_depth);
                    let end = emit_unconditional_jump(&mut code);
                    for exit in exits.drain(..) {
                        patch_near_jump(&mut code, exit)?;
                    }
                    exits.push(end);
                    *awaits_alternate = false;
                    depth = *base_depth;
                }
                NumericValue::ShortCircuitEnd => {
                    let (base_depth, exits, awaits_alternate, result_depth) = branches.pop()?;
                    if awaits_alternate || depth != base_depth + result_depth? {
                        return None;
                    }
                    for exit in exits {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::AsBoolean => {
                    if depth == 0 {
                        return None;
                    }
                }
                NumericValue::BooleanNot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, boolean_not as *const () as u64, depth - 1);
                }
                NumericValue::StrictMismatch(result) => {
                    if depth < 2 {
                        return None;
                    }
                    let destination = depth - 2;
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(u8::from(*result)).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (destination << 3)]);
                    depth -= 1;
                }
                NumericValue::Recur(arity) => {
                    if depth < *arity {
                        return None;
                    }
                    let base = depth - arity;
                    emit_recursive_call(&mut code, base, *arity)?;
                    depth = base + 1;
                }
            }
        }
        (depth == 1
            && branches.is_empty()
            && loops.is_empty()
            && guards.is_empty()
            && switches.is_empty()
            && results.is_empty())
        .then(|| {
            code.push(0xc3);
            code
        })
    }

    fn required_args(&self) -> usize {
        self.0
            .iter()
            .filter_map(|value| match value {
                NumericValue::Argument(index) => Some(*index as usize + 1),
                NumericValue::DynamicArgument(index) => Some(*index as usize + 2),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
}
