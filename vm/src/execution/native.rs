use orbit_compiler::bytecode::{BinaryOp, UnaryOp};

use crate::{
    error::{VmError, VmErrorKind, VmResult},
    execution::operators::ComparisonOutcome,
    id::ObjectId,
    loading::LoadSource,
    native::{
        ArithmeticOp, ComparisonOp, NativeActionKind, NativeContext, NativeServices, NativeToken,
    },
    runtime::GcMode,
    semantics,
    value::{RawValue, Value},
};

use super::{
    Activation, Execution, NativeActivation, NativeResultMode, ReturnTarget,
    tables::{IndexOutcome, NewIndexOutcome},
};

pub(super) enum NativeStep {
    Continue,
    Complete(Box<[RawValue]>),
    Yield(Box<[RawValue]>),
}

impl Execution<'_> {
    pub(super) fn invoke_native_top(&mut self) -> Result<NativeStep, VmError> {
        let roots = self.root_snapshot()?;

        let activation = self.stack.pop().expect("native activation is active");

        let Activation::Native(mut activation) = activation else {
            unreachable!("invoke_native_top requires a native activation");
        };

        let event = match activation.begin_invocation() {
            Ok(event) => event,
            Err(kind) => {
                self.stack.push(Activation::Native(activation));

                return self.route_native_error(VmError::from(kind));
            }
        };

        let callback = activation.callback();

        let callback_result = {
            let event = event.as_data();

            let mut services = ExecutionNativeServices {
                runtime: &mut *self.runtime,
                roots: roots.into_vec(),
            };

            let mut context = NativeContext::new(
                &mut services,
                activation.arguments(),
                activation.captures(),
                event,
            );

            callback(&mut context)
        };

        let action = match callback_result {
            Ok(action) => action.into_kind(),
            Err(error) => {
                self.stack.push(Activation::Native(activation));

                return self.route_native_error(error);
            }
        };

        match action {
            NativeActionKind::Return { values } => {
                let target = activation.return_to();

                match self.deliver_return(target, values) {
                    Ok(Some(values)) => Ok(NativeStep::Complete(values)),
                    Ok(None) => Ok(NativeStep::Continue),
                    Err(kind) => self.route_native_error(VmError::from(kind)),
                }
            }
            NativeActionKind::Call {
                callee,
                arguments,
                continuation,
                token,
            } => self.invoke_native_action(
                activation,
                callee,
                arguments,
                continuation,
                token,
                NativeResultMode::All,
            ),
            NativeActionKind::Get {
                target,
                key,
                continuation,
                token,
            } => match self.resolve_index(target, key) {
                Ok(IndexOutcome::Value(value)) => self.resume_native_action(
                    activation,
                    continuation,
                    token,
                    vec![value].into_boxed_slice(),
                    NativeResultMode::First,
                ),
                Ok(IndexOutcome::Invoke { callee, arguments }) => self.invoke_native_action(
                    activation,
                    callee,
                    arguments,
                    continuation,
                    token,
                    NativeResultMode::First,
                ),
                Err(kind) => {
                    self.fail_native_action(activation, continuation, token, VmError::from(kind))
                }
            },
            NativeActionKind::Set {
                target,
                key,
                value,
                continuation,
                token,
            } => match self.resolve_new_index(target, key, value) {
                Ok(NewIndexOutcome::Done) => self.resume_native_action(
                    activation,
                    continuation,
                    token,
                    Box::default(),
                    NativeResultMode::None,
                ),
                Ok(NewIndexOutcome::Invoke { callee, arguments }) => self.invoke_native_action(
                    activation,
                    callee,
                    arguments,
                    continuation,
                    token,
                    NativeResultMode::None,
                ),
                Err(kind) => {
                    self.fail_native_action(activation, continuation, token, VmError::from(kind))
                }
            },
            NativeActionKind::Yield { values, token } => {
                if let Err(kind) = activation.wait_for_host(token) {
                    self.stack.push(Activation::Native(activation));

                    return self.route_native_error(VmError::from(kind));
                }

                self.stack.push(Activation::Native(activation));

                Ok(NativeStep::Yield(values))
            }
            NativeActionKind::Compare {
                operation,
                left,
                right,
                continuation,
                token,
            } => {
                let operation = match operation {
                    ComparisonOp::Equal => BinaryOp::Equal,
                    ComparisonOp::LessThan => BinaryOp::LessThan,
                    ComparisonOp::LessEqual => BinaryOp::LessEqual,
                };

                match self.resolve_comparison(operation, left, right) {
                    Ok(ComparisonOutcome::Value(result)) => self.resume_native_action(
                        activation,
                        continuation,
                        token,
                        vec![RawValue::Boolean(result)].into_boxed_slice(),
                        NativeResultMode::Boolean,
                    ),

                    Ok(ComparisonOutcome::Invoke { callee, arguments }) => self
                        .invoke_native_action(
                            activation,
                            callee,
                            arguments,
                            continuation,
                            token,
                            NativeResultMode::Boolean,
                        ),

                    Err(error) => self.fail_native_action(
                        activation,
                        continuation,
                        token,
                        VmError::from(error),
                    ),
                }
            }
        }
    }

    fn invoke_native_action(
        &mut self,
        mut activation: NativeActivation,
        callee: RawValue,
        arguments: Box<[RawValue]>,
        continuation: Box<[RawValue]>,
        token: NativeToken,
        results: NativeResultMode,
    ) -> Result<NativeStep, VmError> {
        if let Err(kind) = activation.wait_for_action(token, continuation) {
            self.stack.push(Activation::Native(activation));

            return self.route_native_error(VmError::from(kind));
        }

        self.stack.push(Activation::Native(activation));

        let return_to = ReturnTarget::Native { token, results };

        if let Err(kind) = self.push_callable(callee, arguments, return_to) {
            let parent = self
                .stack
                .last_mut()
                .expect("native caller remains active")
                .as_native_mut()
                .expect("native caller remains on top when child creation fails");

            if let Err(resume_kind) = parent.resume_error_from_action(token, VmError::from(kind)) {
                return self.route_native_error(VmError::from(resume_kind));
            }
        }

        Ok(NativeStep::Continue)
    }

    fn resume_native_action(
        &mut self,
        mut activation: NativeActivation,
        continuation: Box<[RawValue]>,
        token: NativeToken,
        values: Box<[RawValue]>,
        results: NativeResultMode,
    ) -> Result<NativeStep, VmError> {
        if let Err(kind) = activation.wait_for_action(token, continuation) {
            self.stack.push(Activation::Native(activation));

            return self.route_native_error(VmError::from(kind));
        }

        if let Err(kind) = activation.resume_from_action(token, results.normalize(values)) {
            self.stack.push(Activation::Native(activation));

            return self.route_native_error(VmError::from(kind));
        }

        self.stack.push(Activation::Native(activation));

        Ok(NativeStep::Continue)
    }

    fn fail_native_action(
        &mut self,
        mut activation: NativeActivation,
        continuation: Box<[RawValue]>,
        token: NativeToken,
        error: VmError,
    ) -> Result<NativeStep, VmError> {
        if let Err(kind) = activation.wait_for_action(token, continuation) {
            self.stack.push(Activation::Native(activation));

            return self.route_native_error(VmError::from(kind));
        }

        if let Err(kind) = activation.resume_error_from_action(token, error) {
            self.stack.push(Activation::Native(activation));

            return self.route_native_error(VmError::from(kind));
        }

        self.stack.push(Activation::Native(activation));

        Ok(NativeStep::Continue)
    }

    fn route_native_error(&mut self, error: VmError) -> Result<NativeStep, VmError> {
        match self.route_error(error) {
            Ok(()) => Ok(NativeStep::Continue),
            Err(error) => Err(error),
        }
    }
}

struct ExecutionNativeServices<'runtime> {
    runtime: &'runtime mut crate::runtime::Runtime,
    roots: Vec<ObjectId>,
}

impl ExecutionNativeServices<'_> {
    fn keep_alive(&mut self, value: RawValue) -> VmResult<RawValue> {
        let Some(root) = value.object_id() else {
            return Ok(value);
        };

        if self.roots.contains(&root) {
            return Ok(value);
        }

        let requested = self.roots.len().saturating_add(1);

        self.roots
            .try_reserve(1)
            .map_err(|_| VmError::from(VmErrorKind::RootCapacityExceeded { requested }))?;

        self.roots.push(root);
        Ok(value)
    }
}

impl NativeServices for ExecutionNativeServices<'_> {
    fn import_value(&mut self, value: Value) -> VmResult<RawValue> {
        let value = self.runtime.import_value(value)?;
        self.keep_alive(value)
    }

    fn export_value(&mut self, value: &RawValue) -> VmResult<Value> {
        self.runtime.export_value(value).map_err(VmError::from)
    }

    fn create_table(&mut self, array_hint: usize, hash_hint: usize) -> VmResult<RawValue> {
        let value = self
            .runtime
            .allocate_table(array_hint, hash_hint)
            .map(RawValue::Table)?;

        self.keep_alive(value)
    }

    fn create_native_function(
        &mut self,
        name: Box<str>,
        callback: crate::NativeCallback,
        captures: Box<[RawValue]>,
    ) -> VmResult<RawValue> {
        let value = self
            .runtime
            .allocate_native_function(name, callback, captures)
            .map(RawValue::Function)?;

        self.keep_alive(value)
    }

    fn raw_get(&mut self, table: &RawValue, key: &RawValue) -> VmResult<RawValue> {
        let table = table.as_table().ok_or_else(|| {
            VmError::from(VmErrorKind::InvalidTableOperand {
                kind: table.type_name(),
            })
        })?;

        let value = self.runtime.raw_get(table, key)?;
        self.keep_alive(value)
    }

    fn raw_set(&mut self, table: &RawValue, key: RawValue, value: RawValue) -> VmResult<()> {
        let table = table.as_table().ok_or_else(|| {
            VmError::from(VmErrorKind::InvalidTableOperand {
                kind: table.type_name(),
            })
        })?;

        self.runtime
            .raw_set(table, key, value)
            .map_err(VmError::from)
    }

    fn raw_len(&self, table: &RawValue) -> VmResult<i64> {
        let table = table.as_table().ok_or_else(|| {
            VmError::from(VmErrorKind::InvalidLengthOperand {
                kind: table.type_name(),
            })
        })?;

        self.runtime.raw_len(table).map_err(VmError::from)
    }

    fn get_metatable(&mut self, value: &RawValue) -> VmResult<Option<RawValue>> {
        let metatable = self.runtime.metatable(value)?.map(RawValue::Table);

        match metatable {
            Some(value) => self.keep_alive(value).map(Some),
            None => Ok(None),
        }
    }

    fn set_metatable(&mut self, value: &RawValue, metatable: Option<&RawValue>) -> VmResult<()> {
        let metatable = metatable
            .map(|metatable| {
                metatable.as_table().ok_or_else(|| {
                    VmError::from(VmErrorKind::InvalidTableOperand {
                        kind: metatable.type_name(),
                    })
                })
            })
            .transpose()?;

        self.runtime
            .set_metatable(value, metatable)
            .map(|_| ())
            .map_err(VmError::from)
    }

    fn next(
        &mut self,
        table: &RawValue,
        previous: &RawValue,
    ) -> VmResult<Option<(RawValue, RawValue)>> {
        let table = table.as_table().ok_or_else(|| {
            VmError::from(VmErrorKind::InvalidTableOperand {
                kind: table.type_name(),
            })
        })?;

        let entry = self.runtime.next(table, previous)?;

        match entry {
            Some((key, value)) => {
                let key = self.keep_alive(key)?;
                let value = self.keep_alive(value)?;
                Ok(Some((key, value)))
            }
            None => Ok(None),
        }
    }

    fn load_source(
        &mut self,
        source: LoadSource<'_>,
        environment: Option<RawValue>,
    ) -> VmResult<RawValue> {
        let value = self
            .runtime
            .load_source_raw(source, environment)
            .map(RawValue::Function)?;

        self.keep_alive(value)
    }

    fn file_exists(&self, filename: &[u8]) -> bool {
        self.runtime.file_exists(filename)
    }

    fn collect_garbage(&mut self) -> VmResult<usize> {
        self.runtime
            .collect_garbage(&self.roots)
            .map_err(VmError::from)
    }

    fn gc_memory_kbytes(&self) -> f64 {
        self.runtime.memory_kbytes()
    }

    fn set_gc_running(&mut self, running: bool) {
        if running {
            self.runtime.restart_gc();
        } else {
            self.runtime.stop_gc();
        }
    }

    fn set_gc_mode(&mut self, mode: GcMode) -> GcMode {
        self.runtime.set_gc_mode(mode)
    }

    fn gc_running(&self) -> bool {
        self.runtime.gc_running()
    }

    fn raw_arithmetic(
        &mut self,
        operation: ArithmeticOp,
        left: &RawValue,
        right: &RawValue,
    ) -> VmResult<RawValue> {
        let operation = match operation {
            ArithmeticOp::Add => BinaryOp::Add,
            ArithmeticOp::Subtract => BinaryOp::Subtract,
            ArithmeticOp::Multiply => BinaryOp::Multiply,
            ArithmeticOp::Divide => BinaryOp::Divide,
            ArithmeticOp::FloorDivide => BinaryOp::FloorDivide,
            ArithmeticOp::Modulo => BinaryOp::Modulo,
            ArithmeticOp::Power => BinaryOp::Power,
        };

        let value = semantics::binary(operation, left, right).map_err(VmError::from)?;
        self.keep_alive(value)
    }

    fn raw_negate(&mut self, operand: &RawValue) -> VmResult<RawValue> {
        let value = semantics::unary(UnaryOp::Negate, operand).map_err(VmError::from)?;
        self.keep_alive(value)
    }

    fn function_upvalue_id(
        &mut self,
        function: &RawValue,
        index: usize,
    ) -> VmResult<Option<RawValue>> {
        let RawValue::Function(function) = function else {
            return Ok(None);
        };

        let identity = self
            .runtime
            .function_upvalue_id(*function, index)
            .map(|identity| identity.map(RawValue::LightUserdata))
            .map_err(VmError::from)?;

        match identity {
            Some(value) => self.keep_alive(value).map(Some),
            None => Ok(None),
        }
    }
}
