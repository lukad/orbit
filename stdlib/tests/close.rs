use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{CallOutcome, LuaString, State, Value, VmResult};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn execute(source: &str) -> VmResult<Vec<Value>> {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let function = state.load_buffer("close-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("close test unexpectedly yielded"),
    }
}

#[test]
fn closes_values_in_lifo_order_and_preserves_return_values() {
    let values = execute(
        r#"
            local events = {}
            local metatable = {
                __close = function(value, cause)
                    events[#events + 1] = value.name
                    events[#events + 1] = cause == nil
                end
            }

            local function close_and_return()
                local first <close> = setmetatable({ name = "first" }, metatable)
                local second <close> = setmetatable({ name = "second" }, metatable)
                return 17, nil, 23
            end

            local first, second, third = close_and_return()
            return first,
                second,
                third,
                events[1],
                events[2],
                events[3],
                events[4]
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(17),
            Value::Nil,
            Value::Integer(23),
            string("second"),
            Value::Boolean(true),
            string("first"),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn generic_for_closes_its_fourth_control_value() {
    let values = execute(
        r#"
            local closed = false
            local state = setmetatable({}, {
                __close = function(_, cause)
                    closed = cause == nil
                end
            })

            local function iterator()
                return nil
            end

            for value in iterator, nil, nil, state do
            end

            return closed
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Boolean(true)]);
}

#[test]
fn close_errors_replace_previous_errors_and_reach_remaining_closers() {
    let values = execute(
        r#"
            local events = {}

            local function resource(name, failure)
                return setmetatable({
                    name = name,
                    failure = failure,
                }, {
                    __close = function(value, cause)
                        events[#events + 1] = value.name
                        events[#events + 1] = cause
                        error(value.failure)
                    end
                })
            end

            local succeeded, message = pcall(function()
                local first <close> = resource("first", "first close")
                local second <close> = resource("second", "second close")
                error("body")
            end)

            return succeeded,
                message,
                events[1],
                events[2],
                events[3],
                events[4]
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(false),
            string("first close"),
            string("second"),
            string("body"),
            string("first"),
            string("second close"),
        ]
    );
}

#[test]
fn pcall_converts_a_non_closable_vm_error_to_its_lua_message() {
    let values = execute(
        r#"
            local succeeded, message = pcall(function()
                local resource <close> = {}
            end)

            return succeeded, message
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(false),
            string("variable 'resource' got a non-closeable value"),
        ]
    );
}
