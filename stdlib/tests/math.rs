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
