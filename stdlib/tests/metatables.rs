use orbit_stdlib::install;
use orbit_vm::{
    CallOutcome, LuaString, NoLoadService, State, Value, VmError, VmErrorKind, VmResult,
    VmTraceFrame,
};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn call(state: &mut State, name: &[u8], arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Function(function) = state.get_global(name)? else {
        panic!("installed global is not a function");
    };

    match state.call(&function, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => {
            panic!("base-library function unexpectedly yielded")
        }
    }
}

fn assert_native_error(error: VmError, function: &str, message: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: message.into(),
        }
    );

    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == function
    ));
}

#[test]
fn install_registers_both_metatable_functions() {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    assert!(matches!(
        state.get_global(b"getmetatable").unwrap(),
        Value::Function(_)
    ));

    assert!(matches!(
        state.get_global(b"setmetatable").unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn setmetatable_sets_replaces_clears_and_returns_the_table() {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    let target = state.create_table(0, 0).unwrap();
    let first = state.create_table(0, 0).unwrap();
    let second = state.create_table(0, 0).unwrap();
    let target_value = Value::Table(target.clone());

    assert_eq!(
        call(
            &mut state,
            b"setmetatable",
            &[
                target_value.clone(),
                Value::Table(first.clone()),
                Value::Integer(99),
            ],
        )
        .unwrap(),
        vec![target_value.clone()]
    );

    assert_eq!(
        state.get_metatable(&target_value).unwrap(),
        Some(first.clone())
    );

    assert_eq!(
        call(
            &mut state,
            b"getmetatable",
            &[target_value.clone(), Value::Integer(99)],
        )
        .unwrap(),
        vec![Value::Table(first)]
    );

    assert_eq!(
        call(
            &mut state,
            b"setmetatable",
            &[target_value.clone(), Value::Table(second.clone())],
        )
        .unwrap(),
        vec![target_value.clone()]
    );

    assert_eq!(
        state.get_metatable(&target_value).unwrap(),
        Some(second.clone())
    );

    assert_eq!(
        call(
            &mut state,
            b"setmetatable",
            &[target_value.clone(), Value::Nil],
        )
        .unwrap(),
        vec![target_value.clone()]
    );

    assert_eq!(state.get_metatable(&target_value).unwrap(), None);

    assert_eq!(
        call(&mut state, b"getmetatable", &[target_value]).unwrap(),
        vec![Value::Nil]
    );
}

#[test]
fn getmetatable_accepts_any_value_and_uses_shared_type_metatables() {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    assert_eq!(
        call(&mut state, b"getmetatable", &[Value::Nil]).unwrap(),
        vec![Value::Nil]
    );

    let metatable = state.create_table(0, 1).unwrap();

    state
        .raw_set(&metatable, &string("__metatable"), &Value::Integer(42))
        .unwrap();

    state
        .set_metatable(&string("seed"), Some(&metatable))
        .unwrap();

    assert_eq!(
        call(&mut state, b"getmetatable", &[string("another string")],).unwrap(),
        vec![Value::Integer(42)]
    );
}

#[test]
fn protection_value_is_returned_and_blocks_replacement_and_removal() {
    for protection in [string("locked"), Value::Boolean(false)] {
        let mut state = State::new(NoLoadService).unwrap();
        install(&mut state).unwrap();

        let target = state.create_table(0, 0).unwrap();
        let current = state.create_table(0, 1).unwrap();
        let replacement = state.create_table(0, 0).unwrap();
        let target_value = Value::Table(target);

        state
            .raw_set(&current, &string("__metatable"), &protection)
            .unwrap();

        assert_eq!(
            call(
                &mut state,
                b"setmetatable",
                &[target_value.clone(), Value::Table(current.clone())],
            )
            .unwrap(),
            vec![target_value.clone()]
        );

        assert_eq!(
            call(
                &mut state,
                b"getmetatable",
                std::slice::from_ref(&target_value),
            )
            .unwrap(),
            vec![protection.clone()]
        );

        for requested in [Value::Table(replacement.clone()), Value::Nil] {
            let error = call(
                &mut state,
                b"setmetatable",
                &[target_value.clone(), requested],
            )
            .unwrap_err();

            assert_native_error(error, "setmetatable", "cannot change a protected metatable");
        }

        assert_eq!(state.get_metatable(&target_value).unwrap(), Some(current));
    }
}

#[test]
fn protection_lookup_is_raw() {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    let target = state.create_table(0, 0).unwrap();
    let current = state.create_table(0, 0).unwrap();
    let current_metatable = state.create_table(0, 1).unwrap();
    let inherited_fields = state.create_table(0, 1).unwrap();
    let replacement = state.create_table(0, 0).unwrap();
    let target_value = Value::Table(target);

    state
        .raw_set(
            &inherited_fields,
            &string("__metatable"),
            &string("inherited protection must be ignored"),
        )
        .unwrap();

    state
        .raw_set(
            &current_metatable,
            &string("__index"),
            &Value::Table(inherited_fields),
        )
        .unwrap();

    state
        .set_metatable(&Value::Table(current.clone()), Some(&current_metatable))
        .unwrap();

    call(
        &mut state,
        b"setmetatable",
        &[target_value.clone(), Value::Table(current.clone())],
    )
    .unwrap();

    assert_eq!(
        call(
            &mut state,
            b"getmetatable",
            std::slice::from_ref(&target_value),
        )
        .unwrap(),
        vec![Value::Table(current)]
    );

    assert_eq!(
        call(
            &mut state,
            b"setmetatable",
            &[target_value.clone(), Value::Table(replacement.clone())],
        )
        .unwrap(),
        vec![target_value.clone()]
    );

    assert_eq!(
        state.get_metatable(&target_value).unwrap(),
        Some(replacement)
    );
}

#[test]
fn metatable_functions_report_lua_54_argument_errors() {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    let table = Value::Table(state.create_table(0, 0).unwrap());

    let cases = [
        (
            b"getmetatable".as_slice(),
            Vec::new(),
            "bad argument #1 to 'getmetatable' (value expected)",
        ),
        (
            b"setmetatable".as_slice(),
            Vec::new(),
            "bad argument #1 to 'setmetatable' (table expected, got no value)",
        ),
        (
            b"setmetatable".as_slice(),
            vec![Value::Nil, table.clone()],
            "bad argument #1 to 'setmetatable' (table expected, got nil)",
        ),
        (
            b"setmetatable".as_slice(),
            vec![table.clone()],
            "bad argument #2 to 'setmetatable' (nil or table expected, got no value)",
        ),
        (
            b"setmetatable".as_slice(),
            vec![table.clone(), Value::Integer(1)],
            "bad argument #2 to 'setmetatable' (nil or table expected, got number)",
        ),
    ];

    for (function, arguments, expected) in cases {
        let error = call(&mut state, function, &arguments).unwrap_err();
        let function = std::str::from_utf8(function).unwrap();

        assert_native_error(error, function, expected);
    }
}

#[test]
fn second_argument_is_validated_before_protection() {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    let target = Value::Table(state.create_table(0, 0).unwrap());
    let protected = state.create_table(0, 1).unwrap();

    state
        .raw_set(&protected, &string("__metatable"), &string("locked"))
        .unwrap();

    call(
        &mut state,
        b"setmetatable",
        &[target.clone(), Value::Table(protected)],
    )
    .unwrap();

    let error = call(&mut state, b"setmetatable", &[target, Value::Boolean(true)]).unwrap_err();

    assert_native_error(
        error,
        "setmetatable",
        "bad argument #2 to 'setmetatable' (nil or table expected, got boolean)",
    );
}
