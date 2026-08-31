use super::*;

/// A native function signature, used by `HirExpr::FfiCall`. Built today
/// from ambient `declare function` statements (see docs/design/bridge.md);
/// thaw-bridge will eventually also generate these from `.d.ts` files for
/// whole npm packages, via the same type-classification rules.
#[derive(Debug, Clone, PartialEq)]
pub enum FfiErrorAbi {
    Direct,
    ThawResult,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FfiOwnership {
    Borrowed,
    Owned { destroy: Symbol },
    ArenaCopy { destroy: Option<Symbol> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiStringAbi {
    NullTerminated,
    PointerLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiCallingConvention {
    C,
    Fast,
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiVariadicAbi {
    Native,
    I32,
    I64,
    U32,
    U64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiAggregateAbi {
    Internal,
    Portable,
    Packed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiBitFieldLayout {
    pub bit_offset: u8,
    pub bit_width: u8,
    pub storage_bytes: u8,
    pub signed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiRegisterClass {
    Integer,
    Sse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiAggregateLayout {
    pub field_offsets: Vec<u64>,
    pub field_layouts: Vec<Option<Box<FfiAggregateLayout>>>,
    pub field_bitfields: Vec<Option<FfiBitFieldLayout>>,
    pub register_classes: Vec<FfiRegisterClass>,
    pub size: u64,
    pub alignment: u32,
    pub indirect: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FfiSignature {
    pub symbol: Symbol,
    pub params: Vec<HirType>,
    /// Element type of a trailing C varargs sequence. Lowering accepts
    /// TypeScript rest declarations with supported scalar or aggregate
    /// element layouts.
    pub variadic: Option<HirType>,
    pub variadic_abi: FfiVariadicAbi,
    pub ret: HirType,
    pub error_abi: FfiErrorAbi,
    pub return_ownership: FfiOwnership,
    pub error_ownership: FfiOwnership,
    pub param_string_abis: Vec<FfiStringAbi>,
    pub return_string_abi: FfiStringAbi,
    pub calling_convention: FfiCallingConvention,
    pub aggregate_return_abi: FfiAggregateAbi,
    pub aggregate_return_layout: Option<FfiAggregateLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicBackend {
    Jit,
    QuickJs,
    Napi,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicSignature {
    pub backend: DynamicBackend,
    pub symbol: Symbol,
    pub params: Vec<HirType>,
    pub ret: HirType,
}
