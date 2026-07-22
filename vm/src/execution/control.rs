use orbit_compiler::bytecode::Register;

use crate::error::{FaultResult, VmErrorKind};

use super::{Activation, Execution};

impl Execution<'_> {
    pub(super) fn mark_to_close(&mut self, register: Register) -> FaultResult<()> {
        let value = self.read_register(register)?;

        if value.is_truthy() {
            return Err(VmErrorKind::UnsupportedToBeClosedLocal);
        }

        Ok(())
    }

    pub(super) fn close_from(&mut self, base: Register) -> FaultResult<()> {
        let runtime = &*self.runtime;

        self.stack
            .last_mut()
            .and_then(Activation::as_lua_mut)
            .expect("active activation is Lua")
            .frame_mut()
            .close_upvalues_from(runtime, base)
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
}
