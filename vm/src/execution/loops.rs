use orbit_compiler::bytecode::Register;

use crate::{
    error::FaultResult,
    semantics::loops::{advance_numeric_for, prepare_numeric_for},
    value::RawValue,
};

use super::{Execution, offset_register};

impl Execution<'_> {
    pub(super) fn for_prep(&mut self, base: Register, exit_offset: i32) -> FaultResult<()> {
        let limit_register = offset_register(base, 1)?;
        let step_register = offset_register(base, 2)?;
        let visible_register = offset_register(base, 3)?;
        let initial = self.read_register(base)?;
        let limit = self.read_register(limit_register)?;
        let step = self.read_register(step_register)?;
        let preparation = prepare_numeric_for(&initial, &limit, &step)?;
        let (index, limit, step, visible) = preparation.into_parts();

        self.write_register(base, index)?;
        self.write_register(limit_register, limit)?;
        self.write_register(step_register, step)?;

        if let Some(visible) = visible {
            self.write_register(visible_register, visible)?;
        } else {
            self.apply_jump(exit_offset)?;
        }

        Ok(())
    }

    pub(super) fn for_loop(&mut self, base: Register, body_offset: i32) -> FaultResult<()> {
        let limit_register = offset_register(base, 1)?;
        let step_register = offset_register(base, 2)?;
        let visible_register = offset_register(base, 3)?;
        let index = self.read_register(base)?;
        let limit = self.read_register(limit_register)?;
        let step = self.read_register(step_register)?;
        let advance = advance_numeric_for(&index, &limit, &step)?;
        let (index, visible) = advance.into_parts();

        self.write_register(base, index)?;

        if let Some(visible) = visible {
            self.write_register(visible_register, visible)?;

            self.apply_jump(body_offset)?;
        }

        Ok(())
    }

    pub(super) fn generic_for_loop(&mut self, base: Register, body_offset: i32) -> FaultResult<()> {
        let control_register = offset_register(base, 2)?;
        let first_result_register = offset_register(base, 4)?;
        let first_result = self.read_register(first_result_register)?;

        if !matches!(first_result, RawValue::Nil) {
            self.write_register(control_register, first_result)?;

            self.apply_jump(body_offset)?;
        }

        Ok(())
    }
}
