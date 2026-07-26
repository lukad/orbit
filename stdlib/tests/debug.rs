use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{CallOutcome, NativeAction, NativeContext, State, Value, VmResult};

fn state() -> State {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state
}

fn run(state: &mut State, source: &str) -> Vec<Value> {
    let chunk = state.load_buffer("debug-test", source).unwrap();

    match state.call(&chunk, &[]).unwrap() {
        CallOutcome::Returned(values) => values,
        CallOutcome::Yielded { .. } => panic!("debug test unexpectedly yielded"),
    }
}

fn captured_native(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    Ok(context.return_values([context.nil()]))
}

#[test]
fn installs_a_requireable_debug_module_with_upvalueid() {
    let mut state = state();

    let Value::Table(global_debug) = state.get_global(b"debug").unwrap() else {
        panic!("debug was not installed as a table");
    };

    assert_eq!(
        run(
            &mut state,
            r#"
                local required = require "debug"
                return required, required == debug,
                    package.loaded.debug == debug, type(debug.upvalueid)
            "#,
        ),
        vec![
            Value::Table(global_debug),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::String("function".into()),
        ]
    );
}

#[test]
fn upvalueid_identifies_shared_lua_upvalue_cells() {
    let mut state = state();

    assert_eq!(
        run(
            &mut state,
            r#"
                local function shared()
                    local value = 1
                    return function() return value end,
                        function() return value end
                end

                local function separate()
                    local value = 1
                    return function() return value end
                end

                local first, second = shared()
                local third = separate()
                local first_id = debug.upvalueid(first, 1)

                return first_id == debug.upvalueid(first, 1),
                    first_id == debug.upvalueid(second, 1),
                    first_id ~= debug.upvalueid(third, 1),
                    type(first_id),
                    string.find(tostring(first_id), "^userdata: 0x") ~= nil
            "#,
        ),
        vec![
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::String("userdata".into()),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn upvalueid_identifies_native_capture_slots() {
    let mut state = state();
    let native = state
        .create_native_function(
            "captured-native",
            captured_native,
            &[Value::Integer(1), Value::Integer(1)],
        )
        .unwrap();
    state
        .set_global(b"captured_native", &Value::Function(native))
        .unwrap();

    assert_eq!(
        run(
            &mut state,
            r#"
                local first = debug.upvalueid(captured_native, 1)
                local second = debug.upvalueid(captured_native, 2)
                return first == debug.upvalueid(captured_native, 1),
                    first ~= second,
                    type(first),
                    debug.upvalueid(type, 1) == nil
            "#,
        ),
        vec![
            Value::Boolean(true),
            Value::Boolean(true),
            Value::String("userdata".into()),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn upvalueid_returns_nil_for_invalid_indices_and_checks_arguments() {
    let mut state = state();

    assert_eq!(
        run(
            &mut state,
            r#"
                local captured = 1
                local function closure() return captured end

                return debug.upvalueid(closure, 0) == nil,
                    debug.upvalueid(closure, -1) == nil,
                    debug.upvalueid(closure, math.maxinteger) == nil,
                    not pcall(debug.upvalueid, 1, 1),
                    not pcall(debug.upvalueid, closure, "not an integer")
            "#,
        ),
        vec![Value::Boolean(true); 5]
    );
}

#[test]
fn upvalue_ids_are_table_keys_and_do_not_keep_upvalues_alive() {
    let mut state = state();

    assert_eq!(
        run(
            &mut state,
            r#"
                local function make()
                    local captured = 1
                    return function() return captured end
                end

                local closure = make()
                local id = debug.upvalueid(closure, 1)
                local values = {[id] = "found"}
                closure = nil
                collectgarbage()

                return values[id]
            "#,
        ),
        vec![Value::String("found".into())]
    );
}

#[test]
fn upvalue_ids_from_different_states_do_not_collide() {
    let source = r#"
        local captured = 1
        local function closure() return captured end
        return debug.upvalueid(closure, 1)
    "#;

    let mut first = state();
    let mut second = state();
    let [first_id] = run(&mut first, source).try_into().unwrap();
    let [second_id] = run(&mut second, source).try_into().unwrap();

    assert_ne!(first_id, second_id);

    first.set_global(b"first_id", &first_id).unwrap();
    first.set_global(b"second_id", &second_id).unwrap();

    assert_eq!(
        run(&mut first, "return first_id ~= second_id"),
        vec![Value::Boolean(true)]
    );
}
