use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{CallOutcome, LuaString, State, Value, VmErrorKind, VmResult, VmTraceFrame};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn installed_state() -> State {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state
}

fn execute(state: &mut State, source: &str) -> VmResult<Vec<Value>> {
    let function = state.load_buffer("math-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("math test unexpectedly yielded"),
    }
}

#[test]
fn install_registers_math_abs() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&math, &string("abs")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn abs_preserves_integer_values_and_wraps_mininteger() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.abs(7),
                math.abs(-7),
                math.abs(0),
                math.abs(math.mininteger),
                math.abs(math.maxinteger),
                math.abs(-math.maxinteger)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(7),
            Value::Integer(7),
            Value::Integer(0),
            Value::Integer(i64::MIN),
            Value::Integer(i64::MAX),
            Value::Integer(i64::MAX),
        ]
    );
}

#[test]
fn abs_preserves_float_values_and_clears_negative_zero() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.abs(7.0),
                math.abs(-7.5),
                1 / math.abs(-0.0)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(7.0),
            Value::Float(7.5),
            Value::Float(f64::INFINITY),
        ]
    );
}

#[test]
fn abs_coerces_numeric_strings_to_floats() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"return math.abs("-7"), math.abs("-7.5"), math.abs(" -0x10 ")"#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![Value::Float(7.0), Value::Float(7.5), Value::Float(16.0),]
    );
}

#[test]
fn abs_handles_infinities_and_nan() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local nan = math.abs(0 / 0)
            return math.abs(1 / 0), math.abs(-1 / 0), nan ~= nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(f64::INFINITY),
            Value::Float(f64::INFINITY),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn abs_reports_missing_and_non_numeric_arguments() {
    let mut state = installed_state();

    for (argument, actual_type) in [("", "no value"), ("{}", "table"), (r#""nope""#, "string")] {
        let source = format!("return math.abs({argument})");
        let error = execute(&mut state, &source).unwrap_err();

        assert_eq!(
            error.kind,
            VmErrorKind::NativeFunctionFailure {
                message: format!("bad argument #1 to 'abs' (number expected, got {actual_type})")
                    .into(),
            }
        );
        assert!(matches!(
            error.frames.first(),
            Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.abs"
        ));
    }
}

#[test]
fn install_registers_math_ult() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&math, &string("ult")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn ult_compares_integer_bit_patterns_as_unsigned() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.ult(3, 4),
                math.ult(4, 4),
                math.ult(-2, -1),
                math.ult(2, -1),
                math.ult(-2, -2),
                math.ult(math.maxinteger, math.mininteger),
                math.ult(math.mininteger, math.maxinteger)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Boolean(false),
        ]
    );
}

#[test]
fn ult_accepts_exact_integer_floats_and_numeric_strings() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.ult(3.0, 4.0),
                math.ult("3", "4"),
                math.ult("-1", 0),
                math.ult(0, "-1")
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn ult_reports_missing_non_numeric_and_non_integer_arguments() {
    let mut state = installed_state();

    for (source, expected_message) in [
        (
            "return math.ult()",
            "bad argument #1 to 'ult' (number expected, got no value)",
        ),
        (
            "return math.ult(1)",
            "bad argument #2 to 'ult' (number expected, got no value)",
        ),
        (
            "return math.ult({}, 1)",
            "bad argument #1 to 'ult' (number expected, got table)",
        ),
        (
            "return math.ult(1.5, 2)",
            "bad argument #1 to 'ult' (number has no integer representation)",
        ),
        (
            "return math.ult(1, 'nope')",
            "bad argument #2 to 'ult' (number expected, got string)",
        ),
    ] {
        let error = execute(&mut state, source).unwrap_err();

        assert_eq!(
            error.kind,
            VmErrorKind::NativeFunctionFailure {
                message: expected_message.into(),
            }
        );
        assert!(matches!(
            error.frames.first(),
            Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.ult"
        ));
    }
}

#[test]
fn install_registers_math_extrema() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&math, &string("max")).unwrap(),
        Value::Function(_)
    ));
    assert!(matches!(
        state.raw_get(&math, &string("min")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn install_registers_trigonometric_functions() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };

    for name in ["sin", "cos", "tan"] {
        assert!(
            matches!(
                state.raw_get(&math, &string(name)).unwrap(),
                Value::Function(_)
            ),
            "math.{name} was not registered"
        );
    }
}

#[test]
fn huge_is_positive_infinity_and_keeps_its_field_name_in_errors() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };
    assert_eq!(
        state.raw_get(&math, &string("huge")).unwrap(),
        Value::Float(f64::INFINITY)
    );

    let error = execute(&mut state, "return math.huge << 1").unwrap_err();
    assert_eq!(
        error.kind.to_string(),
        "number (field 'huge') has no integer representation"
    );

    let error = execute(&mut state, "return ~math.foo").unwrap_err();
    assert_eq!(
        error.kind.to_string(),
        "attempt to perform bitwise operation on a nil value (field 'foo')"
    );
}

#[test]
fn extrema_return_primitive_values_and_preserve_their_types() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.max(7),
                math.max(-5, 2, 1),
                math.max(1.5, 2, 1.75),
                math.max("alpha", "omega", "beta"),
                math.min(7),
                math.min(-5, 2, 1),
                math.min(1.5, 2, 1.75),
                math.min("alpha", "omega", "beta")
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(7),
            Value::Integer(2),
            Value::Integer(2),
            string("omega"),
            Value::Integer(7),
            Value::Integer(-5),
            Value::Float(1.5),
            string("alpha"),
        ]
    );
}

#[test]
fn max_dispatches_to_lt_in_candidate_order_and_keeps_the_first_tie() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local calls = {}
            local metatable = {
                __lt = function(left, right)
                    calls[#calls + 1] = { left = left, right = right }
                    return left.rank < right.rank
                end,
            }

            local first = setmetatable({ rank = 1 }, metatable)
            local largest = setmetatable({ rank = 3 }, metatable)
            local last = setmetatable({ rank = 2 }, metatable)
            local tied = setmetatable({ rank = 3 }, metatable)

            local maximum = math.max(first, largest, last, tied)

            return
                maximum == largest,
                #calls,
                calls[1].left == first and calls[1].right == largest,
                calls[2].left == largest and calls[2].right == last,
                calls[3].left == largest and calls[3].right == tied
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Integer(3),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn max_uses_the_right_lt_metamethod_and_booleanizes_its_result() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local left = setmetatable({}, {})
            local right
            local called_with_original_operands = false

            right = setmetatable({}, {
                __lt = function(actual_left, actual_right)
                    called_with_original_operands =
                        actual_left == left and actual_right == right
                    return "truthy"
                end,
            })

            return math.max(left, right) == right, called_with_original_operands
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Boolean(true), Value::Boolean(true)]);
}

#[test]
fn min_dispatches_to_lt_with_the_candidate_first_and_keeps_the_first_tie() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local calls = {}
            local metatable = {
                __lt = function(left, right)
                    calls[#calls + 1] = { left = left, right = right }
                    return left.rank < right.rank
                end,
            }

            local first = setmetatable({ rank = 3 }, metatable)
            local smallest = setmetatable({ rank = 1 }, metatable)
            local last = setmetatable({ rank = 2 }, metatable)
            local tied = setmetatable({ rank = 1 }, metatable)

            local minimum = math.min(first, smallest, last, tied)

            return
                minimum == smallest,
                #calls,
                calls[1].left == smallest and calls[1].right == first,
                calls[2].left == last and calls[2].right == smallest,
                calls[3].left == tied and calls[3].right == smallest
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Integer(3),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn extrema_report_missing_and_incomparable_arguments() {
    let mut state = installed_state();

    let missing = execute(&mut state, "return math.max()").unwrap_err();
    assert_eq!(
        missing.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'max' (value expected)".into(),
        }
    );
    assert!(matches!(
        missing.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.max"
    ));

    let incomparable = execute(&mut state, "return math.max(1, false)").unwrap_err();
    assert!(matches!(
        incomparable.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: "<",
            left: "number",
            right: "boolean",
        }
    ));
    assert!(incomparable.frames.iter().any(
        |frame| matches!(frame, VmTraceFrame::Native { name } if name.as_ref() == "math.max")
    ));

    let missing = execute(&mut state, "return math.min()").unwrap_err();
    assert_eq!(
        missing.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'min' (value expected)".into(),
        }
    );
    assert!(matches!(
        missing.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.min"
    ));

    let incomparable = execute(&mut state, "return math.min(1, false)").unwrap_err();
    assert!(matches!(
        incomparable.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: "<",
            left: "boolean",
            right: "number",
        }
    ));
    assert!(incomparable.frames.iter().any(
        |frame| matches!(frame, VmTraceFrame::Native { name } if name.as_ref() == "math.min")
    ));
}

#[test]
fn tointeger_converts_exact_numbers_and_numeric_strings() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.tointeger(42),
                math.tointeger(42.0),
                math.tointeger("42"),
                math.tointeger("42.0"),
                math.tointeger(math.mininteger),
                math.tointeger("-9223372036854775808"),
                math.tointeger(math.maxinteger),
                math.tointeger("9223372036854775807")
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(42),
            Value::Integer(42),
            Value::Integer(42),
            Value::Integer(42),
            Value::Integer(i64::MIN),
            Value::Integer(i64::MIN),
            Value::Integer(i64::MAX),
            Value::Integer(i64::MAX),
        ],
    );
}

#[test]
fn tointeger_returns_nil_for_values_without_an_integer_representation() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.tointeger(34.3),
                math.tointeger("34.3"),
                math.tointeger("not a number"),
                math.tointeger({}),
                math.tointeger(0 / 0),
                math.tointeger(1 / 0),
                math.tointeger(-1 / 0),
                math.tointeger(0.0 - math.mininteger)
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Nil; 8]);
}

#[test]
fn tointeger_reports_a_missing_value() {
    let mut state = installed_state();
    let missing = execute(&mut state, "return math.tointeger()").unwrap_err();

    assert_eq!(
        missing.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'tointeger' (value expected)".into(),
        }
    );
    assert!(matches!(
        missing.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.tointeger"
    ));
}

#[test]
fn fmod_returns_truncating_integer_remainder_for_integer_arguments() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.fmod(10, 3),
                math.fmod(-5, 3),
                math.fmod(5, -3),
                math.fmod(-5, -3),
                math.fmod(0, 7),
                math.fmod(7, 1),
                math.fmod(7, -1),
                math.fmod(math.mininteger, math.mininteger),
                math.fmod(math.maxinteger, math.maxinteger),
                math.fmod(math.mininteger + 1, math.mininteger),
                math.fmod(math.maxinteger - 1, math.maxinteger)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(1),
            Value::Integer(-2),
            Value::Integer(2),
            Value::Integer(-2),
            Value::Integer(0),
            Value::Integer(0),
            Value::Integer(0),
            Value::Integer(0),
            Value::Integer(0),
            Value::Integer(i64::MIN + 1),
            Value::Integer(i64::MAX - 1),
        ]
    );
}

#[test]
fn fmod_returns_float_remainder_when_any_argument_is_a_float() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.fmod(10.0, 3),
                math.fmod(10, 3.0),
                math.fmod(-5.5, 2),
                math.fmod(5.5, -2),
                math.fmod(1.0, 1 / 0)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1.0),
            Value::Float(1.0),
            Value::Float(-1.5),
            Value::Float(1.5),
            Value::Float(1.0),
        ]
    );
}

#[test]
fn fmod_returns_nan_for_indeterminate_float_operations() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local by_zero = math.fmod(3.0, 0.0)
            local nan_dividend = math.fmod(0.0 / 0.0, 1)
            return by_zero ~= by_zero, nan_dividend ~= nan_dividend
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Boolean(true), Value::Boolean(true)]);
}

#[test]
fn fmod_coerces_numeric_strings_preserving_int_or_float_kind() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.fmod("7", "2"),
                math.fmod("10", 3),
                math.fmod(10, "4"),
                math.fmod("7.5", "2"),
                math.fmod(10, "4.0")
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(2),
            Value::Float(1.5),
            Value::Float(2.0),
        ]
    );
}

#[test]
fn fmod_reports_missing_and_non_numeric_arguments() {
    let mut state = installed_state();

    let missing_dividend = execute(&mut state, "return math.fmod()").unwrap_err();
    assert_eq!(
        missing_dividend.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'fmod' (number expected, got no value)".into(),
        }
    );
    assert!(matches!(
        missing_dividend.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.fmod"
    ));

    let missing_divisor = execute(&mut state, "return math.fmod(1)").unwrap_err();
    assert_eq!(
        missing_divisor.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #2 to 'fmod' (number expected, got no value)".into(),
        }
    );

    let table_dividend = execute(&mut state, "return math.fmod({}, 2)").unwrap_err();
    assert_eq!(
        table_dividend.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'fmod' (number expected, got table)".into(),
        }
    );

    let string_divisor = execute(&mut state, "return math.fmod(1, 'nope')").unwrap_err();
    assert_eq!(
        string_divisor.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #2 to 'fmod' (number expected, got string)".into(),
        }
    );
}

#[test]
fn fmod_rejects_an_integer_zero_divisor() {
    let mut state = installed_state();

    let error = execute(&mut state, "return math.fmod(3, 0)").unwrap_err();
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #2 to 'fmod' (zero)".into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.fmod"
    ));
}

#[test]
fn fmod_handles_mininteger_remainder_minus_one_without_overflow() {
    let mut state = installed_state();

    let values = execute(&mut state, "return math.fmod(math.mininteger, -1)").unwrap();
    assert_eq!(values, vec![Value::Integer(0)]);
}

#[test]
fn install_registers_math_floor() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&math, &string("floor")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn floor_passes_integers_through_unchanged() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.floor(7),
                math.floor(-7),
                math.floor(0),
                math.floor(math.mininteger),
                math.floor(math.maxinteger)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(7),
            Value::Integer(-7),
            Value::Integer(0),
            Value::Integer(i64::MIN),
            Value::Integer(i64::MAX),
        ]
    );
}

#[test]
fn floor_rounds_floats_down_and_returns_integers_when_representable() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.floor(3.4),
                math.floor(3.7),
                math.floor(-3.4),
                math.floor(3.0),
                math.floor(0.5),
                math.floor(-0.5),
                math.floor(math.mininteger + 0.0)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(3),
            Value::Integer(3),
            Value::Integer(-4),
            Value::Integer(3),
            Value::Integer(0),
            Value::Integer(-1),
            Value::Integer(i64::MIN),
        ]
    );
}

#[test]
fn floor_keeps_unrepresentable_and_non_finite_floats_as_floats() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local nan = math.floor(0 / 0)
            return
                math.floor(1e50),
                math.floor(-1e50),
                math.floor(9223372036854775808.0),
                math.floor(1 / 0),
                math.floor(-1 / 0),
                nan ~= nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1e50),
            Value::Float(-1e50),
            Value::Float(9223372036854775808.0),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn floor_coerces_numeric_strings() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.floor("3.7"),
                math.floor("-3.7"),
                math.floor("7"),
                math.floor("1e50")
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(3),
            Value::Integer(-4),
            Value::Integer(7),
            Value::Float(1e50),
        ]
    );
}

#[test]
fn floor_reports_missing_and_non_numeric_arguments() {
    let mut state = installed_state();

    let missing = execute(&mut state, "return math.floor()").unwrap_err();
    assert_eq!(
        missing.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'floor' (number expected, got no value)".into(),
        }
    );
    assert!(matches!(
        missing.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.floor"
    ));

    let table = execute(&mut state, "return math.floor({})").unwrap_err();
    assert_eq!(
        table.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'floor' (number expected, got table)".into(),
        }
    );

    let string = execute(&mut state, "return math.floor('nope')").unwrap_err();
    assert_eq!(
        string.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'floor' (number expected, got string)".into(),
        }
    );
}

#[test]
fn trigonometric_functions_use_radians_and_return_floats() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.sin(0), math.sin(1), math.sin(-1),
                math.cos(0), math.cos(1), math.cos(-1),
                math.tan(0), math.tan(1), math.tan(-1)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(0.0_f64.sin()),
            Value::Float(1.0_f64.sin()),
            Value::Float((-1.0_f64).sin()),
            Value::Float(0.0_f64.cos()),
            Value::Float(1.0_f64.cos()),
            Value::Float((-1.0_f64).cos()),
            Value::Float(0.0_f64.tan()),
            Value::Float(1.0_f64.tan()),
            Value::Float((-1.0_f64).tan()),
        ]
    );
}

#[test]
fn trigonometric_functions_coerce_numeric_strings() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"return math.sin("1"), math.cos("-1"), math.tan("0.5")"#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1.0_f64.sin()),
            Value::Float((-1.0_f64).cos()),
            Value::Float(0.5_f64.tan()),
        ]
    );
}

#[test]
fn trigonometric_functions_return_nan_for_infinite_arguments() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local sin = math.sin(1 / 0)
            local cos = math.cos(1 / 0)
            local tan = math.tan(1 / 0)
            return sin ~= sin, cos ~= cos, tan ~= tan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn trigonometric_functions_report_missing_and_non_numeric_arguments() {
    let mut state = installed_state();

    for function in ["sin", "cos", "tan"] {
        for (argument, actual_type) in [("", "no value"), ("{}", "table"), (r#""nope""#, "string")]
        {
            let source = format!("return math.{function}({argument})");
            let error = execute(&mut state, &source).unwrap_err();

            assert_eq!(
                error.kind,
                VmErrorKind::NativeFunctionFailure {
                    message: format!(
                        "bad argument #1 to '{function}' (number expected, got {actual_type})"
                    )
                    .into(),
                }
            );

            let native_name = format!("math.{function}");
            assert!(
                matches!(
                    error.frames.first(),
                    Some(VmTraceFrame::Native { name }) if name.as_ref() == native_name
                ),
                "expected the first traceback frame to be {native_name:?}, got {:?}",
                error.frames.first()
            );
        }
    }
}

#[test]
fn install_registers_the_remaining_math_functions_and_pi() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };

    for name in [
        "ceil", "asin", "acos", "atan", "deg", "rad", "sqrt", "exp", "log", "type", "modf",
    ] {
        assert!(
            matches!(
                state.raw_get(&math, &string(name)).unwrap(),
                Value::Function(_)
            ),
            "math.{name} was not registered"
        );
    }

    assert_eq!(
        state.raw_get(&math, &string("pi")).unwrap(),
        Value::Float(std::f64::consts::PI)
    );
}

#[test]
fn ceil_preserves_integers_and_rounds_floats_up() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.ceil(7),
                math.ceil(math.mininteger),
                math.ceil(math.maxinteger),
                math.ceil(3.4),
                math.ceil(-3.4),
                math.ceil(3.0),
                math.ceil(0.5),
                math.ceil(-0.5),
                math.ceil(math.mininteger + 0.0)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(7),
            Value::Integer(i64::MIN),
            Value::Integer(i64::MAX),
            Value::Integer(4),
            Value::Integer(-3),
            Value::Integer(3),
            Value::Integer(1),
            Value::Integer(0),
            Value::Integer(i64::MIN),
        ]
    );
}

#[test]
fn ceil_keeps_unrepresentable_and_non_finite_floats_as_floats() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local nan = math.ceil(0 / 0)
            return
                math.ceil(1e50),
                math.ceil(-1e50),
                math.ceil(9223372036854775808.0),
                math.ceil(1 / 0),
                math.ceil(-1 / 0),
                nan ~= nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1e50),
            Value::Float(-1e50),
            Value::Float(9223372036854775808.0),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn ceil_coerces_numeric_strings() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"return math.ceil("3.4"), math.ceil("-3.4"), math.ceil("7"), math.ceil("1e50")"#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(4),
            Value::Integer(-3),
            Value::Integer(7),
            Value::Float(1e50),
        ]
    );
}

#[test]
fn ceil_reports_missing_and_non_numeric_arguments() {
    let mut state = installed_state();

    for (argument, actual_type) in [("", "no value"), ("{}", "table"), (r#""nope""#, "string")] {
        let source = format!("return math.ceil({argument})");
        let error = execute(&mut state, &source).unwrap_err();

        assert_eq!(
            error.kind,
            VmErrorKind::NativeFunctionFailure {
                message: format!("bad argument #1 to 'ceil' (number expected, got {actual_type})")
                    .into(),
            }
        );
        assert!(matches!(
            error.frames.first(),
            Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.ceil"
        ));
    }
}

#[test]
fn inverse_trigonometric_functions_return_floats_in_radians() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.asin(-1), math.asin(0), math.asin(1),
                math.acos(-1), math.acos(0), math.acos(1)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float((-1.0_f64).asin()),
            Value::Float(0.0_f64.asin()),
            Value::Float(1.0_f64.asin()),
            Value::Float((-1.0_f64).acos()),
            Value::Float(0.0_f64.acos()),
            Value::Float(1.0_f64.acos()),
        ]
    );
}

#[test]
fn atan_uses_atan2_quadrants_and_defaults_its_second_argument_to_one() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.atan(1),
                math.atan(1, nil),
                math.atan(1, 0),
                math.atan(-1, 0),
                math.atan(1, -1),
                math.atan(-1, -1),
                math.atan(0, -1),
                math.atan(1 / 0, 1 / 0)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1.0_f64.atan2(1.0)),
            Value::Float(1.0_f64.atan2(1.0)),
            Value::Float(1.0_f64.atan2(0.0)),
            Value::Float((-1.0_f64).atan2(0.0)),
            Value::Float(1.0_f64.atan2(-1.0)),
            Value::Float((-1.0_f64).atan2(-1.0)),
            Value::Float(0.0_f64.atan2(-1.0)),
            Value::Float(f64::INFINITY.atan2(f64::INFINITY)),
        ]
    );
}

#[test]
fn inverse_trigonometric_functions_coerce_numeric_strings() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"return math.asin("0.5"), math.acos("-0.5"), math.atan("1", "-1")"#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(0.5_f64.asin()),
            Value::Float((-0.5_f64).acos()),
            Value::Float(1.0_f64.atan2(-1.0)),
        ]
    );
}

#[test]
fn asin_and_acos_return_nan_outside_their_domains() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local asin_high = math.asin(2)
            local asin_low = math.asin(-2)
            local acos_high = math.acos(2)
            local acos_infinite = math.acos(1 / 0)
            return
                asin_high ~= asin_high,
                asin_low ~= asin_low,
                acos_high ~= acos_high,
                acos_infinite ~= acos_infinite
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Boolean(true); 4]);
}

#[test]
fn inverse_trigonometric_functions_report_argument_errors() {
    let mut state = installed_state();

    for function in ["asin", "acos", "atan"] {
        for (argument, actual_type) in [("", "no value"), ("{}", "table"), (r#""nope""#, "string")]
        {
            let source = format!("return math.{function}({argument})");
            let error = execute(&mut state, &source).unwrap_err();

            assert_eq!(
                error.kind,
                VmErrorKind::NativeFunctionFailure {
                    message: format!(
                        "bad argument #1 to '{function}' (number expected, got {actual_type})"
                    )
                    .into(),
                }
            );
        }
    }

    for (argument, actual_type) in [("{}", "table"), (r#""nope""#, "string")] {
        let source = format!("return math.atan(1, {argument})");
        let error = execute(&mut state, &source).unwrap_err();

        assert_eq!(
            error.kind,
            VmErrorKind::NativeFunctionFailure {
                message: format!("bad argument #2 to 'atan' (number expected, got {actual_type})")
                    .into(),
            }
        );
        assert!(matches!(
            error.frames.first(),
            Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.atan"
        ));
    }
}

#[test]
fn degree_and_radian_conversions_return_floats() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.deg(math.pi),
                math.deg(-math.pi / 2),
                math.rad(180),
                math.rad(-90)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(std::f64::consts::PI.to_degrees()),
            Value::Float((-std::f64::consts::FRAC_PI_2).to_degrees()),
            Value::Float(180.0_f64.to_radians()),
            Value::Float((-90.0_f64).to_radians()),
        ]
    );
}

#[test]
fn degree_and_radian_conversions_coerce_strings_and_handle_non_finite_values() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local deg_nan = math.deg(0 / 0)
            local rad_nan = math.rad(0 / 0)
            return
                math.deg("1"),
                math.rad("90"),
                math.deg(1 / 0),
                math.rad(-1 / 0),
                deg_nan ~= deg_nan,
                rad_nan ~= rad_nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1.0_f64.to_degrees()),
            Value::Float(90.0_f64.to_radians()),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn degree_and_radian_conversions_report_argument_errors() {
    let mut state = installed_state();

    for function in ["deg", "rad"] {
        for (argument, actual_type) in [("", "no value"), ("{}", "table"), (r#""nope""#, "string")]
        {
            let source = format!("return math.{function}({argument})");
            let error = execute(&mut state, &source).unwrap_err();

            assert_eq!(
                error.kind,
                VmErrorKind::NativeFunctionFailure {
                    message: format!(
                        "bad argument #1 to '{function}' (number expected, got {actual_type})"
                    )
                    .into(),
                }
            );

            let native_name = format!("math.{function}");
            assert!(matches!(
                error.frames.first(),
                Some(VmTraceFrame::Native { name }) if name.as_ref() == native_name
            ));
        }
    }
}

#[test]
fn sqrt_returns_floats_and_handles_domain_edges() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local negative = math.sqrt(-1)
            local nan = math.sqrt(0 / 0)
            return
                math.sqrt(0),
                math.sqrt(4),
                math.sqrt(2),
                math.sqrt("9"),
                math.sqrt(1 / 0),
                1 / math.sqrt(-0.0),
                negative ~= negative,
                nan ~= nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(0.0),
            Value::Float(2.0),
            Value::Float(2.0_f64.sqrt()),
            Value::Float(3.0),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn exp_returns_floats_and_handles_overflow_and_non_finite_values() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local nan = math.exp(0 / 0)
            return
                math.exp(0),
                math.exp(1),
                math.exp(-1),
                math.exp("2"),
                math.exp(1000),
                math.exp(1 / 0),
                math.exp(-1 / 0),
                nan ~= nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1.0),
            Value::Float(1.0_f64.exp()),
            Value::Float((-1.0_f64).exp()),
            Value::Float(2.0_f64.exp()),
            Value::Float(f64::INFINITY),
            Value::Float(f64::INFINITY),
            Value::Float(0.0),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn log_supports_natural_and_explicit_bases() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.log(1),
                math.log(math.exp(1)),
                math.log(8, 2),
                math.log(100, 10),
                math.log(9, 3),
                math.log(8, nil),
                math.log("8", "2")
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(1.0_f64.ln()),
            Value::Float(1.0_f64.exp().ln()),
            Value::Float(8.0_f64.log(2.0)),
            Value::Float(100.0_f64.log(10.0)),
            Value::Float(9.0_f64.log(3.0)),
            Value::Float(8.0_f64.ln()),
            Value::Float(8.0_f64.log(2.0)),
        ]
    );
}

#[test]
fn log_handles_zero_negative_and_non_finite_inputs() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local negative = math.log(-1)
            local nan = math.log(0 / 0)
            return
                math.log(0),
                math.log(1 / 0),
                math.log(2, 0.5),
                negative ~= negative,
                nan ~= nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(f64::NEG_INFINITY),
            Value::Float(f64::INFINITY),
            Value::Float(2.0_f64.log(0.5)),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn sqrt_exp_and_log_report_first_argument_errors() {
    let mut state = installed_state();

    for function in ["sqrt", "exp", "log"] {
        for (argument, actual_type) in [("", "no value"), ("{}", "table"), (r#""nope""#, "string")]
        {
            let source = format!("return math.{function}({argument})");
            let error = execute(&mut state, &source).unwrap_err();

            assert_eq!(
                error.kind,
                VmErrorKind::NativeFunctionFailure {
                    message: format!(
                        "bad argument #1 to '{function}' (number expected, got {actual_type})"
                    )
                    .into(),
                }
            );

            let native_name = format!("math.{function}");
            assert!(matches!(
                error.frames.first(),
                Some(VmTraceFrame::Native { name }) if name.as_ref() == native_name
            ));
        }
    }
}

#[test]
fn log_reports_non_numeric_second_arguments() {
    let mut state = installed_state();

    for (argument, actual_type) in [("{}", "table"), (r#""nope""#, "string")] {
        let source = format!("return math.log(2, {argument})");
        let error = execute(&mut state, &source).unwrap_err();

        assert_eq!(
            error.kind,
            VmErrorKind::NativeFunctionFailure {
                message: format!("bad argument #2 to 'log' (number expected, got {actual_type})")
                    .into(),
            }
        );
        assert!(matches!(
            error.frames.first(),
            Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.log"
        ));
    }
}

#[test]
fn math_type_distinguishes_integer_and_float_values() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.type(0),
                math.type(0.0),
                math.type(math.mininteger),
                math.type(1 / 2),
                math.type("10"),
                math.type({}),
                math.type(false),
                math.type(nil)
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            string("integer"),
            string("float"),
            string("integer"),
            string("float"),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ]
    );
}

#[test]
fn math_type_reports_a_missing_value() {
    let mut state = installed_state();
    let error = execute(&mut state, "return math.type()").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'type' (value expected)".into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.type"
    ));
}

#[test]
fn modf_splits_floats_toward_zero_and_returns_a_float_fraction() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local positive_integer, positive_fraction = math.modf(3.5)
            local negative_integer, negative_fraction = math.modf(-2.5)
            local exact_integer, exact_fraction = math.modf(-3.0)
            return
                positive_integer, positive_fraction,
                negative_integer, negative_fraction,
                exact_integer, 1 / exact_fraction
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(3),
            Value::Float(0.5),
            Value::Integer(-2),
            Value::Float(-0.5),
            Value::Integer(-3),
            Value::Float(f64::INFINITY),
        ]
    );
}

#[test]
fn modf_preserves_integer_arguments_including_their_full_range() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local small, small_fraction = math.modf(3)
            local minimum, minimum_fraction = math.modf(math.mininteger)
            local maximum, maximum_fraction = math.modf(math.maxinteger)
            return
                small, small_fraction,
                minimum, minimum_fraction,
                maximum, maximum_fraction
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(3),
            Value::Float(0.0),
            Value::Integer(i64::MIN),
            Value::Float(0.0),
            Value::Integer(i64::MAX),
            Value::Float(0.0),
        ]
    );
}

#[test]
fn modf_handles_large_non_finite_and_nan_floats() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local negative_large, negative_large_fraction = math.modf(-3e23)
            local positive_large, positive_large_fraction = math.modf(3e35)
            local negative_infinite, negative_infinite_fraction = math.modf(-1 / 0)
            local positive_infinite, positive_infinite_fraction = math.modf(1 / 0)
            local nan_integer, nan_fraction = math.modf(0 / 0)
            return
                negative_large, negative_large_fraction,
                positive_large, positive_large_fraction,
                negative_infinite, negative_infinite_fraction,
                positive_infinite, positive_infinite_fraction,
                nan_integer ~= nan_integer,
                nan_fraction ~= nan_fraction
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Float(-3e23),
            Value::Float(0.0),
            Value::Float(3e35),
            Value::Float(0.0),
            Value::Float(f64::NEG_INFINITY),
            Value::Float(0.0),
            Value::Float(f64::INFINITY),
            Value::Float(0.0),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn modf_coerces_numeric_strings() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local positive_integer, positive_fraction = math.modf("3.5")
            local negative_integer, negative_fraction = math.modf("-2.5")
            local exact_integer, exact_fraction = math.modf("3")
            return
                positive_integer, positive_fraction,
                negative_integer, negative_fraction,
                exact_integer, exact_fraction
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(3),
            Value::Float(0.5),
            Value::Integer(-2),
            Value::Float(-0.5),
            Value::Integer(3),
            Value::Float(0.0),
        ]
    );
}

#[test]
fn modf_reports_missing_and_non_numeric_arguments() {
    let mut state = installed_state();

    for (argument, actual_type) in [("", "no value"), ("{}", "table"), (r#""nope""#, "string")] {
        let source = format!("return math.modf({argument})");
        let error = execute(&mut state, &source).unwrap_err();

        assert_eq!(
            error.kind,
            VmErrorKind::NativeFunctionFailure {
                message: format!("bad argument #1 to 'modf' (number expected, got {actual_type})")
                    .into(),
            }
        );
        assert!(matches!(
            error.frames.first(),
            Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.modf"
        ));
    }
}
