use orbit_compiler::bytecode::{BinaryOp, Register, UnaryOp};

use crate::{error::FaultResult, semantics, value::RawValue};

use super::{Execution, FrameBoundary, ResultTarget};

pub(crate) enum ComparisonOutcome {
    Value(bool),
    Invoke {
        callee: RawValue,
        arguments: Box<[RawValue]>,
    },
}

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
                let Some(name) = unary_metamethod_name(operation) else {
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

        if matches!(
            operation,
            BinaryOp::Equal
                | BinaryOp::LessThan
                | BinaryOp::LessEqual
                | BinaryOp::GreaterThan
                | BinaryOp::GreaterEqual
        ) {
            return match self.resolve_comparison(operation, left, right)? {
                ComparisonOutcome::Value(result) => {
                    self.write_register(destination, RawValue::Boolean(result))?;
                    Ok(None)
                }
                ComparisonOutcome::Invoke { callee, arguments } => {
                    Ok(Some(FrameBoundary::Invoke {
                        callee,
                        arguments,
                        target: ResultTarget::Comparison { destination },
                    }))
                }
            };
        }

        match semantics::binary(operation, &left, &right) {
            Ok(result) => {
                self.write_register(destination, result)?;
                Ok(None)
            }

            Err(error) => {
                let Some(name) = binary_metamethod_name(operation) else {
                    return Err(error);
                };

                if binary_primitive_applies(operation, &left, &right) {
                    return Err(error);
                }

                let Some(metamethod) = self.find_binary_metamethod(&left, &right, name)? else {
                    return Err(error);
                };

                Ok(Some(FrameBoundary::Invoke {
                    callee: metamethod,
                    arguments: vec![left, right].into_boxed_slice(),
                    target: ResultTarget::Operator { destination },
                }))
            }
        }
    }

    pub(super) fn resolve_comparison(
        &self,
        operation: BinaryOp,
        left: RawValue,
        right: RawValue,
    ) -> FaultResult<ComparisonOutcome> {
        match operation {
            BinaryOp::Equal => {
                let result = semantics::binary(BinaryOp::Equal, &left, &right)?;
                let RawValue::Boolean(equal) = result else {
                    unreachable!("equality must produce a boolean");
                };
                if equal || !matches!((&left, &right), (RawValue::Table(_), RawValue::Table(_))) {
                    return Ok(ComparisonOutcome::Value(equal));
                }
                let Some(callee) = self.find_binary_metamethod(&left, &right, b"__eq")? else {
                    return Ok(ComparisonOutcome::Value(false));
                };
                Ok(ComparisonOutcome::Invoke {
                    callee,
                    arguments: vec![left, right].into_boxed_slice(),
                })
            }
            BinaryOp::LessThan
            | BinaryOp::LessEqual
            | BinaryOp::GreaterThan
            | BinaryOp::GreaterEqual => match semantics::binary(operation, &left, &right) {
                Ok(RawValue::Boolean(result)) => Ok(ComparisonOutcome::Value(result)),
                Ok(_) => unreachable!("ordering must produce a boolean"),
                Err(error) => {
                    let (name, reversed) = match operation {
                        BinaryOp::LessThan => (b"__lt".as_slice(), false),
                        BinaryOp::LessEqual => (b"__le".as_slice(), false),
                        BinaryOp::GreaterThan => (b"__lt".as_slice(), true),
                        BinaryOp::GreaterEqual => (b"__le".as_slice(), true),
                        _ => unreachable!(),
                    };

                    let (comparison_left, comparison_right) = if reversed {
                        (&right, &left)
                    } else {
                        (&left, &right)
                    };

                    let Some(callee) =
                        self.find_binary_metamethod(comparison_left, comparison_right, name)?
                    else {
                        return Err(error);
                    };

                    let arguments = if reversed {
                        vec![right, left].into_boxed_slice()
                    } else {
                        vec![left, right].into_boxed_slice()
                    };

                    Ok(ComparisonOutcome::Invoke { callee, arguments })
                }
            },
            _ => unreachable!("not a comparison operation"),
        }
    }

    /// Finds a binary metamethod for the given operation by first checking the left value, then the right value.
    fn find_binary_metamethod(
        &self,
        left: &RawValue,
        right: &RawValue,
        name: &'static [u8],
    ) -> FaultResult<Option<RawValue>> {
        let metamethod = self.runtime.metamethod(left, name)?;

        if !metamethod.is_nil() {
            return Ok(Some(metamethod));
        }

        let metamethod = self.runtime.metamethod(right, name)?;

        if !metamethod.is_nil() {
            return Ok(Some(metamethod));
        }

        Ok(None)
    }
}

fn unary_metamethod_name(operation: UnaryOp) -> Option<&'static [u8]> {
    match operation {
        UnaryOp::Negate => Some(b"__unm"),
        UnaryOp::BitwiseNot => Some(b"__bnot"),
        UnaryOp::Not | UnaryOp::Length => None,
    }
}

fn binary_metamethod_name(operation: BinaryOp) -> Option<&'static [u8]> {
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
        BinaryOp::LessThan => Some(b"__lt"),
        BinaryOp::LessEqual => Some(b"__le"),
        BinaryOp::GreaterThan => Some(b"__lt"),
        BinaryOp::GreaterEqual => Some(b"__le"),
        BinaryOp::Concat | BinaryOp::Equal => None,
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

        BinaryOp::LessThan
        | BinaryOp::LessEqual
        | BinaryOp::GreaterThan
        | BinaryOp::GreaterEqual => false,
        BinaryOp::Concat | BinaryOp::Equal => true,
    }
}
