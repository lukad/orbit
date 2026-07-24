use std::borrow::Cow;

use orbit_common::Span;

use crate::{Value, loading::LoadError};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VmErrorKind {
    #[error("program counter out of bounds: {pc}")]
    ProgramCounterOutOfBounds { pc: usize },
    #[error("invalid register: {register}")]
    InvalidRegister { register: u8 },
    #[error("invalid jump offset: {offset}")]
    InvalidJump { offset: i32 },
    #[error("invalid constant index: {constant}")]
    InvalidConstant { constant: u32 },
    #[error("invalid string index: {string}")]
    InvalidString { string: u32 },
    #[error("attempt to get the length of a {kind} value")]
    InvalidLengthOperand { kind: &'static str },
    #[error("string length does not fit in a Lua integer: {length}")]
    StringTooLong { length: usize },
    #[error("attempt to negate a {kind} value")]
    InvalidNegateOperand { kind: &'static str },
    #[error("attempt to perform bitwise not on a {kind} value")]
    InvalidBitwiseOperand { kind: &'static str },
    #[error("attempt to add a {left} value and a {right} value")]
    InvalidAddOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to subtract a {right} value from a {left} value")]
    InvalidSubtractOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to multiply a {left} value by a {right} value")]
    InvalidMultiplyOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to divide a {left} value by a {right} value")]
    InvalidDivideOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to floor-divide a {left} value by a {right} value")]
    InvalidFloorDivideOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to divide an integer by zero")]
    IntegerDivisionByZero,
    #[error("attempt to calculate modulo of a {left} value by a {right} value")]
    InvalidModuloOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to calculate integer modulo by zero")]
    IntegerModuloByZero,
    #[error("attempt to raise a {left} value to a {right} value")]
    InvalidPowerOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to perform {operation} on a {left} value and a {right} value")]
    InvalidBitwiseOperands {
        operation: &'static str,
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to concatenate a {left} value and a {right} value")]
    InvalidConcatOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to compare a {left} value with a {right} value using {operation}")]
    InvalidComparisonOperands {
        operation: &'static str,
        left: &'static str,
        right: &'static str,
    },
    #[error("register R{base} cannot be offset by {offset}")]
    InvalidRegisterOffset { base: u8, offset: u8 },
    #[error("numeric for-loop step cannot be zero")]
    ZeroForStep,
    #[error("numeric for-loop control values must be numbers")]
    InvalidForControl,
    #[error("attempt to index a {kind} value")]
    InvalidTableOperand { kind: &'static str },
    #[error("'{metamethod}' chain too long; possible loop")]
    MetamethodChainTooLong { metamethod: &'static str },
    #[error("table index is nil")]
    NilTableKey,
    #[error("table index is NaN")]
    NaNTableKey,
    #[error("SetList first index must be at least one, got {first_index}")]
    InvalidListIndex { first_index: u32 },
    #[error("no open result extent is available")]
    MissingOpenResultExtent,
    #[error("invalid upvalue index: {upvalue}")]
    InvalidUpvalue { upvalue: u32 },
    #[error("invalid child prototype index: {child}")]
    InvalidChildPrototype { child: u32 },
    #[error("entry upvalue {upvalue} tries to capture a parent frame")]
    InvalidEntryUpvalue { upvalue: usize },
    #[error("child prototype {child} upvalue {upvalue} directly captures the external environment")]
    InvalidChildExternalEnvironment { child: u32, upvalue: usize },
    #[error("to-be-closed locals require __close metamethod support")]
    UnsupportedToBeClosedLocal,
    #[error("attempt to call a {kind} value")]
    InvalidCallOperand { kind: &'static str },
    #[error("invalid register range: start {start}, count {count}")]
    InvalidRegisterRange { start: usize, count: usize },
    #[error("prototype declares {parameters} parameters but only provides {registers} registers")]
    InvalidPrototypeRegisters { parameters: u8, registers: u16 },
    #[error("attempt to read varargs from a non-vararg function")]
    InvalidVarargAccess,
    #[error(
        "open results begin at register {result_base}, after requested register {requested_start}"
    )]
    InvalidOpenResultStart {
        requested_start: usize,
        result_base: usize,
    },
    #[error("generic for requires at least one visible variable")]
    InvalidGenericForVariableCount,
    #[error("native function failed: {message}")]
    NativeFunctionFailure { message: Box<str> },
    #[error("table capacity exceeds implementation limits: requested {requested}")]
    TableCapacityExceeded { requested: usize },
    #[error("invalid key passed to 'next'")]
    InvalidKeyToNext,
    #[error("heap capacity exceeds implementation limits: requested {requested}")]
    HeapCapacityExceeded { requested: usize },
    #[error("dangling heap object reference: slot {slot}, generation {generation}")]
    DanglingObject { slot: u32, generation: u32 },
    #[error("heap object has kind {actual}, expected {expected}")]
    WrongObjectKind {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("foreign {kind}: expected state {expected_state}, got state {actual_state}")]
    ForeignObject {
        kind: &'static str,
        expected_state: u64,
        actual_state: u64,
    },
    #[error("external-root capacity exceeded: requested {requested}")]
    RootCapacityExceeded { requested: usize },
    #[error("call-frame capacity exceeded: requested {requested}")]
    FrameCapacityExceeded { requested: usize },
    #[error("closure has {actual} upvalues, but its prototype declares {expected}")]
    InvalidClosureUpvalueCount { expected: usize, actual: usize },
    #[error("invalid native continuation state: {message}")]
    InvalidNativeContinuation { message: &'static str },
    #[error(transparent)]
    LoadFailure(LoadError),
    #[error("Lua error")]
    Raised,
}

impl From<LoadError> for VmErrorKind {
    fn from(error: LoadError) -> Self {
        Self::LoadFailure(error)
    }
}

pub(crate) type FaultResult<T> = Result<T, VmErrorKind>;

/// Describes how a Lua frame should be identified in a traceback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaTraceFunction {
    MainChunk,
    Named(Box<str>),
    Anonymous,
}

impl std::fmt::Display for LuaTraceFunction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MainChunk => formatter.write_str("in main chunk"),
            Self::Named(name) => write!(formatter, "in function '{name}'"),
            Self::Anonymous => formatter.write_str("in anonymous function"),
        }
    }
}

/// One frame in a runtime traceback, ordered from the innermost frame outward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmTraceFrame {
    Lua {
        function: LuaTraceFunction,
        function_span: Span,
        pc: usize,
        instruction_span: Option<Span>,
    },
    Native {
        name: Box<str>,
    },
}

/// A runtime failure together with the Lua/native frames active when it occurred.
#[derive(Debug, Clone, PartialEq)]
pub struct VmError {
    pub kind: VmErrorKind,
    pub frames: Box<[VmTraceFrame]>,
    object: Option<Value>,
    level: i64,
}

impl VmError {
    pub fn new(kind: VmErrorKind) -> Self {
        Self {
            kind,
            frames: Box::new([]),
            object: None,
            level: 0,
        }
    }

    pub fn raised(object: Value, level: i64) -> Self {
        Self {
            kind: VmErrorKind::Raised,
            frames: Box::new([]),
            object: Some(object),
            level,
        }
    }

    pub(crate) fn with_frames(kind: VmErrorKind, frames: Box<[VmTraceFrame]>) -> Self {
        Self {
            kind,
            frames,
            object: None,
            level: 0,
        }
    }

    pub(crate) fn append_frames(&mut self, frames: impl IntoIterator<Item = VmTraceFrame>) {
        let mut combined = Vec::from(std::mem::take(&mut self.frames));
        combined.extend(frames);
        self.frames = combined.into_boxed_slice();
    }

    pub fn object(&self) -> Option<&Value> {
        self.object.as_ref()
    }

    pub fn message(&self) -> Cow<'_, str> {
        match (&self.kind, self.object.as_ref().unwrap_or(&Value::Nil)) {
            (VmErrorKind::Raised, Value::String(message)) => {
                String::from_utf8_lossy(message.as_bytes())
            }
            (VmErrorKind::Raised, object) => {
                Cow::Owned(format!("(error object is a {} value)", object.type_name()))
            }
            (kind, _) => Cow::Owned(kind.to_string()),
        }
    }
}

impl From<VmErrorKind> for VmError {
    fn from(kind: VmErrorKind) -> Self {
        Self::new(kind)
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.kind)?;

        if self.frames.is_empty() {
            return Ok(());
        }

        write!(formatter, "\nstack traceback:")?;

        for frame in &self.frames {
            match frame {
                VmTraceFrame::Lua {
                    function,
                    function_span,
                    pc,
                    instruction_span,
                } => {
                    let span = instruction_span.unwrap_or(*function_span);
                    write!(
                        formatter,
                        "\n\t[source {} bytes {}..{}, pc {}]: {function}",
                        span.source.get(),
                        span.start,
                        span.end,
                        pc
                    )?;
                }
                VmTraceFrame::Native { name } => {
                    write!(formatter, "\n\t[C]: in function '{name}'")?;
                }
            }
        }

        Ok(())
    }
}

impl std::error::Error for VmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

pub type VmResult<T> = Result<T, VmError>;
