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

fn call_sub(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(string_library) = state.get_global(b"string")? else {
        panic!("string was not installed as a table");
    };
    let Value::Function(sub) = state.raw_get(&string_library, &string("sub"))? else {
        panic!("string.sub was not installed as a function");
    };

    match state.call(&sub, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("string.sub unexpectedly yielded"),
    }
}

fn assert_sub_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "string.sub"
    ));
}

#[test]
fn install_registers_string_sub() {
    let mut state = installed_state();
    let Value::Table(string_library) = state.get_global(b"string").unwrap() else {
        panic!("string was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&string_library, &string("sub")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn positive_positions_are_one_based_and_the_end_is_inclusive() {
    let mut state = installed_state();

    for (arguments, expected) in [
        (
            vec![string("abcde"), Value::Integer(2), Value::Integer(4)],
            "bcd",
        ),
        (vec![string("abcde"), Value::Integer(2)], "bcde"),
        (
            vec![string("abcde"), Value::Integer(5), Value::Integer(5)],
            "e",
        ),
        (
            vec![string("abcde"), Value::Integer(2), Value::Integer(1)],
            "",
        ),
    ] {
        assert_eq!(
            call_sub(&mut state, &arguments).unwrap(),
            vec![string(expected)]
        );
    }
}

#[test]
fn zero_negative_and_out_of_range_positions_follow_lua_rules() {
    let mut state = installed_state();

    for (arguments, expected) in [
        (vec![string("abcde"), Value::Integer(0)], "abcde"),
        (vec![string("abcde"), Value::Integer(-1)], "e"),
        (vec![string("abcde"), Value::Integer(-10)], "abcde"),
        (vec![string("abcde"), Value::Integer(6)], ""),
        (vec![string("abcde"), Value::Integer(600)], ""),
        (
            vec![string("abcde"), Value::Integer(1), Value::Integer(-1)],
            "abcde",
        ),
        (
            vec![string("abcde"), Value::Integer(1), Value::Integer(-5)],
            "a",
        ),
        (
            vec![string("abcde"), Value::Integer(1), Value::Integer(-10)],
            "",
        ),
        (
            vec![string("abcde"), Value::Integer(1), Value::Integer(600)],
            "abcde",
        ),
        (vec![string("abcde"), Value::Integer(i64::MIN)], "abcde"),
        (vec![string("abcde"), Value::Integer(i64::MAX)], ""),
    ] {
        assert_eq!(
            call_sub(&mut state, &arguments).unwrap(),
            vec![string(expected)]
        );
    }
}

#[test]
fn nil_end_uses_the_default() {
    let mut state = installed_state();

    assert_eq!(
        call_sub(
            &mut state,
            &[string("abcde"), Value::Integer(3), Value::Nil]
        )
        .unwrap(),
        vec![string("cde")]
    );
}

#[test]
fn integer_arguments_coerce_numeric_strings() {
    let mut state = installed_state();

    assert_eq!(
        call_sub(&mut state, &[string("abcde"), string("2"), string("4.0")]).unwrap(),
        vec![string("bcd")]
    );
}

#[test]
fn positions_count_bytes_and_preserve_arbitrary_string_data() {
    let mut state = installed_state();
    let subject = string([0xff, 0x00, b'a', 0xc3, 0xa9]);

    assert_eq!(
        call_sub(&mut state, &[subject, Value::Integer(2), Value::Integer(4)]).unwrap(),
        vec![string([0x00, b'a', 0xc3])]
    );
}

#[test]
fn invalid_indices_report_lua_argument_errors() {
    let mut state = installed_state();
    let cases = [
        (
            vec![string("abcde")],
            "bad argument #2 to 'sub' (number expected, got no value)",
        ),
        (
            vec![string("abcde"), Value::Boolean(true)],
            "bad argument #2 to 'sub' (number expected, got boolean)",
        ),
        (
            vec![string("abcde"), Value::Float(1.5)],
            "bad argument #2 to 'sub' (number has no integer representation)",
        ),
        (
            vec![string("abcde"), string("1.5")],
            "bad argument #2 to 'sub' (number has no integer representation)",
        ),
        (
            vec![string("abcde"), Value::Integer(1), Value::Boolean(false)],
            "bad argument #3 to 'sub' (number expected, got boolean)",
        ),
        (
            vec![string("abcde"), Value::Integer(1), string("4.5")],
            "bad argument #3 to 'sub' (number has no integer representation)",
        ),
    ];

    for (arguments, expected) in cases {
        let error = call_sub(&mut state, &arguments).unwrap_err();
        assert_sub_error(error, expected);
    }
}

#[test]
fn install_configures_the_shared_string_metatable() {
    let mut state = installed_state();

    let Value::Table(string_library) = state.get_global(b"string").unwrap() else {
        panic!("string was not installed as a table");
    };

    let first = state
        .get_metatable(&string("first"))
        .unwrap()
        .expect("strings should have a metatable");

    let second = state
        .get_metatable(&string("second"))
        .unwrap()
        .expect("strings should share a metatable");

    assert_eq!(first, second);
    assert_eq!(
        state.raw_get(&first, &string("__index")).unwrap(),
        Value::Table(string_library),
    );
}
