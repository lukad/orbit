use crate::{
    error::{FaultResult, VmErrorKind},
    number::{float_modulo, integer_floor_divide, integer_modulo, shift_left, shift_right},
    value::RawValue,
};

pub(super) fn negate(operand: &RawValue) -> FaultResult<RawValue> {
    match operand {
        RawValue::Integer(value) => Ok(RawValue::Integer(value.wrapping_neg())),
        RawValue::Float(value) => Ok(RawValue::Float(-value)),
        value => Err(VmErrorKind::InvalidNegateOperand {
            kind: value.type_name(),
        }),
    }
}

pub(super) fn bitwise_not(operand: &RawValue) -> FaultResult<RawValue> {
    let integer = operand
        .to_integer()
        .ok_or(VmErrorKind::InvalidBitwiseOperand {
            kind: operand.type_name(),
        })?;

    Ok(RawValue::Integer(!integer))
}

pub(super) fn add(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    match (left, right) {
        (RawValue::Integer(left), RawValue::Integer(right)) => {
            Ok(RawValue::Integer(left.wrapping_add(*right)))
        }
        _ => {
            let (Some(left), Some(right)) = (left.to_float(), right.to_float()) else {
                return Err(VmErrorKind::InvalidAddOperands {
                    left: left.type_name(),
                    right: right.type_name(),
                });
            };

            Ok(RawValue::Float(left + right))
        }
    }
}

pub(super) fn subtract(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    match (left, right) {
        (RawValue::Integer(left), RawValue::Integer(right)) => {
            Ok(RawValue::Integer(left.wrapping_sub(*right)))
        }
        _ => {
            let (Some(left), Some(right)) = (left.to_float(), right.to_float()) else {
                return Err(VmErrorKind::InvalidSubtractOperands {
                    left: left.type_name(),
                    right: right.type_name(),
                });
            };

            Ok(RawValue::Float(left - right))
        }
    }
}

pub(super) fn multiply(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    match (left, right) {
        (RawValue::Integer(left), RawValue::Integer(right)) => {
            Ok(RawValue::Integer(left.wrapping_mul(*right)))
        }
        _ => {
            let (Some(left), Some(right)) = (left.to_float(), right.to_float()) else {
                return Err(VmErrorKind::InvalidMultiplyOperands {
                    left: left.type_name(),
                    right: right.type_name(),
                });
            };

            Ok(RawValue::Float(left * right))
        }
    }
}

pub(super) fn divide(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    let (Some(left_number), Some(right_number)) = (left.to_float(), right.to_float()) else {
        return Err(VmErrorKind::InvalidDivideOperands {
            left: left.type_name(),
            right: right.type_name(),
        });
    };

    Ok(RawValue::Float(left_number / right_number))
}

pub(super) fn floor_divide(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    match (left, right) {
        (RawValue::Integer(left), RawValue::Integer(right)) => {
            if *right == 0 {
                return Err(VmErrorKind::IntegerDivisionByZero);
            }

            Ok(RawValue::Integer(integer_floor_divide(*left, *right)))
        }
        _ => {
            let (Some(left_number), Some(right_number)) = (left.to_float(), right.to_float())
            else {
                return Err(VmErrorKind::InvalidFloorDivideOperands {
                    left: left.type_name(),
                    right: right.type_name(),
                });
            };

            Ok(RawValue::Float((left_number / right_number).floor()))
        }
    }
}

pub(super) fn modulo(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    match (left, right) {
        (RawValue::Integer(left), RawValue::Integer(right)) => {
            if *right == 0 {
                return Err(VmErrorKind::IntegerModuloByZero);
            }

            Ok(RawValue::Integer(integer_modulo(*left, *right)))
        }
        _ => {
            let (Some(left_number), Some(right_number)) = (left.to_float(), right.to_float())
            else {
                return Err(VmErrorKind::InvalidModuloOperands {
                    left: left.type_name(),
                    right: right.type_name(),
                });
            };

            Ok(RawValue::Float(float_modulo(left_number, right_number)))
        }
    }
}

pub(super) fn power(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    let (Some(left_number), Some(right_number)) = (left.to_float(), right.to_float()) else {
        return Err(VmErrorKind::InvalidPowerOperands {
            left: left.type_name(),
            right: right.type_name(),
        });
    };

    Ok(RawValue::Float(left_number.powf(right_number)))
}

pub(super) fn bitwise_and(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    bitwise("bitwise and", left, right, |left, right| left & right)
}

pub(super) fn bitwise_or(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    bitwise("bitwise or", left, right, |left, right| left | right)
}

pub(super) fn bitwise_xor(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    bitwise("bitwise xor", left, right, |left, right| left ^ right)
}

pub(super) fn shift_left_value(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    bitwise("left shift", left, right, shift_left)
}

pub(super) fn shift_right_value(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    bitwise("right shift", left, right, shift_right)
}

fn bitwise(
    operation: &'static str,
    left: &RawValue,
    right: &RawValue,
    apply: impl FnOnce(i64, i64) -> i64,
) -> FaultResult<RawValue> {
    let (Some(left_integer), Some(right_integer)) = (left.to_integer(), right.to_integer()) else {
        return Err(VmErrorKind::InvalidBitwiseOperands {
            operation,
            left: left.type_name(),
            right: right.type_name(),
        });
    };

    Ok(RawValue::Integer(apply(left_integer, right_integer)))
}
