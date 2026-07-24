use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{
    CallOutcome, LuaString, State, Value, VmError, VmErrorKind, VmResult, VmTraceFrame,
};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn installed_state() -> State {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state
}

fn call_error(state: &mut State, arguments: &[Value]) -> VmError {
    let Value::Function(error) = state.get_global(b"error").unwrap() else {
        panic!("error was not installed as a function");
    };

    match state.call(&error, arguments) {
        Err(error) => error,
        Ok(_) => panic!("error returned instead of raising"),
    }
}

fn execute(state: &mut State, source: &str) -> VmResult<Vec<Value>> {
    let function = state.load_buffer("error-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("error test unexpectedly yielded"),
    }
}

#[test]
fn install_registers_error() {
    let mut state = installed_state();

    assert!(matches!(
        state.get_global(b"error").unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn error_raises_string_nil_and_table_objects() {
    let mut state = installed_state();

    let string_error = call_error(&mut state, &[string("this is a test"), Value::Integer(0)]);
    assert_eq!(string_error.kind, VmErrorKind::Raised);
    assert_eq!(string_error.object(), Some(&string("this is a test")));
    assert_eq!(string_error.message(), "this is a test");
    assert!(matches!(
        string_error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "error"
    ));

    let nil_error = call_error(&mut state, &[]);
    assert_eq!(nil_error.kind, VmErrorKind::Raised);
    assert_eq!(nil_error.object(), Some(&Value::Nil));
    assert_eq!(nil_error.message(), "(error object is a nil value)");

    let table = state.create_table(0, 0).unwrap();
    let table_value = Value::Table(table);
    let table_error = call_error(&mut state, std::slice::from_ref(&table_value));
    assert_eq!(table_error.kind, VmErrorKind::Raised);
    assert_eq!(table_error.object(), Some(&table_value));
    assert_eq!(table_error.message(), "(error object is a table value)");
}

#[test]
fn error_accepts_integer_and_nil_levels() {
    let mut state = installed_state();

    for level in [
        Value::Integer(-1),
        Value::Integer(0),
        Value::Integer(4),
        Value::Nil,
    ] {
        let raised = call_error(&mut state, &[string("message"), level]);
        assert_eq!(raised.kind, VmErrorKind::Raised);
        assert_eq!(raised.object(), Some(&string("message")));
    }
}

#[test]
fn error_rejects_non_integer_levels() {
    let mut state = installed_state();
    let failure = call_error(&mut state, &[string("message"), Value::Boolean(false)]);

    assert_eq!(
        failure.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #2 to 'error' (number expected, got boolean)".into(),
        }
    );
    assert!(matches!(
        failure.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "error"
    ));
}

#[test]
fn pcall_preserves_explicit_error_objects() {
    let mut state = installed_state();
    let values = execute(
        &mut state,
        r#"
            local marker = {}
            local string_ok, string_error = pcall(error, "boom", 0)
            local table_ok, table_error = pcall(error, marker, 0)
            local nil_ok, nil_error = pcall(error)

            return string_ok, string_error,
                   table_ok, table_error == marker,
                   nil_ok, nil_error
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(false),
            string("boom"),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Nil,
        ]
    );
}
