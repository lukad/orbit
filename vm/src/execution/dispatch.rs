use orbit_compiler::bytecode::Instruction;

use crate::error::FaultResult;

use super::{Execution, FrameBoundary};

impl Execution<'_> {
    pub(super) fn dispatch(
        &mut self,
        instruction: Instruction,
    ) -> FaultResult<Option<FrameBoundary>> {
        match instruction {
            Instruction::LoadNil { dst } => {
                self.load_nil(dst)?;
            }
            Instruction::LoadBool { dst, value } => {
                self.load_boolean(dst, value)?;
            }
            Instruction::LoadSmallInt { dst, value } => {
                self.load_small_integer(dst, value)?;
            }
            Instruction::LoadConst { dst, constant } => {
                self.load_constant(dst, constant)?;
            }
            Instruction::Move { dst, src } => {
                self.move_value(dst, src)?;
            }
            Instruction::GetUpvalue { dst, upvalue } => {
                self.get_upvalue(dst, upvalue)?;
            }
            Instruction::SetUpvalue { upvalue, src } => {
                self.set_upvalue(upvalue, src)?;
            }
            Instruction::Vararg { base, results } => {
                self.vararg(base, results)?;
            }
            Instruction::Closure { dst, child } => {
                self.create_closure(dst, child)?;
            }
            Instruction::NewTable {
                dst,
                array_hint,
                hash_hint,
            } => {
                self.new_table(dst, array_hint, hash_hint)?;
            }
            Instruction::GetTable { dst, table, key } => {
                if let Some(boundary) = self.get_table(dst, table, key)? {
                    return Ok(Some(boundary));
                }
            }
            Instruction::SetTable { table, key, value } => {
                if let Some(boundary) = self.set_table(table, key, value)? {
                    return Ok(Some(boundary));
                }
            }
            Instruction::SetList {
                table,
                src,
                first_index,
                count,
            } => {
                self.set_list(table, src, first_index, count)?;
            }
            Instruction::Unary { op, dst, operand } => {
                if let Some(boundary) = self.unary(op, dst, operand)? {
                    return Ok(Some(boundary));
                }
            }
            Instruction::Binary {
                op,
                dst,
                left,
                right,
            } => {
                if let Some(boundary) = self.binary(op, dst, left, right)? {
                    return Ok(Some(boundary));
                }
            }
            Instruction::MarkToClose { register } => {
                self.mark_to_close(register)?;
            }
            Instruction::CloseFrom { base } => {
                self.close_from(base)?;
            }
            Instruction::Jump { offset } => {
                self.jump(offset)?;
            }
            Instruction::JumpIfFalsy { condition, offset } => {
                self.jump_if_falsy(condition, offset)?;
            }
            Instruction::ForPrep { base, exit_offset } => {
                self.for_prep(base, exit_offset)?;
            }
            Instruction::ForLoop { base, body_offset } => {
                self.for_loop(base, body_offset)?;
            }
            Instruction::TForLoop { base, body_offset } => {
                self.generic_for_loop(base, body_offset)?;
            }
            Instruction::Call {
                base,
                arguments,
                results,
            } => {
                return self.call(base, arguments, results).map(Some);
            }
            Instruction::TailCall {
                base,
                arguments,
                close_from,
            } => {
                return self.tail_call(base, arguments, close_from).map(Some);
            }
            Instruction::TForCall { base, variables } => {
                return self.generic_for_call(base, variables).map(Some);
            }
            Instruction::Return {
                base,
                values,
                close_from,
            } => {
                return self.return_values(base, values, close_from).map(Some);
            }
        }

        Ok(None)
    }
}
