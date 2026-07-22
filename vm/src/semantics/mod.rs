mod arithmetic;
mod comparison;
mod concat;
pub(crate) mod loops;

#[cfg(test)]
mod tests;

use orbit_compiler::bytecode::{BinaryOp, UnaryOp};

use crate::{
    error::{FaultResult, VmErrorKind},
    value::RawValue,
};

pub(crate) fn unary(operation: UnaryOp, operand: &RawValue) -> FaultResult<RawValue> {
    match operation {
        UnaryOp::Not => Ok(RawValue::Boolean(!operand.is_truthy())),
        UnaryOp::Length => match operand {
            RawValue::String(string) => {
                let length =
                    i64::try_from(string.len()).map_err(|_| VmErrorKind::StringTooLong {
                        length: string.len(),
                    })?;

                Ok(RawValue::Integer(length))
            }
            value => Err(VmErrorKind::InvalidLengthOperand {
                kind: value.type_name(),
            }),
        },
        UnaryOp::Negate => arithmetic::negate(operand),
        UnaryOp::BitwiseNot => arithmetic::bitwise_not(operand),
    }
}

pub(crate) fn binary(
    operation: BinaryOp,
    left: &RawValue,
    right: &RawValue,
) -> FaultResult<RawValue> {
    match operation {
        BinaryOp::Add => arithmetic::add(left, right),
        BinaryOp::Subtract => arithmetic::subtract(left, right),
        BinaryOp::Multiply => arithmetic::multiply(left, right),
        BinaryOp::Divide => arithmetic::divide(left, right),
        BinaryOp::FloorDivide => arithmetic::floor_divide(left, right),
        BinaryOp::Modulo => arithmetic::modulo(left, right),
        BinaryOp::Power => arithmetic::power(left, right),
        BinaryOp::BitwiseAnd => arithmetic::bitwise_and(left, right),
        BinaryOp::BitwiseOr => arithmetic::bitwise_or(left, right),
        BinaryOp::BitwiseXor => arithmetic::bitwise_xor(left, right),
        BinaryOp::ShiftLeft => arithmetic::shift_left_value(left, right),
        BinaryOp::ShiftRight => arithmetic::shift_right_value(left, right),
        BinaryOp::Concat => concat::concat(left, right),
        BinaryOp::Equal => Ok(RawValue::Boolean(comparison::equal(left, right))),
        BinaryOp::NotEqual => Ok(RawValue::Boolean(!comparison::equal(left, right))),
        BinaryOp::LessThan => comparison::less_than(left, right).map(RawValue::Boolean),
        BinaryOp::LessEqual => comparison::less_equal(left, right).map(RawValue::Boolean),
        BinaryOp::GreaterThan => comparison::greater_than(left, right).map(RawValue::Boolean),
        BinaryOp::GreaterEqual => comparison::greater_equal(left, right).map(RawValue::Boolean),
    }
}
