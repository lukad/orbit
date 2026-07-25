use orbit_compiler::bytecode::{Count, Register};

use crate::{
    error::{FaultResult, VmErrorKind},
    execution::Activation,
    id::FunctionId,
    value::RawValue,
};

use super::{Execution, FrameBoundary, ResultTarget, offset_register};

const CALL_METAMETHOD: &[u8] = b"__call";
const MAX_CALL_REDIRECTS: usize = 2_000;

impl Execution<'_> {
    pub(super) fn call(
        &mut self,
        base: Register,
        arguments: Count,
        results: Count,
    ) -> FaultResult<FrameBoundary> {
        let (callee, arguments) = {
            let runtime = &*self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .collect_call(runtime, base, arguments)?
        };

        Ok(FrameBoundary::Invoke {
            callee,
            arguments,
            target: ResultTarget::Call {
                base: usize::from(base.0),
                results,
            },
        })
    }

    pub(super) fn tail_call(
        &mut self,
        base: Register,
        arguments: Count,
        close_from: Option<Register>,
    ) -> FaultResult<FrameBoundary> {
        let mut collected = std::mem::take(&mut self.tail_arguments);
        let collect_result = {
            let runtime = &*self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .collect_call_into(runtime, base, arguments, &mut collected)
        };
        let callee = match collect_result {
            Ok(callee) => callee,
            Err(error) => {
                self.tail_arguments = collected;
                return Err(error);
            }
        };

        if let Some(close_from) = close_from {
            let runtime = &*self.runtime;

            if let Err(error) = self
                .stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .close_upvalues_from(runtime, close_from)
            {
                self.tail_arguments = collected;
                return Err(error);
            }
        }

        Ok(FrameBoundary::TailInvoke {
            callee,
            arguments: collected,
        })
    }

    pub(super) fn resolve_callable(
        &self,
        mut callee: RawValue,
        mut arguments: Box<[RawValue]>,
    ) -> FaultResult<(FunctionId, Box<[RawValue]>)> {
        let mut call_chain = Vec::new();

        let function = loop {
            if let RawValue::Function(function) = callee {
                break function;
            }

            if call_chain.len() == MAX_CALL_REDIRECTS {
                return Err(VmErrorKind::MetamethodChainTooLong {
                    metamethod: "__call",
                });
            }

            let metamethod = self.runtime.metamethod(&callee, CALL_METAMETHOD)?;

            if metamethod.is_nil() {
                return Err(VmErrorKind::InvalidCallOperand {
                    kind: callee.type_name(),
                });
            }

            let requested =
                call_chain
                    .len()
                    .checked_add(1)
                    .ok_or(VmErrorKind::FrameCapacityExceeded {
                        requested: usize::MAX,
                    })?;

            call_chain
                .try_reserve(1)
                .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested })?;

            call_chain.push(callee);
            callee = metamethod;
        };

        if !call_chain.is_empty() {
            let requested = call_chain.len().checked_add(arguments.len()).ok_or(
                VmErrorKind::FrameCapacityExceeded {
                    requested: usize::MAX,
                },
            )?;

            let mut forwarded = Vec::new();

            forwarded
                .try_reserve_exact(requested)
                .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested })?;

            forwarded.extend(call_chain.into_iter().rev());
            forwarded.extend(arguments);
            arguments = forwarded.into_boxed_slice();
        }

        Ok((function, arguments))
    }

    pub(super) fn resolve_callable_vec(
        &self,
        mut callee: RawValue,
        arguments: &mut Vec<RawValue>,
    ) -> FaultResult<FunctionId> {
        let mut redirects = 0;

        loop {
            if let RawValue::Function(function) = callee {
                return Ok(function);
            }

            if redirects == MAX_CALL_REDIRECTS {
                return Err(VmErrorKind::MetamethodChainTooLong {
                    metamethod: "__call",
                });
            }

            let metamethod = self.runtime.metamethod(&callee, CALL_METAMETHOD)?;

            if metamethod.is_nil() {
                return Err(VmErrorKind::InvalidCallOperand {
                    kind: callee.type_name(),
                });
            }

            let requested =
                arguments
                    .len()
                    .checked_add(1)
                    .ok_or(VmErrorKind::FrameCapacityExceeded {
                        requested: usize::MAX,
                    })?;
            arguments
                .try_reserve(1)
                .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested })?;
            arguments.insert(0, callee);

            callee = metamethod;
            redirects += 1;
        }
    }

    pub(super) fn return_values(
        &mut self,
        base: Register,
        values: Count,
        close_from: Option<Register>,
    ) -> FaultResult<FrameBoundary> {
        let values = {
            let runtime = &*self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .collect_return(runtime, base, values)?
        };

        if let Some(close_from) = close_from {
            let runtime = &*self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .close_upvalues_from(runtime, close_from)?;
        }

        Ok(FrameBoundary::Return { values })
    }

    pub(super) fn generic_for_call(
        &mut self,
        base: Register,
        variables: u8,
    ) -> FaultResult<FrameBoundary> {
        if variables == 0 {
            return Err(VmErrorKind::InvalidGenericForVariableCount);
        }

        let state_register = offset_register(base, 1)?;
        let control_register = offset_register(base, 2)?;
        let result_register = offset_register(base, 4)?;
        let iterator = self.read_register(base)?;
        let state = self.read_register(state_register)?;
        let control = self.read_register(control_register)?;

        Ok(FrameBoundary::Invoke {
            callee: iterator,
            arguments: vec![state, control].into_boxed_slice(),
            target: ResultTarget::GenericFor {
                start: usize::from(result_register.0),
                variables: usize::from(variables),
            },
        })
    }
}
