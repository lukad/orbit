use orbit_stdlib::install;
use orbit_vm::{
    CallOutcome, LuaString, NoLoadService, State, Value, VmError, VmErrorKind, VmResult,
    VmTraceFrame,
};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn installed_state() -> State {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();
    state
}

fn call_select(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Function(select) = state.get_global(b"select")? else {
        panic!("select was not installed as a function");
    };

    match state.call(&select, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("select unexpectedly yielded"),
    }
}

fn assert_select_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "select"
    ));
}

#[test]
fn hash_returns_the_number_of_extra_arguments() {
    let mut state = installed_state();

    assert_eq!(
        call_select(&mut state, &[string("#")]).unwrap(),
        vec![Value::Integer(0)]
    );
    assert_eq!(
        call_select(
            &mut state,
            &[
                string("#"),
                Value::Integer(10),
                Value::Nil,
                Value::Boolean(false),
            ],
        )
        .unwrap(),
        vec![Value::Integer(3)]
    );
}

#[test]
fn positive_indices_return_the_requested_tail_and_preserve_nil() {
    let mut state = installed_state();
    let values = [
        Value::Integer(1),
        string("first"),
        Value::Nil,
        string("last"),
    ];

    assert_eq!(
        call_select(&mut state, &values).unwrap(),
        vec![string("first"), Value::Nil, string("last")]
    );

    let values = [
        Value::Integer(2),
        string("first"),
        Value::Nil,
        string("last"),
    ];
    assert_eq!(
        call_select(&mut state, &values).unwrap(),
        vec![Value::Nil, string("last")]
    );
}

#[test]
fn positive_indices_past_the_end_return_no_values() {
    let mut state = installed_state();

    for arguments in [
        vec![Value::Integer(1)],
        vec![Value::Integer(4), string("a"), string("b"), string("c")],
        vec![Value::Integer(10_000), string("a"), string("b")],
    ] {
        assert_eq!(call_select(&mut state, &arguments).unwrap(), vec![]);
    }
}

#[test]
fn negative_indices_count_back_from_the_last_argument() {
    let mut state = installed_state();
    let arguments = [
        Value::Integer(-1),
        string("first"),
        Value::Nil,
        string("last"),
    ];
    assert_eq!(
        call_select(&mut state, &arguments).unwrap(),
        vec![string("last")]
    );

    let arguments = [
        Value::Float(-3.0),
        string("first"),
        Value::Nil,
        string("last"),
    ];
    assert_eq!(
        call_select(&mut state, &arguments).unwrap(),
        vec![string("first"), Value::Nil, string("last")]
    );
}

#[test]
fn rejects_missing_non_integer_and_out_of_range_indices() {
    let mut state = installed_state();
    let cases = [
        (
            vec![],
            "bad argument #1 to 'select' (number expected, got no value)",
        ),
        (
            vec![Value::Boolean(true)],
            "bad argument #1 to 'select' (number expected, got boolean)",
        ),
        (
            vec![Value::Float(1.5), string("value")],
            "bad argument #1 to 'select' (number has no integer representation)",
        ),
        (
            vec![Value::Integer(0), string("value")],
            "bad argument #1 to 'select' (index out of range)",
        ),
        (
            vec![Value::Integer(-3), string("a"), string("b")],
            "bad argument #1 to 'select' (index out of range)",
        ),
        (
            vec![Value::Integer(i64::MIN), string("value")],
            "bad argument #1 to 'select' (index out of range)",
        ),
    ];

    for (arguments, expected) in cases {
        let error = call_select(&mut state, &arguments).unwrap_err();
        assert_select_error(error, expected);
    }
}
