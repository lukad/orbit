use orbit_compiler::bytecode::ImmediateOperandSide;
use orbit_compiler::bytecode::Register;

use crate::execution::FrameBoundary;
use crate::execution::ResultTarget;
use crate::execution::activation::CloseCompletion;
use crate::{
    error::{FaultResult, VmErrorKind},
    semantics,
    value::RawValue,
};

use super::{Activation, Execution};

impl Execution<'_> {
    pub(super) fn mark_to_close(&mut self, register: Register) -> FaultResult<()> {
        let value = self.read_register(register)?;

        if value.is_falsy() {
            return Ok(());
        }

        let close = self.runtime.metamethod(&value, b"__close")?;

        if close.is_nil() {
            let name = self.active_lua_frame().close_name().unwrap_or("?");
            return Err(VmErrorKind::NonClosableValue { name: name.into() });
        }

        self.active_lua_frame_mut().mark_to_close(register)
    }

    pub(super) fn close_from(&mut self, base: Register) -> FaultResult<Option<FrameBoundary>> {
        self.prepare_close(base, RawValue::Nil, CloseCompletion::Resume)?;
        self.continue_close()
    }

    pub(super) fn continue_close(&mut self) -> FaultResult<Option<FrameBoundary>> {
        if !self.active_lua_activation().is_closing() {
            return Ok(None);
        }

        if let Some((register, cause)) = self.active_lua_activation_mut().next_to_close() {
            let value = self.read_register(register)?;
            let close = self.runtime.metamethod(&value, b"__close")?;

            return Ok(Some(FrameBoundary::Invoke {
                callee: close,
                arguments: vec![value, cause].into_boxed_slice(),
                target: ResultTarget::Close,
            }));
        }

        match self.active_lua_activation_mut().finish_close() {
            CloseCompletion::Resume => Ok(None),
            CloseCompletion::ReturnOwned(values) => Ok(Some(FrameBoundary::ReturnOwned { values })),
            CloseCompletion::Unwind(error) => Ok(Some(FrameBoundary::UnwindOwned { error })),
        }
    }

    pub(super) fn prepare_close(
        &mut self,
        base: Register,
        cause: RawValue,
        completion: CloseCompletion,
    ) -> FaultResult<()> {
        {
            let runtime = &*self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .close_upvalues_from(runtime, base)?;
        }

        self.active_lua_activation_mut()
            .begin_close(base, cause, completion);

        Ok(())
    }

    pub(super) fn jump(&mut self, offset: i32) -> FaultResult<()> {
        self.apply_jump(offset)
    }

    pub(super) fn jump_if_falsy(&mut self, condition: Register, offset: i32) -> FaultResult<()> {
        if !self.read_register(condition)?.is_truthy() {
            self.apply_jump(offset)?;
        }

        Ok(())
    }

    pub(super) fn jump_if_not_equal_small_integer(
        &mut self,
        register: Register,
        immediate: i16,
        side: ImmediateOperandSide,
        offset: i32,
    ) -> FaultResult<()> {
        let register = self.read_register(register)?;
        let immediate = RawValue::Integer(i64::from(immediate));
        let (left, right) = match side {
            ImmediateOperandSide::Left => (immediate, register),
            ImmediateOperandSide::Right => (register, immediate),
        };
        let RawValue::Boolean(equal) =
            semantics::binary(orbit_compiler::bytecode::BinaryOp::Equal, &left, &right)?
        else {
            unreachable!("equality always produces a boolean");
        };

        if !equal {
            self.apply_jump(offset)?;
        }

        Ok(())
    }
}
