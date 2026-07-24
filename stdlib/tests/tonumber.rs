use orbit_stdlib::install;
use orbit_vm::{
    CallOutcome, LuaString, NoLoadService, State, Value, VmError, VmErrorKind, VmResult,
    VmTraceFrame,
};

fn string(value: impl AsRef<[u8]>) -> Value {
    Value::String(LuaString::new(value.as_ref()))
}

fn installed_state() -> State {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();
    state
}

fn call_tonumber(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Function(tonumber) = state.get_global(b"tonumber")? else {
        panic!("tonumber was not installed as a function");
    };

    match state.call(&tonumber, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("tonumber unexpectedly yielded"),
    }
}

fn assert_tonumber_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "tonumber"
    ));
}

#[test]
fn numbers_are_returned_without_changing_their_numeric_type() {
    let mut state = installed_state();

    for value in [Value::Integer(42), Value::Float(42.0), Value::Float(1.5)] {
        assert_eq!(
            call_tonumber(&mut state, std::slice::from_ref(&value)).unwrap(),
            vec![value],
        );
    }
}

#[test]
fn decimal_strings_convert_to_integers_or_floats() {
    let mut state = installed_state();

    for (source, expected) in [
        ("42", Value::Integer(42)),
        (" \t-42\n", Value::Integer(-42)),
        ("12.5", Value::Float(12.5)),
        ("1e2", Value::Float(100.0)),
    ] {
        assert_eq!(
            call_tonumber(&mut state, &[string(source)]).unwrap(),
            vec![expected],
        );
    }
}

#[test]
fn hexadecimal_strings_convert_to_integers_or_floats() {
    let mut state = installed_state();

    for (source, expected) in [
        ("0x10", Value::Integer(16)),
        ("-0xffffffffffffffff", Value::Integer(1)),
        ("0x1.8p1", Value::Float(3.0)),
    ] {
        assert_eq!(
            call_tonumber(&mut state, &[string(source)]).unwrap(),
            vec![expected],
        );
    }
}

#[test]
fn values_without_a_numeric_conversion_return_nil() {
    let mut state = installed_state();

    for value in [
        Value::Nil,
        Value::Boolean(false),
        Value::Boolean(true),
        string(""),
        string("not a number"),
        string("12 trailing"),
    ] {
        assert_eq!(
            call_tonumber(&mut state, &[value]).unwrap(),
            vec![Value::Nil],
        );
    }
}

#[test]
fn a_missing_value_is_an_argument_error() {
    let mut state = installed_state();
    let error = call_tonumber(&mut state, &[]).unwrap_err();

    assert_tonumber_error(error, "bad argument #1 to 'tonumber' (value expected)");
}

#[test]
fn explicit_bases_convert_integer_numerals() {
    let mut state = installed_state();

    for (source, base, expected) in [
        ("101", 2, 5),
        (" -fF ", 16, -255),
        ("z", 36, 35),
        ("ffffffffffffffff", 16, -1),
    ] {
        assert_eq!(
            call_tonumber(&mut state, &[string(source), Value::Integer(base)],).unwrap(),
            vec![Value::Integer(expected)],
        );
    }
}

#[test]
fn invalid_explicit_base_numerals_return_nil() {
    let mut state = installed_state();

    for (source, base) in [("", 10), ("+", 10), ("2", 2), ("10x", 10)] {
        assert_eq!(
            call_tonumber(&mut state, &[string(source), Value::Integer(base)],).unwrap(),
            vec![Value::Nil],
        );
    }
}

#[test]
fn explicit_base_arguments_are_validated() {
    let mut state = installed_state();
    let cases = [
        (
            vec![string("10"), Value::Integer(1)],
            "bad argument #2 to 'tonumber' (base out of range)",
        ),
        (
            vec![string("10"), Value::Integer(37)],
            "bad argument #2 to 'tonumber' (base out of range)",
        ),
        (
            vec![string("10"), Value::Float(2.5)],
            "bad argument #2 to 'tonumber' (number has no integer representation)",
        ),
        (
            vec![Value::Integer(10), Value::Integer(2)],
            "bad argument #1 to 'tonumber' (string expected, got number)",
        ),
    ];

    for (arguments, expected) in cases {
        let error = call_tonumber(&mut state, &arguments).unwrap_err();
        assert_tonumber_error(error, expected);
    }
}

#[test]
fn nil_base_uses_standard_conversion() {
    let mut state = installed_state();

    assert_eq!(
        call_tonumber(&mut state, &[string("12.5"), Value::Nil]).unwrap(),
        vec![Value::Float(12.5)],
    );
}
