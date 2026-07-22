use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{
    CallOutcome, LuaString, NativeAction, NativeContext, NativeEvent, NativeToken, State, Table,
    Value, VmError, VmErrorKind, VmResult,
};

const METAMETHOD_RESUME: NativeToken = NativeToken::new(1);

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn installed_state() -> State {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state
}

fn execute(state: &mut State, source: &str) -> VmResult<Vec<Value>> {
    let function = state.load_buffer("require-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("test chunk unexpectedly yielded"),
    }
}

fn call_require<'state>(state: &'state mut State, name: &str) -> VmResult<CallOutcome<'state>> {
    let Value::Function(require) = state.get_global(b"require")? else {
        panic!("require was not installed as a function");
    };

    state.call(&require, &[string(name)])
}

fn package_table(state: &mut State) -> Table {
    let Value::Table(package) = state.get_global(b"package").unwrap() else {
        panic!("package was not installed as a table");
    };

    package
}

fn table_field(state: &mut State, table: &Table, name: &str) -> Value {
    state
        .raw_get(table, &Value::String(LuaString::from(name)))
        .unwrap()
}

fn set_table_field(state: &mut State, table: &Table, name: &str, value: &Value) {
    state
        .raw_set(table, &Value::String(LuaString::from(name)), value)
        .unwrap();
}

fn install_yielding_metamethod(state: &mut State, target: &Table, name: &str) {
    let metamethod = state
        .create_native_function(format!("yielding {name}"), yielding_metamethod, &[])
        .unwrap();
    let metatable = state.create_table(0, 1).unwrap();

    set_table_field(state, &metatable, name, &Value::Function(metamethod));
    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();
}

fn yielding_metamethod(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            Ok(context.yield_values([context.string("metamethod paused")], METAMETHOD_RESUME))
        }
        NativeEvent::Resume {
            token: METAMETHOD_RESUME,
        } => Ok(context.return_values([context.resume_value(0).unwrap_or_else(|| context.nil())])),
        NativeEvent::ResumeError {
            token: METAMETHOD_RESUME,
        } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
        NativeEvent::Resume { .. } | NativeEvent::ResumeError { .. } => {
            Err(VmErrorKind::InvalidNativeContinuation {
                message: "unexpected yielding metamethod continuation",
            }
            .into())
        }
    }
}

fn assert_native_message(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
}

#[test]
fn require_only_caches_truthy_values_and_cached_calls_return_one_value() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local runs = 0
            package.preload["cache-test"] = function()
                runs = runs + 1
                return "loaded"
            end

            local first, first_data = require("cache-test")
            local second, second_data = require("cache-test")

            package.loaded["cache-test"] = false
            local third, third_data = require("cache-test")

            return first, first_data, second, second_data,
                third, third_data, runs
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            string("loaded"),
            string(":preload:"),
            string("loaded"),
            Value::Nil,
            string("loaded"),
            string(":preload:"),
            Value::Integer(2),
        ]
    );
}

#[test]
fn searcher_and_loader_receive_the_documented_name_and_loader_data() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local search_name
            local loader_name
            local loader_argument

            package.searchers = {
                function(name)
                    search_name = name
                    return function(received_name, received_data)
                        loader_name = received_name
                        loader_argument = received_data
                        return 123
                    end, "custom loader data"
                end,
            }

            local module, returned_data = require("requested.module")
            return module, returned_data, search_name, loader_name, loader_argument
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(123),
            string("custom loader data"),
            string("requested.module"),
            string("requested.module"),
            string("custom loader data"),
        ]
    );
}

#[test]
fn nil_loader_results_use_manual_package_loaded_values_or_default_to_true() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            package.preload["defaulted"] = function()
                return nil
            end

            package.preload["manual"] = function(name)
                package.loaded[name] = "installed manually"
                return nil
            end

            local defaulted, defaulted_data = require("defaulted")
            local manual, manual_data = require("manual")

            return defaulted, defaulted_data, package.loaded["defaulted"],
                manual, manual_data, package.loaded["manual"]
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            string(":preload:"),
            Value::Boolean(true),
            string("installed manually"),
            string(":preload:"),
            string("installed manually"),
        ]
    );
}

#[test]
fn searchers_run_in_numeric_order_and_stop_after_finding_a_loader() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local order = ""
            package.searchers = {
                function()
                    order = order .. "1"
                    return "first miss"
                end,
                function()
                    order = order .. "2"
                    return false
                end,
                function()
                    order = order .. "3"
                    return function()
                        return "found"
                    end, "third searcher"
                end,
                function()
                    order = order .. "4"
                    return function()
                        return "wrong loader"
                    end
                end,
            }

            local module, loader_data = require("ordered")
            return module, loader_data, order
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![string("found"), string("third searcher"), string("123"),]
    );
}

#[test]
fn require_accumulates_only_string_convertible_searcher_messages() {
    let mut state = installed_state();

    let error = execute(
        &mut state,
        r#"
            package.searchers = {
                function() return "first miss" end,
                function() return 42 end,
                function() return false end,
            }
            return require("absent")
        "#,
    )
    .unwrap_err();

    assert_native_message(error, "module 'absent' not found:\n\tfirst miss\n\t42");
}

#[test]
fn searcher_and_loader_runtime_errors_propagate_unchanged() {
    for source in [
        r#"
            package.searchers = {
                function()
                    local zero = 0
                    return 1 // zero
                end,
            }
            return require("searcher-error")
        "#,
        r#"
            package.searchers = {
                function()
                    return function()
                        local zero = 0
                        return 1 // zero
                    end
                end,
            }
            return require("loader-error")
        "#,
    ] {
        let mut state = installed_state();
        let error = execute(&mut state, source).unwrap_err();
        assert_eq!(error.kind, VmErrorKind::IntegerDivisionByZero);
    }
}

#[test]
fn package_loaded_honors_index_and_newindex_metamethods() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local backing = { cached = "virtual cache" }
            setmetatable(package.loaded, {
                __index = backing,
                __newindex = backing,
            })

            package.preload["stored through metamethod"] = function()
                return "loader result"
            end

            local cached, cached_data = require("cached")
            local loaded, loaded_data = require("stored through metamethod")

            return cached, cached_data, loaded, loaded_data,
                rawget(package.loaded, "stored through metamethod"),
                backing["stored through metamethod"]
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            string("virtual cache"),
            Value::Nil,
            string("loader result"),
            string(":preload:"),
            Value::Nil,
            string("loader result"),
        ]
    );
}

#[test]
fn package_loaded_index_can_yield_and_remains_gc_safe() {
    let mut state = installed_state();
    let package = package_table(&mut state);
    let Value::Table(loaded) = table_field(&mut state, &package, "loaded") else {
        panic!("package.loaded was not installed as a table");
    };

    install_yielding_metamethod(&mut state, &loaded, "__index");

    let CallOutcome::Yielded {
        values,
        mut suspension,
    } = call_require(&mut state, "yielded-cache").unwrap()
    else {
        panic!("package.loaded __index did not yield");
    };

    assert_eq!(values, vec![string("metamethod paused")]);
    suspension.collect_garbage().unwrap();

    let CallOutcome::Returned(values) = suspension.resume(&[Value::Integer(91)]).unwrap() else {
        panic!("resumed require unexpectedly yielded again");
    };

    assert_eq!(values, vec![Value::Integer(91)]);
}

#[test]
fn package_loaded_newindex_can_yield_and_remains_gc_safe() {
    let mut state = installed_state();

    execute(
        &mut state,
        r#"
            package.preload["yielded-store"] = function()
                return 93
            end
        "#,
    )
    .unwrap();

    let package = package_table(&mut state);
    let Value::Table(loaded) = table_field(&mut state, &package, "loaded") else {
        panic!("package.loaded was not installed as a table");
    };
    install_yielding_metamethod(&mut state, &loaded, "__newindex");

    let CallOutcome::Yielded {
        values,
        mut suspension,
    } = call_require(&mut state, "yielded-store").unwrap()
    else {
        panic!("package.loaded __newindex did not yield");
    };

    assert_eq!(values, vec![string("metamethod paused")]);
    suspension.collect_garbage().unwrap();

    let CallOutcome::Yielded {
        values,
        mut suspension,
    } = suspension.resume(&[]).unwrap()
    else {
        panic!("default package.loaded __newindex did not yield");
    };

    assert_eq!(values, vec![string("metamethod paused")]);
    suspension.collect_garbage().unwrap();

    let CallOutcome::Returned(values) = suspension.resume(&[]).unwrap() else {
        panic!("resumed require unexpectedly yielded a third time");
    };

    assert_eq!(values, vec![Value::Boolean(true), string(":preload:")]);
}

#[test]
fn package_searchers_field_lookup_can_yield_and_remains_gc_safe() {
    let mut state = installed_state();

    execute(
        &mut state,
        r#"
            package.preload["yielded-searchers"] = function()
                return 92
            end
        "#,
    )
    .unwrap();

    let package = package_table(&mut state);
    let searchers = table_field(&mut state, &package, "searchers");
    set_table_field(&mut state, &package, "searchers", &Value::Nil);
    install_yielding_metamethod(&mut state, &package, "__index");

    let CallOutcome::Yielded {
        values,
        mut suspension,
    } = call_require(&mut state, "yielded-searchers").unwrap()
    else {
        panic!("package.searchers lookup did not yield");
    };

    assert_eq!(values, vec![string("metamethod paused")]);
    suspension.collect_garbage().unwrap();

    let CallOutcome::Returned(values) = suspension.resume(&[searchers]).unwrap() else {
        panic!("resumed require unexpectedly yielded again");
    };

    assert_eq!(values, vec![Value::Integer(92), string(":preload:")]);
}

#[test]
fn numeric_searcher_slots_are_read_raw() {
    let mut state = installed_state();

    let error = execute(
        &mut state,
        r#"
            numeric_searcher_index_hits = 0
            package.searchers = setmetatable({}, {
                __index = function()
                    numeric_searcher_index_hits = numeric_searcher_index_hits + 1
                    return function()
                        return "must not load"
                    end
                end,
            })

            return require("raw-searcher-slots")
        "#,
    )
    .unwrap_err();

    assert_native_message(error, "module 'raw-searcher-slots' not found:");
    assert_eq!(
        state.get_global(b"numeric_searcher_index_hits").unwrap(),
        Value::Integer(0)
    );
}

#[test]
fn require_and_preload_searcher_keep_their_original_loaded_and_preload_tables() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local original_loaded = package.loaded
            local original_preload = package.preload

            package.loaded = {}
            package.preload = {}

            original_loaded["captured cache"] = 31
            original_preload["captured preload"] = function()
                return 32
            end

            local cached, cached_data = require("captured cache")
            local preloaded, preloaded_data = require("captured preload")

            return cached, cached_data, preloaded, preloaded_data,
                package.loaded["captured cache"],
                package.loaded["captured preload"]
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(31),
            Value::Nil,
            Value::Integer(32),
            string(":preload:"),
            Value::Nil,
            Value::Nil,
        ]
    );
}
