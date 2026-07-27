use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{CallOutcome, LuaString, State, Value, VmError, VmErrorKind, VmResult};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn installed_state() -> State {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state
}

fn execute(state: &mut State, source: &str) -> VmResult<Vec<Value>> {
    let function = state.load_buffer("collectgarbage-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("collectgarbage test unexpectedly yielded"),
    }
}

fn call_error(state: &mut State, arguments: &[Value]) -> VmError {
    let Value::Function(collectgarbage) = state.get_global(b"collectgarbage").unwrap() else {
        panic!("collectgarbage was not installed as a function");
    };

    match state.call(&collectgarbage, arguments) {
        Err(error) => error,
        Ok(_) => panic!("collectgarbage returned instead of raising"),
    }
}

#[test]
fn install_registers_collectgarbage() {
    let mut state = installed_state();

    assert!(matches!(
        state.get_global(b"collectgarbage").unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn collect_returns_zero_with_default_and_explicit_option() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"return collectgarbage(), collectgarbage("collect")"#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Integer(0), Value::Integer(0)]);
}

#[test]
fn count_tracks_heap_growth_and_collection() {
    let mut state = installed_state();

    // Build a sizeable live graph, release its final local reference, and
    // verify that an explicit collection reclaims it.
    let values = execute(
        &mut state,
        r#"
            collectgarbage("collect")
            local before = collectgarbage("count")
            local function build()
                local tables = {}
                for i = 1, 512 do tables[i] = {i} end
                return tables
            end
            local hold = build()
            local grown = collectgarbage("count")
            hold = nil
            collectgarbage("collect")
            local shrunk = collectgarbage("count")
            return type(before), before > 0, grown > before, shrunk < grown
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            string("number"),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn collection_drops_out_of_scope_values_but_keeps_active_locals() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local weak = setmetatable({}, { __mode = "v" })

            do
                local dead = {}
                weak.dead = dead
            end

            local live = {}
            weak.live = live

            collectgarbage("collect")

            return weak.dead == nil, weak.live == live
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Boolean(true), Value::Boolean(true)]);
}

#[test]
fn automatic_collection_is_not_blocked_by_stale_condition_registers() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local weak = setmetatable({}, { __mode = "v" })

            do
                local dead = {}
                weak[1] = dead
            end

            local iterations = 0

            while weak[1] ~= nil and iterations < 10000 do
                local garbage = {}
                garbage[garbage] = garbage
                local text =
                    iterations .. iterations .. iterations .. iterations
                iterations = iterations + 1
            end

            return weak[1] == nil, iterations < 10000
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Boolean(true), Value::Boolean(true)]);
}

#[test]
fn stop_and_restart_toggle_isrunning() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local initially = collectgarbage("isrunning")
            local stop_result = collectgarbage("stop")
            local stopped = collectgarbage("isrunning")
            local restart_result = collectgarbage("restart")
            local restarted = collectgarbage("isrunning")
            return initially, stop_result, stopped, restart_result, restarted
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Integer(0),
            Value::Boolean(false),
            Value::Integer(0),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn stop_prevents_automatic_collection() {
    let mut state = installed_state();

    execute(
        &mut state,
        r#"
            collectgarbage("stop")
            for i = 1, 3000 do local garbage = {i} end
        "#,
    )
    .unwrap();

    // The loop crossed the automatic-collection threshold several times over.
    // With the collector stopped, all of that garbage must still be there for
    // an explicit host-driven collection to reclaim.
    let reclaimed = state.collect_garbage().unwrap();
    assert!(
        reclaimed >= 2000,
        "expected at least 2000 uncollected objects after crossing the threshold while stopped, \
         got {reclaimed}"
    );
}

#[test]
fn restart_resets_allocation_pressure() {
    let mut state = installed_state();

    execute(
        &mut state,
        r#"
            collectgarbage("stop")
            for i = 1, 3000 do local garbage = {i} end
            collectgarbage("restart")
            local survivor = {1, 2, 3}
        "#,
    )
    .unwrap();

    // Restarting clears the allocation debt, so the handful of allocations
    // after it must not trigger an immediate automatic collection despite the
    // debt having been far over the threshold before the restart.
    let reclaimed = state.collect_garbage().unwrap();
    assert!(
        reclaimed >= 2000,
        "expected garbage to survive past restart (debt reset), got {reclaimed} reclaimed"
    );
}

#[test]
fn explicit_collection_works_while_stopped() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            collectgarbage("stop")
            local collect_result = collectgarbage("collect")
            local step_result = collectgarbage("step")
            local running = collectgarbage("isrunning")
            collectgarbage("restart")
            return collect_result, step_result, running
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(0),
            Value::Boolean(true),
            Value::Boolean(false),
        ]
    );
}

#[test]
fn step_returns_true_with_and_without_size() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"return collectgarbage("step"), collectgarbage("step", 0), collectgarbage("step", 1024)"#,
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
fn mode_switching_returns_previous_mode() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local first = collectgarbage("generational")
            local second = collectgarbage("generational")
            local third = collectgarbage("incremental")
            local fourth = collectgarbage("incremental")
            local with_args = collectgarbage("generational", 20, 20)
            return first, second, third, fourth, with_args
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            string("incremental"),
            string("generational"),
            string("generational"),
            string("incremental"),
            string("incremental"),
        ]
    );
}

#[test]
fn invalid_option_raises() {
    let mut state = installed_state();

    let failure = call_error(&mut state, &[string("bogus")]);

    assert_eq!(
        failure.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'collectgarbage' (invalid option 'bogus')".into(),
        }
    );
}

#[test]
fn non_string_option_raises() {
    let mut state = installed_state();

    let failure = call_error(&mut state, &[Value::Boolean(true)]);

    assert_eq!(
        failure.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'collectgarbage' (string expected, got boolean)".into(),
        }
    );
}
