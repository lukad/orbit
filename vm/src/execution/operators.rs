use orbit_compiler::bytecode::{BinaryOp, Register, UnaryOp};

use crate::{error::FaultResult, semantics, value::RawValue};

use super::{Execution, FrameBoundary, ResultTarget};

impl Execution<'_> {
    pub(super) fn unary(
        &mut self,
        operation: UnaryOp,
        destination: Register,
        operand: Register,
    ) -> FaultResult<Option<FrameBoundary>> {
        let operand = self.read_register(operand)?;

        let primitive = match (operation, &operand) {
            (UnaryOp::Length, RawValue::Table(table)) => {
                Ok(RawValue::Integer(self.runtime.raw_len(*table)?))
            }
            _ => semantics::unary(operation, &operand),
        };

        match primitive {
            Ok(result) => {
                self.write_register(destination, result)?;

                Ok(None)
            }
            Err(error) => {
                let Some(name) = unary_metamethod(operation) else {
                    return Err(error);
                };

                if unary_primitive_applies(operation, &operand) {
                    return Err(error);
                }

                let metamethod = self.runtime.metamethod(&operand, name)?;

                if metamethod.is_nil() {
                    return Err(error);
                }

                let arguments = vec![operand.clone(), operand].into_boxed_slice();

                Ok(Some(FrameBoundary::Invoke {
                    callee: metamethod,
                    arguments,
                    target: ResultTarget::Operator { destination },
                }))
            }
        }
    }

    pub(super) fn binary(
        &mut self,
        operation: BinaryOp,
        destination: Register,
        left: Register,
        right: Register,
    ) -> FaultResult<Option<FrameBoundary>> {
        let left = self.read_register(left)?;
        let right = self.read_register(right)?;

        match semantics::binary(operation, &left, &right) {
            Ok(result) => {
                self.write_register(destination, result)?;

                Ok(None)
            }
            Err(error) => {
                let Some(name) = binary_metamethod(operation) else {
                    return Err(error);
                };

                if binary_primitive_applies(operation, &left, &right) {
                    return Err(error);
                }

                let mut metamethod = self.runtime.metamethod(&left, name)?;

                if metamethod.is_nil() {
                    metamethod = self.runtime.metamethod(&right, name)?;
                }

                if metamethod.is_nil() {
                    return Err(error);
                }

                let arguments = vec![left, right].into_boxed_slice();

                Ok(Some(FrameBoundary::Invoke {
                    callee: metamethod,
                    arguments,
                    target: ResultTarget::Operator { destination },
                }))
            }
        }
    }
}

fn unary_metamethod(operation: UnaryOp) -> Option<&'static [u8]> {
    match operation {
        UnaryOp::Negate => Some(b"__unm"),
        UnaryOp::BitwiseNot => Some(b"__bnot"),
        UnaryOp::Not | UnaryOp::Length => None,
    }
}

fn binary_metamethod(operation: BinaryOp) -> Option<&'static [u8]> {
    match operation {
        BinaryOp::Add => Some(b"__add"),
        BinaryOp::Subtract => Some(b"__sub"),
        BinaryOp::Multiply => Some(b"__mul"),
        BinaryOp::Divide => Some(b"__div"),
        BinaryOp::FloorDivide => Some(b"__idiv"),
        BinaryOp::Modulo => Some(b"__mod"),
        BinaryOp::Power => Some(b"__pow"),
        BinaryOp::BitwiseAnd => Some(b"__band"),
        BinaryOp::BitwiseOr => Some(b"__bor"),
        BinaryOp::BitwiseXor => Some(b"__bxor"),
        BinaryOp::ShiftLeft => Some(b"__shl"),
        BinaryOp::ShiftRight => Some(b"__shr"),
        BinaryOp::Concat
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::LessThan
        | BinaryOp::LessEqual
        | BinaryOp::GreaterThan
        | BinaryOp::GreaterEqual => None,
    }
}

fn unary_primitive_applies(operation: UnaryOp, operand: &RawValue) -> bool {
    match operation {
        UnaryOp::Negate => operand.to_float().is_some(),
        UnaryOp::BitwiseNot => operand.to_integer().is_some(),
        UnaryOp::Not | UnaryOp::Length => true,
    }
}

fn binary_primitive_applies(operation: BinaryOp, left: &RawValue, right: &RawValue) -> bool {
    match operation {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::FloorDivide
        | BinaryOp::Modulo
        | BinaryOp::Power => left.to_float().is_some() && right.to_float().is_some(),

        BinaryOp::BitwiseAnd
        | BinaryOp::BitwiseOr
        | BinaryOp::BitwiseXor
        | BinaryOp::ShiftLeft
        | BinaryOp::ShiftRight => left.to_integer().is_some() && right.to_integer().is_some(),

        BinaryOp::Concat
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::LessThan
        | BinaryOp::LessEqual
        | BinaryOp::GreaterThan
        | BinaryOp::GreaterEqual => true,
    }
}
