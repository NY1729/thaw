//! Error types. Messages are V8's text without the `WebAssembly.X(): ` prefix;
//! the embedder adds the prefix that matches the API it was called through.

use std::fmt;

/// Module bytes rejected. Node prints this as `CompileError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
}

impl CompileError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CompileError {}

/// Import resolution failed. `type_error` is true where Node throws a
/// `TypeError` (the import module is missing) and false for `LinkError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError {
    pub type_error: bool,
    pub message: String,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LinkError {}

/// A wasm trap, classified the way V8 reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trap {
    Unreachable,
    MemoryOutOfBounds,
    TableOutOfBounds,
    /// call_indirect hit a null entry or a signature mismatch (V8 merges both).
    IndirectCallMismatch,
    DivideByZero,
    /// i32/i64 division overflow (MIN / -1).
    DivideOverflow,
    FloatUnrepresentable,
    /// Node raises this one as a `RangeError`, not a `RuntimeError`.
    StackOverflow,
    /// Anything wasmi reports that has no V8 counterpart: the wasmi text.
    Other(String),
}

impl Trap {
    pub fn message(&self) -> &str {
        match self {
            Trap::Unreachable => "unreachable",
            Trap::MemoryOutOfBounds => "memory access out of bounds",
            Trap::TableOutOfBounds => "table index is out of bounds",
            Trap::IndirectCallMismatch => "null function or function signature mismatch",
            Trap::DivideByZero => "divide by zero",
            Trap::DivideOverflow => "divide result unrepresentable",
            Trap::FloatUnrepresentable => "float unrepresentable in integer range",
            Trap::StackOverflow => "Maximum call stack size exceeded",
            Trap::Other(text) => text,
        }
    }

    pub fn is_range_error(&self) -> bool {
        matches!(self, Trap::StackOverflow)
    }
}

/// Failure of a call into wasm (or of a start function).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    Trap(Trap),
    /// A host callback returned `Err(id)`; the embedder rethrows its own value `id`.
    Exception(u32),
    /// WASI `proc_exit(code)`.
    Exit(i32),
    /// Bad argument count or type from the embedder.
    Argument(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Trap(trap) => f.write_str(trap.message()),
            CallError::Exception(id) => write!(f, "host exception {id}"),
            CallError::Exit(code) => write!(f, "proc_exit({code})"),
            CallError::Argument(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CallError {}

/// Error from `Store::instantiate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstantiateError {
    Link(LinkError),
    /// The start function trapped or threw.
    Call(CallError),
}

impl fmt::Display for InstantiateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstantiateError::Link(error) => error.fmt(f),
            InstantiateError::Call(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for InstantiateError {}
