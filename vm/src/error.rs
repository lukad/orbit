use std::borrow::Cow;

use orbit_common::Span;

use crate::{LuaString, Value, loading::LoadError};

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
    #[error("variable '{name}' got a non-closeable value")]
    NonClosableValue { name: Box<str> },
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
    #[error("to-be-closed register R{register} was marked after R{previous}")]
    InvalidToCloseOrder { previous: u8, register: u8 },
}

impl From<LoadError> for VmErrorKind {
    fn from(error: LoadError) -> Self {
        Self::LoadFailure(error)
    }
}

impl VmError {
    pub fn object_or_message(&self) -> Value {
        self.object
            .clone()
            .unwrap_or_else(|| Value::String(LuaString::from(self.kind.to_string().into_bytes())))
    }
}

pub(crate) type FaultResult<T> = Result<T, VmErrorKind>;

const TRACEBACK_HEAD_LEVELS: usize = 10;
const TRACEBACK_TAIL_LEVELS: usize = 11;

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
    omitted_frames: usize,
    object: Option<Value>,
    level: i64,
}

impl VmError {
    pub fn new(kind: VmErrorKind) -> Self {
        Self {
            kind,
            frames: Box::new([]),
            omitted_frames: 0,
            object: None,
            level: 0,
        }
    }

    pub fn raised(object: Value, level: i64) -> Self {
        Self {
            kind: VmErrorKind::Raised,
            frames: Box::new([]),
            omitted_frames: 0,
            object: Some(object),
            level,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_frames(kind: VmErrorKind, frames: Box<[VmTraceFrame]>) -> Self {
        let mut error = Self::new(kind);
        error.append_trace(frames, 0);
        error
    }

    #[cfg(test)]
    pub(crate) fn append_frames(&mut self, frames: impl IntoIterator<Item = VmTraceFrame>) {
        self.append_trace(frames.into_iter().collect::<Vec<_>>().into_boxed_slice(), 0);
    }

    pub(crate) fn append_trace(&mut self, frames: Box<[VmTraceFrame]>, omitted: usize) {
        let existing = std::mem::take(&mut self.frames);
        let existing_omitted = self.omitted_frames;
        let existing_len = existing.len().saturating_add(existing_omitted);
        let appended_len = frames.len().saturating_add(omitted);
        let total_len = existing_len.saturating_add(appended_len);

        if total_len <= TRACEBACK_HEAD_LEVELS + TRACEBACK_TAIL_LEVELS {
            debug_assert_eq!(existing_omitted, 0);
            debug_assert_eq!(omitted, 0);

            let mut combined = Vec::from(existing);
            combined.extend(frames);
            self.frames = combined.into_boxed_slice();
            self.omitted_frames = 0;
            return;
        }

        let mut retained = Vec::with_capacity(TRACEBACK_HEAD_LEVELS + TRACEBACK_TAIL_LEVELS);

        let existing_head = existing_len.min(TRACEBACK_HEAD_LEVELS);
        retained.extend(existing.iter().take(existing_head).cloned());

        if existing_head < TRACEBACK_HEAD_LEVELS {
            retained.extend(
                frames
                    .iter()
                    .take(TRACEBACK_HEAD_LEVELS - existing_head)
                    .cloned(),
            );
        }

        let appended_tail = appended_len.min(TRACEBACK_TAIL_LEVELS);
        let existing_tail = TRACEBACK_TAIL_LEVELS - appended_tail;

        if existing_tail > 0 {
            retained.extend(
                existing
                    .iter()
                    .skip(existing.len().saturating_sub(existing_tail))
                    .cloned(),
            );
        }

        retained.extend(
            frames
                .iter()
                .skip(frames.len().saturating_sub(appended_tail))
                .cloned(),
        );

        self.omitted_frames = total_len.saturating_sub(retained.len());
        self.frames = retained.into_boxed_slice();
    }

    /// Splits a traceback into the leading and trailing frames that should be
    /// displayed, returning the number of frames omitted between them.
    ///
    /// Short tracebacks are returned entirely in the leading section with an
    /// empty trailing section and an omitted count of zero.
    pub fn traceback_sections(&self) -> (&[VmTraceFrame], usize, &[VmTraceFrame]) {
        if self.omitted_frames == 0 {
            return (&self.frames, 0, &[]);
        }

        (
            &self.frames[..TRACEBACK_HEAD_LEVELS],
            self.omitted_frames,
            &self.frames[TRACEBACK_HEAD_LEVELS..],
        )
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

        let (head, skipped, tail) = self.traceback_sections();

        for frame in head {
            write_trace_frame(formatter, frame)?;
        }

        if skipped > 0 {
            write!(formatter, "\n\t...\t(skipping {skipped} levels)")?;
        }

        for frame in tail {
            write_trace_frame(formatter, frame)?;
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

fn write_trace_frame(
    formatter: &mut std::fmt::Formatter<'_>,
    frame: &VmTraceFrame,
) -> std::fmt::Result {
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
            )
        }
        VmTraceFrame::Native { name } => {
            write!(formatter, "\n\t[C]: in function '{name}'")
        }
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::{SourceId, Span};

    use super::{LuaTraceFunction, VmError, VmErrorKind, VmTraceFrame};

    fn lua_frames(count: usize) -> Box<[VmTraceFrame]> {
        let span = Span::new(SourceId::new(0), 0, 1);

        (0..count)
            .map(|pc| VmTraceFrame::Lua {
                function: LuaTraceFunction::MainChunk,
                function_span: span,
                pc,
                instruction_span: Some(span),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }

    #[test]
    fn traceback_sections_keep_short_traces_whole() {
        let error = VmError::with_frames(
            VmErrorKind::ProgramCounterOutOfBounds { pc: 0 },
            lua_frames(21),
        );

        let (head, skipped, tail) = error.traceback_sections();

        assert_eq!(head, error.frames.as_ref());
        assert_eq!(skipped, 0);
        assert!(tail.is_empty());
    }

    #[test]
    fn traceback_sections_keep_ten_head_and_eleven_tail_levels() {
        let error = VmError::with_frames(
            VmErrorKind::ProgramCounterOutOfBounds { pc: 0 },
            lua_frames(30),
        );

        let (head, skipped, tail) = error.traceback_sections();

        assert_eq!(head, &error.frames[..10]);
        assert_eq!(skipped, 9);
        assert_eq!(tail, &error.frames[10..]);

        let rendered = error.to_string();
        assert!(rendered.contains("\n\t...\t(skipping 9 levels)"));
        assert_eq!(rendered.matches("\n\t[source").count(), 21);
    }

    #[test]
    fn appending_to_a_condensed_trace_preserves_global_head_and_tail() {
        let mut error = VmError::with_frames(
            VmErrorKind::ProgramCounterOutOfBounds { pc: 0 },
            lua_frames(15),
        );
        error.append_frames(
            lua_frames(16)
                .into_vec()
                .into_iter()
                .map(|frame| match frame {
                    VmTraceFrame::Lua {
                        function,
                        function_span,
                        pc,
                        instruction_span,
                    } => VmTraceFrame::Lua {
                        function,
                        function_span,
                        pc: pc + 15,
                        instruction_span,
                    },
                    VmTraceFrame::Native { .. } => unreachable!(),
                }),
        );

        let (head, skipped, tail) = error.traceback_sections();
        let pcs = |frames: &[VmTraceFrame]| {
            frames
                .iter()
                .map(|frame| match frame {
                    VmTraceFrame::Lua { pc, .. } => *pc,
                    VmTraceFrame::Native { .. } => unreachable!(),
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(pcs(head), (0..10).collect::<Vec<_>>());
        assert_eq!(skipped, 10);
        assert_eq!(pcs(tail), (20..31).collect::<Vec<_>>());
    }
}
