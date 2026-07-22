mod activation;
mod call;
mod control;
mod data;
mod dispatch;
mod frame;
mod loops;
mod native;
mod operators;
mod tables;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod native_tests;

use crate::{
    error::{FaultResult, VmError, VmErrorKind, VmResult, VmTraceFrame},
    function::FunctionSnapshot,
    id::ObjectId,
    runtime::Runtime,
    value::{RawValue, Value},
};

pub(crate) use activation::{
    Activation, FrameBoundary, LuaActivation, NativeActivation, NativeResultMode, ResultTarget,
    ReturnTarget,
};

pub(crate) use frame::{CallFrame, offset_register};

use self::native::NativeStep;

pub(crate) struct Execution<'runtime> {
    runtime: &'runtime mut Runtime,
    stack: Vec<Activation>,
}

impl<'runtime> Execution<'runtime> {
    fn returned(self, values: Box<[RawValue]>) -> ExecutionOutcome<'runtime> {
        ExecutionOutcome::Returned {
            values,
            runtime: self.runtime,
        }
    }
}

pub(crate) enum ExecutionOutcome<'runtime> {
    Returned {
        values: Box<[RawValue]>,
        runtime: &'runtime mut Runtime,
    },
    Yielded {
        values: Box<[RawValue]>,
        suspension: SuspendedExecution<'runtime>,
    },
}

pub(crate) struct SuspendedExecution<'runtime> {
    execution: Execution<'runtime>,
}

impl<'runtime> SuspendedExecution<'runtime> {
    pub(crate) fn resume(
        mut self,
        values: Box<[RawValue]>,
    ) -> VmResult<ExecutionOutcome<'runtime>> {
        let result = match self
            .execution
            .stack
            .last_mut()
            .and_then(Activation::as_native_mut)
        {
            Some(activation) => activation.resume_from_host(values),
            None => {
                return Err(self
                    .execution
                    .runtime_error(VmErrorKind::InvalidNativeContinuation {
                        message: "suspended execution has no native activation",
                    }));
            }
        };

        if let Err(kind) = result {
            return Err(self.execution.runtime_error(kind));
        }

        self.execution.run()
    }

    pub(crate) fn resume_error(mut self, error: VmError) -> VmResult<ExecutionOutcome<'runtime>> {
        let result = match self
            .execution
            .stack
            .last_mut()
            .and_then(Activation::as_native_mut)
        {
            Some(activation) => activation.resume_error_from_host(error),
            None => {
                return Err(self
                    .execution
                    .runtime_error(VmErrorKind::InvalidNativeContinuation {
                        message: "suspended execution has no native activation",
                    }));
            }
        };

        if let Err(kind) = result {
            return Err(self.execution.runtime_error(kind));
        }

        self.execution.run()
    }

    pub(crate) fn import_values(&self, values: &[Value]) -> FaultResult<Box<[RawValue]>> {
        self.execution.runtime.import_values(values.iter().cloned())
    }

    pub(crate) fn export_values(&mut self, values: &[RawValue]) -> FaultResult<Vec<Value>> {
        self.execution.runtime.export_values(values)
    }

    pub(crate) fn collect_garbage(&mut self) -> FaultResult<usize> {
        self.execution.collect_garbage()
    }
}

impl<'runtime> Execution<'runtime> {
    pub(crate) fn new(
        runtime: &'runtime mut Runtime,
        function: FunctionSnapshot,
        arguments: Box<[RawValue]>,
    ) -> FaultResult<Self> {
        let activation = match function {
            FunctionSnapshot::Lua(invocation) => Activation::Lua(LuaActivation::entry(
                CallFrame::new(invocation, &arguments)?,
            )),
            FunctionSnapshot::Native(invocation) => {
                Activation::Native(NativeActivation::entry(invocation, arguments))
            }
        };

        let mut stack = Vec::new();
        stack
            .try_reserve(1)
            .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested: 1 })?;

        stack.push(activation);

        Ok(Self { runtime, stack })
    }

    pub(crate) fn run(mut self) -> VmResult<ExecutionOutcome<'runtime>> {
        loop {
            if let Err(kind) = self.collect_if_due() {
                return Err(self.runtime_error(kind));
            }

            if self
                .stack
                .last()
                .expect("execution stack is not empty")
                .is_native()
            {
                match self.invoke_native_top()? {
                    NativeStep::Continue => {
                        continue;
                    }
                    NativeStep::Complete(values) => {
                        return Ok(self.returned(values));
                    }
                    NativeStep::Yield(values) => {
                        return Ok(ExecutionOutcome::Yielded {
                            values,
                            suspension: SuspendedExecution { execution: self },
                        });
                    }
                }
            }

            let boundary = match self.run_until_boundary() {
                Ok(boundary) => boundary,
                Err(kind) => match self.route_error(VmError::from(kind)) {
                    Ok(()) => continue,
                    Err(error) => {
                        return Err(error);
                    }
                },
            };

            match boundary {
                FrameBoundary::Invoke {
                    callee,
                    arguments,
                    target,
                } => {
                    if let Err(kind) =
                        self.push_callable(callee, arguments, ReturnTarget::Lua(target))
                    {
                        match self.route_error(VmError::from(kind)) {
                            Ok(()) => continue,
                            Err(error) => {
                                return Err(error);
                            }
                        }
                    }
                }

                FrameBoundary::Return { values } => {
                    let activation = self.stack.pop().expect("returning activation is active");

                    match self.deliver_return(activation.return_to(), values) {
                        Ok(Some(values)) => {
                            return Ok(self.returned(values));
                        }
                        Ok(None) => {}
                        Err(kind) => match self.route_error(VmError::from(kind)) {
                            Ok(()) => continue,
                            Err(error) => {
                                return Err(error);
                            }
                        },
                    }
                }
            }
        }
    }

    pub(crate) fn visit_roots(&self, mut visit: impl FnMut(ObjectId)) {
        for activation in &self.stack {
            activation.visit_roots(&mut visit);
        }
    }

    fn run_until_boundary(&mut self) -> FaultResult<FrameBoundary> {
        loop {
            self.collect_if_due()?;

            let instruction = self.active_lua_frame_mut().next_instruction()?;

            if let Some(boundary) = self.dispatch(instruction)? {
                return Ok(boundary);
            }
        }
    }

    fn push_callable(
        &mut self,
        callee: RawValue,
        arguments: Box<[RawValue]>,
        return_to: ReturnTarget,
    ) -> FaultResult<()> {
        let (function, arguments) = self.resolve_callable(callee, arguments)?;

        let snapshot = self.runtime.function_snapshot(function)?;

        let activation = match snapshot {
            FunctionSnapshot::Lua(invocation) => Activation::Lua(LuaActivation::called(
                CallFrame::new(invocation, &arguments)?,
                return_to,
            )),
            FunctionSnapshot::Native(invocation) => {
                Activation::Native(NativeActivation::called(invocation, arguments, return_to))
            }
        };

        let requested = self.stack.len().saturating_add(1);

        self.stack
            .try_reserve(1)
            .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested })?;

        self.stack.push(activation);

        Ok(())
    }

    fn deliver_return(
        &mut self,
        target: Option<ReturnTarget>,
        values: Box<[RawValue]>,
    ) -> FaultResult<Option<Box<[RawValue]>>> {
        let Some(target) = target else {
            debug_assert!(self.stack.is_empty());

            return Ok(Some(values));
        };

        match target {
            ReturnTarget::Lua(target) => {
                let runtime = &mut *self.runtime;

                self.stack
                    .last_mut()
                    .and_then(Activation::as_lua_mut)
                    .ok_or(VmErrorKind::InvalidNativeContinuation {
                        message: "Lua return target has no Lua caller",
                    })?
                    .frame_mut()
                    .accept_results(runtime, target, &values)?;
            }

            ReturnTarget::Native { token, results } => {
                let values = results.normalize(values);

                self.stack
                    .last_mut()
                    .and_then(Activation::as_native_mut)
                    .ok_or(VmErrorKind::InvalidNativeContinuation {
                        message: "native return target has no native caller",
                    })?
                    .resume_from_action(token, values)?;
            }
        }

        Ok(None)
    }

    fn route_error(&mut self, mut error: VmError) -> Result<(), VmError> {
        let boundary = self.stack.iter().rposition(|activation| {
            matches!(activation.return_to(), Some(ReturnTarget::Native { .. }))
        });

        let Some(boundary) = boundary else {
            error.append_frames(self.stack.iter().rev().map(Activation::trace_frame));

            return Err(error);
        };

        let token = match self.stack[boundary]
            .return_to()
            .expect("boundary has a return target")
        {
            ReturnTarget::Native { token, .. } => token,
            ReturnTarget::Lua(_) => {
                unreachable!("boundary search selected a native target")
            }
        };

        error.append_frames(
            self.stack[boundary..]
                .iter()
                .rev()
                .map(Activation::trace_frame),
        );

        self.stack.truncate(boundary);

        let result = self
            .stack
            .last_mut()
            .and_then(Activation::as_native_mut)
            .ok_or(VmErrorKind::InvalidNativeContinuation {
                message: "native error boundary has no native caller",
            })
            .and_then(|activation| activation.resume_error_from_action(token, error));

        if let Err(kind) = result {
            let mut invariant = VmError::from(kind);

            invariant.append_frames(self.stack.iter().rev().map(Activation::trace_frame));

            return Err(invariant);
        }

        Ok(())
    }

    fn active_lua_frame(&self) -> &CallFrame {
        self.stack
            .last()
            .and_then(Activation::as_lua)
            .expect("active activation is Lua")
            .frame()
    }

    fn active_lua_frame_mut(&mut self) -> &mut CallFrame {
        self.stack
            .last_mut()
            .and_then(Activation::as_lua_mut)
            .expect("active activation is Lua")
            .frame_mut()
    }

    fn read_register(&self, register: orbit_compiler::bytecode::Register) -> FaultResult<RawValue> {
        self.active_lua_frame()
            .get_register(&*self.runtime, register)
    }

    fn write_register(
        &mut self,
        register: orbit_compiler::bytecode::Register,
        value: RawValue,
    ) -> FaultResult<()> {
        let runtime = &mut *self.runtime;

        self.stack
            .last_mut()
            .and_then(Activation::as_lua_mut)
            .expect("active activation is Lua")
            .frame_mut()
            .set_register(runtime, register, value)
    }

    fn apply_jump(&mut self, offset: i32) -> FaultResult<()> {
        self.active_lua_frame_mut().apply_jump(offset)
    }

    fn trace_frames(&self) -> Box<[VmTraceFrame]> {
        self.stack
            .iter()
            .rev()
            .map(Activation::trace_frame)
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }

    fn runtime_error(&self, kind: VmErrorKind) -> VmError {
        VmError::with_frames(kind, self.trace_frames())
    }

    fn collect_if_due(&mut self) -> FaultResult<()> {
        if !self.runtime.collection_due() {
            return Ok(());
        }

        self.collect_garbage()?;

        Ok(())
    }

    fn collect_garbage(&mut self) -> FaultResult<usize> {
        let roots = self.root_snapshot()?;

        self.runtime.collect_garbage(&roots)
    }

    fn root_snapshot(&self) -> FaultResult<Box<[ObjectId]>> {
        let mut roots = Vec::new();
        let mut capacity_error = None;

        self.visit_roots(|root| {
            if capacity_error.is_some() {
                return;
            }

            let requested = roots.len().saturating_add(1);

            if roots.try_reserve(1).is_err() {
                capacity_error = Some(VmErrorKind::RootCapacityExceeded { requested });
            } else {
                roots.push(root);
            }
        });

        if let Some(error) = capacity_error {
            return Err(error);
        }

        Ok(roots.into_boxed_slice())
    }
}
