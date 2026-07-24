use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{CallOutcome, LuaString, State, Value, VmResult};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn execute(source: &str) -> VmResult<Vec<Value>> {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let function = state.load_buffer("pcall-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("pcall test unexpectedly yielded"),
    }
}

#[test]
fn pcall_forwards_arguments_preserves_results_and_catches_errors() {
    let values = execute(
        r#"
            local succeeded, first, second, third = pcall(
                function(a, b, c)
                    return a, b, c
                end,
                17,
                nil,
                false
            )

            local failed, message = pcall(function()
                return {} + 1
            end)

            return succeeded, first, second, third, failed, message
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Integer(17),
            Value::Nil,
            Value::Boolean(false),
            Value::Boolean(false),
            string("attempt to add a table value and a number value"),
        ]
    );
}
