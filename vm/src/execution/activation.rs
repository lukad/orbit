use orbit_compiler::bytecode::{Count, Register};

use crate::{
    error::{FaultResult, VmError, VmErrorKind, VmTraceFrame},
    function::NativeInvocation,
    id::ObjectId,
    native::{NativeCallback, NativeEventData, NativeToken},
    value::RawValue,
};

use super::frame::CallFrame;

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenExtent {
    pub(super) base: usize,
    pub(super) top: usize,
}

struct CloseOperation {
    base: Register,
    cause: RawValue,
    completion: CloseCompletion,
}

pub(super) enum CloseCompletion {
    Resume,
    ReturnOwned(Box<[RawValue]>),
    Unwind(VmError),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ResultTarget {
    Call { base: usize, results: Count },
    GenericFor { start: usize, variables: usize },
    Index { destination: Register },
    Operator { destination: Register },
    Comparison { destination: Register },
    NewIndex,
    Close,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ReturnTarget {
    Lua(ResultTarget),
    Native {
        token: NativeToken,
        results: NativeResultMode,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum NativeResultMode {
    All,
    First,
    Boolean,
    None,
}

impl NativeResultMode {
    pub(crate) fn normalize(self, values: Box<[RawValue]>) -> Box<[RawValue]> {
        match self {
            Self::All => values,
            Self::First => {
                vec![values.first().cloned().unwrap_or(RawValue::Nil)].into_boxed_slice()
            }
            Self::Boolean => {
                let result = values.first().is_some_and(RawValue::is_truthy);
                vec![RawValue::Boolean(result)].into_boxed_slice()
            }
            Self::None => Box::default(),
        }
    }
}

pub(crate) enum FrameBoundary {
    /// Defers operand collection so a direct Lua callee can copy from the
    /// caller's register window without allocating an argument buffer.
    Call {
        base: Register,
        arguments: Count,
        results: Count,
    },
    Invoke {
        callee: RawValue,
        arguments: Box<[RawValue]>,
        target: ResultTarget,
    },
    TailInvoke {
        callee: RawValue,
        arguments: Vec<RawValue>,
    },
    Return {
        base: Register,
        values: Count,
    },
    /// Cleanup returns snapshot their values before closing captured locals.
    ReturnOwned {
        values: Box<[RawValue]>,
    },
    UnwindOwned {
        error: VmError,
    },
}

pub(crate) enum Activation {
    Lua(LuaActivation),
    Native(NativeActivation),
}

impl Activation {
    pub(crate) fn return_to(&self) -> Option<ReturnTarget> {
        match self {
            Self::Lua(activation) => activation.return_to(),
            Self::Native(activation) => activation.return_to(),
        }
    }

    pub(crate) fn trace_frame(&self) -> VmTraceFrame {
        match self {
            Self::Lua(activation) => activation.frame().trace_frame(),
            Self::Native(activation) => VmTraceFrame::Native {
                name: activation.name().into(),
            },
        }
    }

    pub(crate) fn visit_roots(&self, visit: impl FnMut(ObjectId)) {
        match self {
            Self::Lua(activation) => {
                activation.visit_roots(visit);
            }
            Self::Native(activation) => {
                activation.visit_roots(visit);
            }
        }
    }

    pub(crate) fn as_lua(&self) -> Option<&LuaActivation> {
        match self {
            Self::Lua(activation) => Some(activation),
            Self::Native(_) => None,
        }
    }

    pub(crate) fn as_lua_mut(&mut self) -> Option<&mut LuaActivation> {
        match self {
            Self::Lua(activation) => Some(activation),
            Self::Native(_) => None,
        }
    }

    pub(crate) fn as_native_mut(&mut self) -> Option<&mut NativeActivation> {
        match self {
            Self::Native(activation) => Some(activation),
            Self::Lua(_) => None,
        }
    }

    pub(crate) fn is_native(&self) -> bool {
        matches!(self, Self::Native(_))
    }
}

pub(crate) struct LuaActivation {
    frame: CallFrame,
    return_to: Option<ReturnTarget>,
    close: Option<CloseOperation>,
}

impl LuaActivation {
    pub(crate) fn entry(frame: CallFrame) -> Self {
        Self {
            frame,
            return_to: None,
            close: None,
        }
    }

    pub(crate) fn called(frame: CallFrame, return_to: ReturnTarget) -> Self {
        Self {
            frame,
            return_to: Some(return_to),
            close: None,
        }
    }

    pub(crate) fn frame(&self) -> &CallFrame {
        &self.frame
    }

    pub(crate) fn frame_mut(&mut self) -> &mut CallFrame {
        &mut self.frame
    }

    pub(crate) fn return_to(&self) -> Option<ReturnTarget> {
        self.return_to
    }

    pub(crate) fn into_frame(self) -> CallFrame {
        assert!(
            self.close.is_none(),
            "recycling an activation during cleanup"
        );
        self.frame
    }

    pub(crate) fn visit_roots(&self, mut visit: impl FnMut(ObjectId)) {
        self.frame.visit_roots(&mut visit);

        let Some(operation) = &self.close else {
            return;
        };

        if let Some(object) = operation.cause.object_id() {
            visit(object);
        }

        if let CloseCompletion::ReturnOwned(values) = &operation.completion {
            for value in values {
                if let Some(object) = value.object_id() {
                    visit(object);
                }
            }
        }
    }

    pub(super) fn begin_close(
        &mut self,
        base: Register,
        cause: RawValue,
        completion: CloseCompletion,
    ) {
        assert!(self.close.is_none(), "activation is already closing");

        self.close = Some(CloseOperation {
            base,
            cause,
            completion,
        });
    }

    pub(super) fn is_closing(&self) -> bool {
        self.close.is_some()
    }

    pub(super) fn next_to_close(&mut self) -> Option<(Register, RawValue)> {
        let operation = self.close.as_ref()?;
        let register = self.frame.pop_to_close_from(operation.base)?;

        Some((register, operation.cause.clone()))
    }

    pub(super) fn finish_close(&mut self) -> CloseCompletion {
        self.close
            .take()
            .expect("activation has no active close operation")
            .completion
    }

    pub(super) fn replace_close_error(&mut self, cause: RawValue, error: VmError) {
        let operation = self
            .close
            .as_mut()
            .expect("activation has no active close operation");

        operation.cause = cause;
        operation.completion = CloseCompletion::Unwind(error);
    }
}

pub(crate) struct NativeActivation {
    invocation: NativeInvocation,
    arguments: Box<[RawValue]>,
    state: NativeState,
    return_to: Option<ReturnTarget>,
}

impl NativeActivation {
    pub(crate) fn entry(invocation: NativeInvocation, arguments: Box<[RawValue]>) -> Self {
        Self {
            invocation,
            arguments,
            state: NativeState::Start,
            return_to: None,
        }
    }

    pub(crate) fn called(
        invocation: NativeInvocation,
        arguments: Box<[RawValue]>,
        return_to: ReturnTarget,
    ) -> Self {
        Self {
            invocation,
            arguments,
            state: NativeState::Start,
            return_to: Some(return_to),
        }
    }

    pub(crate) fn name(&self) -> &str {
        self.invocation.name()
    }

    pub(crate) fn callback(&self) -> NativeCallback {
        self.invocation.callback()
    }

    pub(crate) fn arguments(&self) -> &[RawValue] {
        &self.arguments
    }

    pub(crate) fn captures(&self) -> &[RawValue] {
        self.invocation.captures()
    }

    pub(crate) fn return_to(&self) -> Option<ReturnTarget> {
        self.return_to
    }

    pub(crate) fn begin_invocation(&mut self) -> FaultResult<NativeOwnedEvent> {
        let state = std::mem::replace(&mut self.state, NativeState::Running);

        match state {
            NativeState::Start => Ok(NativeOwnedEvent::Start),
            NativeState::Resume {
                token,
                values,
                continuation,
            } => Ok(NativeOwnedEvent::Resume {
                token,
                values,
                continuation,
            }),
            NativeState::ResumeError {
                token,
                error,
                continuation,
            } => Ok(NativeOwnedEvent::ResumeError {
                token,
                error,
                continuation,
            }),
            NativeState::WaitingForAction { .. } => {
                self.state = state;
                Err(invalid_continuation(
                    "native callback is still waiting for an asynchronous action",
                ))
            }
            NativeState::WaitingForHost { .. } => {
                self.state = state;
                Err(invalid_continuation(
                    "yielded native callback must be resumed by its host",
                ))
            }
            NativeState::Running => Err(invalid_continuation("native callback is already running")),
        }
    }

    pub(crate) fn wait_for_action(
        &mut self,
        token: NativeToken,
        continuation: Box<[RawValue]>,
    ) -> FaultResult<()> {
        if !matches!(self.state, NativeState::Running) {
            return Err(invalid_continuation(
                "native callback requested an action while it was not running",
            ));
        }

        self.state = NativeState::WaitingForAction {
            token,
            continuation,
        };

        Ok(())
    }

    pub(crate) fn resume_from_action(
        &mut self,
        token: NativeToken,
        values: Box<[RawValue]>,
    ) -> FaultResult<()> {
        let state = std::mem::replace(&mut self.state, NativeState::Running);

        match state {
            NativeState::WaitingForAction {
                token: expected,
                continuation,
            } if expected == token => {
                self.state = NativeState::Resume {
                    token,
                    values,
                    continuation,
                };
                Ok(())
            }
            state => {
                self.state = state;
                Err(invalid_continuation(
                    "asynchronous action returned to the wrong native continuation",
                ))
            }
        }
    }

    pub(crate) fn resume_error_from_action(
        &mut self,
        token: NativeToken,
        error: VmError,
    ) -> FaultResult<()> {
        let state = std::mem::replace(&mut self.state, NativeState::Running);

        match state {
            NativeState::WaitingForAction {
                token: expected,
                continuation,
            } if expected == token => {
                self.state = NativeState::ResumeError {
                    token,
                    error,
                    continuation,
                };
                Ok(())
            }
            state => {
                self.state = state;
                Err(invalid_continuation(
                    "asynchronous action error returned to the wrong native continuation",
                ))
            }
        }
    }

    pub(crate) fn wait_for_host(&mut self, token: NativeToken) -> FaultResult<()> {
        if !matches!(self.state, NativeState::Running) {
            return Err(invalid_continuation(
                "native callback yielded while it was not running",
            ));
        }

        self.state = NativeState::WaitingForHost { token };

        Ok(())
    }

    pub(crate) fn resume_from_host(&mut self, values: Box<[RawValue]>) -> FaultResult<()> {
        let NativeState::WaitingForHost { token } = self.state else {
            return Err(invalid_continuation(
                "execution is not waiting for host values",
            ));
        };

        self.state = NativeState::Resume {
            token,
            values,
            continuation: Box::default(),
        };

        Ok(())
    }

    pub(crate) fn resume_error_from_host(&mut self, error: VmError) -> FaultResult<()> {
        let NativeState::WaitingForHost { token } = self.state else {
            return Err(invalid_continuation(
                "execution is not waiting for a host error",
            ));
        };

        self.state = NativeState::ResumeError {
            token,
            error,
            continuation: Box::default(),
        };

        Ok(())
    }

    pub(crate) fn visit_roots(&self, mut visit: impl FnMut(ObjectId)) {
        for value in self.arguments.iter().chain(self.invocation.captures()) {
            if let Some(object) = value.object_id() {
                visit(object);
            }
        }

        let (values, continuation) = match &self.state {
            NativeState::WaitingForAction { continuation, .. }
            | NativeState::ResumeError { continuation, .. } => (&[][..], continuation.as_ref()),
            NativeState::Resume {
                values,
                continuation,
                ..
            } => (values.as_ref(), continuation.as_ref()),
            NativeState::Start | NativeState::Running | NativeState::WaitingForHost { .. } => {
                (&[][..], &[][..])
            }
        };

        for value in values.iter().chain(continuation) {
            if let Some(object) = value.object_id() {
                visit(object);
            }
        }
    }
}

pub(crate) enum NativeOwnedEvent {
    Start,
    Resume {
        token: NativeToken,
        values: Box<[RawValue]>,
        continuation: Box<[RawValue]>,
    },
    ResumeError {
        token: NativeToken,
        error: VmError,
        continuation: Box<[RawValue]>,
    },
}

impl NativeOwnedEvent {
    pub(crate) fn as_data(&self) -> NativeEventData<'_> {
        match self {
            Self::Start => NativeEventData::Start,
            Self::Resume {
                token,
                values,
                continuation,
            } => NativeEventData::Resume {
                token: *token,
                values,
                continuation,
            },
            Self::ResumeError {
                token,
                error,
                continuation,
            } => NativeEventData::ResumeError {
                token: *token,
                error,
                continuation,
            },
        }
    }
}

enum NativeState {
    Start,
    Running,
    WaitingForAction {
        token: NativeToken,
        continuation: Box<[RawValue]>,
    },
    WaitingForHost {
        token: NativeToken,
    },
    Resume {
        token: NativeToken,
        values: Box<[RawValue]>,
        continuation: Box<[RawValue]>,
    },
    ResumeError {
        token: NativeToken,
        error: VmError,
        continuation: Box<[RawValue]>,
    },
}

fn invalid_continuation(message: &'static str) -> VmErrorKind {
    VmErrorKind::InvalidNativeContinuation { message }
}
