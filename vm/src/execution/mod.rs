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
    execution::activation::CloseCompletion,
    function::FunctionSnapshot,
    id::ObjectId,
    runtime::Runtime,
    value::{RawValue, Value},
};

pub(crate) use activation::{
    Activation, FrameBoundary, LuaActivation, NativeActivation, NativeResultMode, ResultTarget,
    ReturnTarget,
};

pub(crate) use frame::{CallFrame, CallFrameStorage, offset_register};
use orbit_compiler::bytecode::Register;

use self::native::NativeStep;

pub(crate) struct Execution<'runtime> {
    runtime: &'runtime mut Runtime,
    stack: Vec<Activation>,
    tail_arguments: Vec<RawValue>,
    // Shallow repeated calls can reuse their callee's register allocation.
    spare_lua_frame: Option<CallFrameStorage>,
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

        Ok(Self {
            runtime,
            stack,
            tail_arguments: Vec::new(),
            spare_lua_frame: None,
        })
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
                FrameBoundary::Call {
                    base,
                    arguments,
                    results,
                } => {
                    if let Err(kind) = self.push_instruction_call(base, arguments, results) {
                        match self.route_error(VmError::from(kind)) {
                            Ok(()) => continue,
                            Err(error) => {
                                return Err(error);
                            }
                        }
                    }
                }
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
                FrameBoundary::TailInvoke { callee, arguments } => {
                    if let Err(kind) = self.replace_callable(callee, arguments) {
                        match self.route_error(VmError::from(kind)) {
                            Ok(()) => continue,
                            Err(error) => {
                                return Err(error);
                            }
                        }
                    }
                }
                FrameBoundary::Return { base, values } => {
                    match self.return_from_lua(base, values) {
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
                FrameBoundary::ReturnOwned { values } => {
                    let activation = self.stack.pop().expect("returning activation is active");
                    let target = activation.return_to();
                    self.recycle_lua_activation(activation);

                    match self.deliver_return(target, values) {
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
                FrameBoundary::UnwindOwned { error } => {
                    let activation = self.stack.pop().expect("unwinding activation is active");
                    let target = activation.return_to();
                    self.recycle_lua_activation(activation);
                    self.forward_error(target, error)?;
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

            if let Some(boundary) = self.continue_close()? {
                return Ok(boundary);
            }

            let instruction = self.active_lua_frame_mut().next_instruction()?;

            if let Some(boundary) = self.dispatch(instruction)? {
                return Ok(boundary);
            }
        }
    }

    fn build_callable_activation(
        &mut self,
        callee: RawValue,
        arguments: Box<[RawValue]>,
        return_to: Option<ReturnTarget>,
    ) -> FaultResult<Activation> {
        let (function, arguments) = self.resolve_callable(callee, arguments)?;

        let snapshot = self.runtime.function_snapshot(function)?;

        match snapshot {
            FunctionSnapshot::Lua(invocation) => {
                let storage = self.spare_lua_frame.take().unwrap_or_default();
                let frame = CallFrame::new_reusing(invocation, &arguments, storage)?;
                Ok(Activation::Lua(match return_to {
                    Some(target) => LuaActivation::called(frame, target),
                    None => LuaActivation::entry(frame),
                }))
            }
            FunctionSnapshot::Native(invocation) => Ok(Activation::Native(match return_to {
                Some(target) => NativeActivation::called(invocation, arguments, target),
                None => NativeActivation::entry(invocation, arguments),
            })),
        }
    }

    fn push_instruction_call(
        &mut self,
        base: orbit_compiler::bytecode::Register,
        arguments: orbit_compiler::bytecode::Count,
        results: orbit_compiler::bytecode::Count,
    ) -> FaultResult<()> {
        let target = ResultTarget::Call {
            base: usize::from(base.0),
            results,
        };
        let (callee, argument_start, argument_count) = self
            .active_lua_frame()
            .call_register_range(&*self.runtime, base, arguments)?;

        if let RawValue::Function(function) = callee {
            match self.runtime.function_snapshot(function)? {
                FunctionSnapshot::Lua(invocation) => {
                    let storage = self.spare_lua_frame.take().unwrap_or_default();
                    let frame = CallFrame::new_from_frame(
                        invocation,
                        &*self.runtime,
                        self.active_lua_frame(),
                        argument_start,
                        argument_count,
                        storage,
                    )?;

                    self.active_lua_frame_mut()
                        .consume_open_call_arguments(arguments);

                    return self.push_activation(Activation::Lua(LuaActivation::called(
                        frame,
                        ReturnTarget::Lua(target),
                    )));
                }
                FunctionSnapshot::Native(invocation) => {
                    let (_, arguments) = {
                        let runtime = &*self.runtime;

                        self.stack
                            .last_mut()
                            .and_then(Activation::as_lua_mut)
                            .expect("active activation is Lua")
                            .frame_mut()
                            .collect_call(runtime, base, arguments)?
                    };

                    return self.push_activation(Activation::Native(NativeActivation::called(
                        invocation,
                        arguments,
                        ReturnTarget::Lua(target),
                    )));
                }
            }
        }

        let (callee, arguments) = {
            let runtime = &*self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .collect_call(runtime, base, arguments)?
        };

        self.push_callable(callee, arguments, ReturnTarget::Lua(target))
    }

    fn push_callable(
        &mut self,
        callee: RawValue,
        arguments: Box<[RawValue]>,
        return_to: ReturnTarget,
    ) -> FaultResult<()> {
        let activation = self.build_callable_activation(callee, arguments, Some(return_to))?;
        self.push_activation(activation)
    }

    fn push_activation(&mut self, activation: Activation) -> FaultResult<()> {
        let requested = self.stack.len().saturating_add(1);

        self.stack
            .try_reserve(1)
            .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested })?;

        self.stack.push(activation);

        Ok(())
    }

    fn replace_callable(
        &mut self,
        callee: RawValue,
        mut arguments: Vec<RawValue>,
    ) -> FaultResult<()> {
        let return_to = self
            .stack
            .last()
            .expect("tail caller is active")
            .return_to();

        let function = match self.resolve_callable_vec(callee, &mut arguments) {
            Ok(function) => function,
            Err(error) => {
                arguments.clear();
                self.tail_arguments = arguments;
                return Err(error);
            }
        };

        let is_same_lua_function = self
            .stack
            .last()
            .and_then(Activation::as_lua)
            .is_some_and(|activation| activation.frame().function() == function);

        if is_same_lua_function {
            let result = self
                .stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("tail caller is Lua")
                .frame_mut()
                .restart(&arguments);

            arguments.clear();
            self.tail_arguments = arguments;
            return result;
        }

        let snapshot = match self.runtime.function_snapshot(function) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                arguments.clear();
                self.tail_arguments = arguments;
                return Err(error);
            }
        };

        match snapshot {
            FunctionSnapshot::Lua(invocation) => {
                let active = self.stack.last_mut().expect("tail caller is active");

                debug_assert!(
                    active.as_lua().is_some(),
                    "only Lua activations execute TailCall"
                );

                let result = active
                    .as_lua_mut()
                    .expect("tail caller is Lua")
                    .frame_mut()
                    .replace(invocation, &arguments);

                arguments.clear();
                self.tail_arguments = arguments;
                result
            }
            FunctionSnapshot::Native(invocation) => {
                let arguments = arguments.into_boxed_slice();
                let active = self.stack.last_mut().expect("tail caller is active");
                *active = Activation::Native(match return_to {
                    Some(target) => NativeActivation::called(invocation, arguments, target),
                    None => NativeActivation::entry(invocation, arguments),
                });
                Ok(())
            }
        }
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

    fn return_from_lua(
        &mut self,
        base: orbit_compiler::bytecode::Register,
        values: orbit_compiler::bytecode::Count,
    ) -> FaultResult<Option<Box<[RawValue]>>> {
        let target = self
            .stack
            .last()
            .expect("returning activation is active")
            .return_to();

        match target {
            Some(ReturnTarget::Lua(target)) => {
                let callee_index = self.stack.len() - 1;
                let (callers, callees) = self.stack.split_at_mut(callee_index);
                let caller = callers.last_mut().and_then(Activation::as_lua_mut).ok_or(
                    VmErrorKind::InvalidNativeContinuation {
                        message: "Lua return target has no Lua caller",
                    },
                )?;
                let callee = callees[0]
                    .as_lua()
                    .expect("only Lua activations produce Return boundaries");

                caller.frame_mut().accept_results_from_frame(
                    &mut *self.runtime,
                    target,
                    callee.frame(),
                    base,
                    values,
                )?;

                let activation = self.stack.pop().expect("returning activation is active");
                self.recycle_lua_activation(activation);

                Ok(None)
            }
            target => {
                let values = {
                    let runtime = &*self.runtime;

                    self.stack
                        .last_mut()
                        .and_then(Activation::as_lua_mut)
                        .expect("active activation is Lua")
                        .frame_mut()
                        .collect_return(runtime, base, values)?
                };
                let activation = self.stack.pop().expect("returning activation is active");
                self.recycle_lua_activation(activation);

                self.deliver_return(target, values)
            }
        }
    }

    fn recycle_lua_activation(&mut self, activation: Activation) {
        let Activation::Lua(activation) = activation else {
            unreachable!("only Lua activations produce Return boundaries");
        };

        self.spare_lua_frame = Some(activation.into_frame().into_storage());
    }

    fn error_argument(&self, error: &VmError) -> FaultResult<RawValue> {
        self.runtime.import_value(error.object_or_message())
    }

    fn route_error(&mut self, mut error: VmError) -> Result<(), VmError> {
        loop {
            let Some(activation) = self.stack.last() else {
                return Err(error);
            };

            if activation.as_lua().is_some() {
                let frame = activation.trace_frame();
                error.append_trace(vec![frame].into_boxed_slice(), 0);

                let cause = self.error_argument(&error).map_err(VmError::from)?;

                if self.active_lua_activation().is_closing() {
                    self.active_lua_activation_mut()
                        .replace_close_error(cause, error);

                    return Ok(());
                }

                self.prepare_close(Register(0), cause, CloseCompletion::Unwind(error))
                    .map_err(VmError::from)?;

                return Ok(());
            }

            let activation = self
                .stack
                .pop()
                .expect("error routing inspected an active activation");

            error.append_trace(vec![activation.trace_frame()].into_boxed_slice(), 0);

            let target = activation.return_to();

            match target {
                Some(ReturnTarget::Native { token, .. }) => {
                    let result = self
                        .stack
                        .last_mut()
                        .and_then(Activation::as_native_mut)
                        .ok_or(VmErrorKind::InvalidNativeContinuation {
                            message: "native error boundary has no native caller",
                        })
                        .and_then(|parent| parent.resume_error_from_action(token, error));

                    return result.map_err(VmError::from);
                }
                Some(ReturnTarget::Lua(_)) => (),
                None => {
                    if self.stack.is_empty() {
                        return Err(error);
                    }
                }
            }
        }
    }

    fn forward_error(
        &mut self,
        target: Option<ReturnTarget>,
        error: VmError,
    ) -> Result<(), VmError> {
        match target {
            Some(ReturnTarget::Native { token, .. }) => {
                let result = self
                    .stack
                    .last_mut()
                    .and_then(Activation::as_native_mut)
                    .ok_or(VmErrorKind::InvalidNativeContinuation {
                        message: "native error boundary has no native caller",
                    })
                    .and_then(|parent| parent.resume_error_from_action(token, error));

                match result {
                    Ok(()) => Ok(()),
                    Err(kind) => self.route_error(VmError::from(kind)),
                }
            }
            Some(ReturnTarget::Lua(_)) | None => self.route_error(error),
        }
    }

    fn active_lua_activation(&self) -> &LuaActivation {
        self.stack
            .last()
            .and_then(Activation::as_lua)
            .expect("active activation is Lua")
    }

    fn active_lua_activation_mut(&mut self) -> &mut LuaActivation {
        self.stack
            .last_mut()
            .and_then(Activation::as_lua_mut)
            .expect("active activation is Lua")
    }

    fn active_lua_frame(&self) -> &CallFrame {
        self.active_lua_activation().frame()
    }

    fn active_lua_frame_mut(&mut self) -> &mut CallFrame {
        self.active_lua_activation_mut().frame_mut()
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

    fn trace_frames(activations: &[Activation]) -> (Box<[VmTraceFrame]>, usize) {
        const HEAD: usize = 10;
        const TAIL: usize = 11;

        if activations.len() <= HEAD + TAIL {
            return (
                activations
                    .iter()
                    .rev()
                    .map(Activation::trace_frame)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                0,
            );
        }

        let frames = activations
            .iter()
            .rev()
            .take(HEAD)
            .chain(activations.iter().take(TAIL).rev())
            .map(Activation::trace_frame)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        (frames, activations.len() - HEAD - TAIL)
    }

    fn runtime_error(&self, kind: VmErrorKind) -> VmError {
        let (frames, omitted) = Self::trace_frames(&self.stack);
        let mut error = VmError::new(kind);
        error.append_trace(frames, omitted);
        error
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
