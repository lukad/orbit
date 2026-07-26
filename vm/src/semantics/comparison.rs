use crate::{
    error::{FaultResult, VmErrorKind},
    number::{
        float_less_equal_integer, float_less_integer, float_to_integer, integer_less_equal_float,
        integer_less_float,
    },
    value::RawValue,
};

pub(super) fn equal(left: &RawValue, right: &RawValue) -> bool {
    match (left, right) {
        (RawValue::Nil, RawValue::Nil) => true,
        (RawValue::Boolean(left), RawValue::Boolean(right)) => left == right,
        (RawValue::Integer(left), RawValue::Integer(right)) => left == right,
        (RawValue::Float(left), RawValue::Float(right)) => left == right,
        (RawValue::Integer(left), RawValue::Float(right)) => {
            float_to_integer(*right) == Some(*left)
        }
        (RawValue::Float(left), RawValue::Integer(right)) => {
            float_to_integer(*left) == Some(*right)
        }
        (RawValue::String(left), RawValue::String(right)) => left == right,
        (RawValue::Table(left), RawValue::Table(right)) => left == right,
        (RawValue::Function(left), RawValue::Function(right)) => left == right,
        (RawValue::LightUserdata(left), RawValue::LightUserdata(right)) => left == right,
        _ => false,
    }
}

pub(super) fn less_than(left: &RawValue, right: &RawValue) -> FaultResult<bool> {
    ordered("<", left, right, raw_less_than)
}

pub(super) fn less_equal(left: &RawValue, right: &RawValue) -> FaultResult<bool> {
    ordered("<=", left, right, raw_less_equal)
}

pub(super) fn greater_than(left: &RawValue, right: &RawValue) -> FaultResult<bool> {
    ordered(">", left, right, |left, right| raw_less_than(right, left))
}

pub(super) fn greater_equal(left: &RawValue, right: &RawValue) -> FaultResult<bool> {
    ordered(">=", left, right, |left, right| raw_less_equal(right, left))
}

fn ordered(
    operation: &'static str,
    left: &RawValue,
    right: &RawValue,
    compare: impl FnOnce(&RawValue, &RawValue) -> Option<bool>,
) -> FaultResult<bool> {
    compare(left, right).ok_or(VmErrorKind::InvalidComparisonOperands {
        operation,
        left: left.type_name(),
        right: right.type_name(),
    })
}

fn raw_less_than(left: &RawValue, right: &RawValue) -> Option<bool> {
    match (left, right) {
        (RawValue::Integer(left), RawValue::Integer(right)) => Some(left < right),
        (RawValue::Float(left), RawValue::Float(right)) => Some(left < right),
        (RawValue::Integer(left), RawValue::Float(right)) => {
            Some(integer_less_float(*left, *right))
        }
        (RawValue::Float(left), RawValue::Integer(right)) => {
            Some(float_less_integer(*left, *right))
        }
        (RawValue::String(left), RawValue::String(right)) => {
            Some(left.as_bytes() < right.as_bytes())
        }
        _ => None,
    }
}

fn raw_less_equal(left: &RawValue, right: &RawValue) -> Option<bool> {
    match (left, right) {
        (RawValue::Integer(left), RawValue::Integer(right)) => Some(left <= right),
        (RawValue::Float(left), RawValue::Float(right)) => Some(left <= right),
        (RawValue::Integer(left), RawValue::Float(right)) => {
            Some(integer_less_equal_float(*left, *right))
        }
        (RawValue::Float(left), RawValue::Integer(right)) => {
            Some(float_less_equal_integer(*left, *right))
        }
        (RawValue::String(left), RawValue::String(right)) => {
            Some(left.as_bytes() <= right.as_bytes())
        }
        _ => None,
    }
}
