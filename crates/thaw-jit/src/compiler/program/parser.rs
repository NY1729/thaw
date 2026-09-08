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

}
