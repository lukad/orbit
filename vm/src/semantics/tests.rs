use orbit_compiler::bytecode::{BinaryOp, UnaryOp};

use crate::{error::VmErrorKind, string::LuaString, value::RawValue};

use super::{
    binary,
    loops::{advance_numeric_for, prepare_numeric_for},
    unary,
};

#[test]
fn unary_operations_follow_lua_rules() {
    assert_eq!(
        unary(UnaryOp::Not, &RawValue::Nil).unwrap(),
        RawValue::Boolean(true)
    );

    assert_eq!(
        unary(UnaryOp::Not, &RawValue::Integer(0),).unwrap(),
        RawValue::Boolean(false)
    );

    assert_eq!(
        unary(UnaryOp::Negate, &RawValue::Integer(i64::MIN),).unwrap(),
        RawValue::Integer(i64::MIN)
    );

    assert_eq!(
        unary(
            UnaryOp::Length,
            &RawValue::String(LuaString::from("orbit"),),
        )
        .unwrap(),
        RawValue::Integer(5)
    );
}

#[test]
fn integer_arithmetic_wraps() {
    assert_eq!(
        binary(
            BinaryOp::Add,
            &RawValue::Integer(i64::MAX),
            &RawValue::Integer(1),
        )
        .unwrap(),
        RawValue::Integer(i64::MIN)
    );

    assert_eq!(
        binary(
            BinaryOp::Multiply,
            &RawValue::Integer(i64::MAX),
            &RawValue::Integer(2),
        )
        .unwrap(),
        RawValue::Integer(-2)
    );
}

#[test]
fn mixed_arithmetic_produces_floats() {
    assert_eq!(
        binary(BinaryOp::Add, &RawValue::Integer(2), &RawValue::Float(0.5),).unwrap(),
        RawValue::Float(2.5)
    );

    assert_eq!(
        binary(
            BinaryOp::Divide,
            &RawValue::Integer(7),
            &RawValue::Integer(2),
        )
        .unwrap(),
        RawValue::Float(3.5)
    );
}

#[test]
fn floor_division_and_modulo_use_lua_rounding() {
    assert_eq!(
        binary(
            BinaryOp::FloorDivide,
            &RawValue::Integer(-7),
            &RawValue::Integer(3),
        )
        .unwrap(),
        RawValue::Integer(-3)
    );

    assert_eq!(
        binary(
            BinaryOp::Modulo,
            &RawValue::Integer(-7),
            &RawValue::Integer(3),
        )
        .unwrap(),
        RawValue::Integer(2)
    );
}

#[test]
fn integer_division_by_zero_is_an_error() {
    assert_eq!(
        binary(
            BinaryOp::FloorDivide,
            &RawValue::Integer(1),
            &RawValue::Integer(0),
        )
        .unwrap_err(),
        VmErrorKind::IntegerDivisionByZero
    );

    assert_eq!(
        binary(
            BinaryOp::Modulo,
            &RawValue::Integer(1),
            &RawValue::Integer(0),
        )
        .unwrap_err(),
        VmErrorKind::IntegerModuloByZero
    );
}

#[test]
fn bitwise_operations_accept_exact_floats() {
    assert_eq!(
        binary(
            BinaryOp::BitwiseAnd,
            &RawValue::Float(3.0),
            &RawValue::Integer(1),
        )
        .unwrap(),
        RawValue::Integer(1)
    );

    assert!(matches!(
        binary(
            BinaryOp::BitwiseAnd,
            &RawValue::Float(3.5),
            &RawValue::Integer(1),
        ),
        Err(VmErrorKind::NoIntegerRepresentation { .. })
    ));
}

#[test]
fn concatenates_strings_and_numbers() {
    assert_eq!(
        binary(
            BinaryOp::Concat,
            &RawValue::String(LuaString::from("orbit"),),
            &RawValue::Integer(42),
        )
        .unwrap(),
        RawValue::String(LuaString::from("orbit42"),)
    );

    assert_eq!(
        binary(
            BinaryOp::Concat,
            &RawValue::Integer(1),
            &RawValue::Float(2.5),
        )
        .unwrap(),
        RawValue::String(LuaString::from("12.5"),)
    );
}

#[test]
fn mixed_numeric_comparison_is_exact() {
    assert_eq!(
        binary(
            BinaryOp::Equal,
            &RawValue::Integer(1),
            &RawValue::Float(1.0),
        )
        .unwrap(),
        RawValue::Boolean(true)
    );

    assert_eq!(
        binary(
            BinaryOp::LessThan,
            &RawValue::Integer(i64::MAX),
            &RawValue::Float(-(i64::MIN as f64),),
        )
        .unwrap(),
        RawValue::Boolean(true)
    );
}

#[test]
fn prepares_and_advances_integer_for_loops() {
    let preparation = prepare_numeric_for(
        &RawValue::Integer(1),
        &RawValue::Integer(3),
        &RawValue::Integer(1),
    )
    .unwrap();

    assert_eq!(
        preparation.into_parts(),
        (
            RawValue::Integer(1),
            RawValue::Integer(3),
            RawValue::Integer(1),
            Some(RawValue::Integer(1)),
        )
    );

    let advance = advance_numeric_for(
        &RawValue::Integer(1),
        &RawValue::Integer(3),
        &RawValue::Integer(1),
    )
    .unwrap();

    assert_eq!(
        advance.into_parts(),
        (RawValue::Integer(2), Some(RawValue::Integer(2)),)
    );
}

#[test]
fn integer_for_overflow_ends_the_loop() {
    let advance = advance_numeric_for(
        &RawValue::Integer(i64::MAX),
        &RawValue::Integer(i64::MAX),
        &RawValue::Integer(1),
    )
    .unwrap();

    assert_eq!(advance.into_parts(), (RawValue::Integer(i64::MAX), None));
}
