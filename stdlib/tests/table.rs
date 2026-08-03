use orbit_loader::Loader;
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

fn installed_loader_state() -> State {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state
}

fn indexed_table(state: &mut State, entries: impl IntoIterator<Item = (i64, Value)>) -> Value {
    let entries = entries.into_iter().collect::<Vec<_>>();
    let table = state.create_table(entries.len(), 0).unwrap();

    for (index, value) in entries {
        state
            .raw_set(&table, &Value::Integer(index), &value)
            .unwrap();
    }

    Value::Table(table)
}

fn call_concat(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(table_library) = state.get_global(b"table")? else {
        panic!("table was not installed as a table");
    };
    let Value::Function(concat) = state.raw_get(&table_library, &string("concat"))? else {
        panic!("table.concat was not installed as a function");
    };

    match state.call(&concat, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("table.concat unexpectedly yielded"),
    }
}

fn call_unpack(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(table_library) = state.get_global(b"table")? else {
        panic!("table was not installed as a table");
    };
    let Value::Function(unpack) = state.raw_get(&table_library, &string("unpack"))? else {
        panic!("table.unpack was not installed as a function");
    };

    match state.call(&unpack, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("table.unpack unexpectedly yielded"),
    }
}

fn execute_in_state(state: &mut State, source: &str) -> VmResult<Vec<Value>> {
    let function = state.load_buffer("table-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("table test unexpectedly yielded"),
    }
}

fn execute_table_test(source: &str) -> VmResult<Vec<Value>> {
    execute_in_state(&mut installed_loader_state(), source)
}

fn assert_concat_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "table.concat"
    ));
}

fn assert_unpack_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "table.unpack"
    ));
}

#[test]
fn install_registers_table_concat() {
    let mut state = installed_state();
    let Value::Table(table_library) = state.get_global(b"table").unwrap() else {
        panic!("table was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&table_library, &string("concat")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn concat_joins_the_default_or_requested_range() {
    let mut state = installed_state();
    let values = indexed_table(
        &mut state,
        [
            (1, string("first")),
            (2, string("second")),
            (3, string("third")),
        ],
    );

    for (arguments, expected) in [
        (vec![values.clone()], "firstsecondthird"),
        (vec![values.clone(), string("-")], "first-second-third"),
        (
            vec![
                values.clone(),
                Value::Nil,
                Value::Integer(2),
                Value::Integer(3),
                string("ignored"),
            ],
            "secondthird",
        ),
        (
            vec![
                values.clone(),
                string(","),
                Value::Integer(3),
                Value::Integer(2),
            ],
            "",
        ),
    ] {
        assert_eq!(
            call_concat(&mut state, &arguments).unwrap(),
            vec![string(expected)]
        );
    }

    let empty = indexed_table(&mut state, []);
    assert_eq!(call_concat(&mut state, &[empty]).unwrap(), vec![string("")]);
}

#[test]
fn concat_converts_numbers_and_preserves_arbitrary_bytes() {
    let mut state = installed_state();
    let values = indexed_table(
        &mut state,
        [
            (1, Value::Integer(-12)),
            (2, Value::Float(3.5)),
            (3, string([0x00, 0x80, 0xff])),
        ],
    );

    assert_eq!(
        call_concat(&mut state, &[values, string([0xc3, 0xa9])]).unwrap(),
        vec![string([
            b'-', b'1', b'2', 0xc3, 0xa9, b'3', b'.', b'5', 0xc3, 0xa9, 0x00, 0x80, 0xff,
        ])]
    );

    let values = indexed_table(&mut state, [(1, string("left")), (2, string("right"))]);
    assert_eq!(
        call_concat(&mut state, &[values, Value::Integer(10)]).unwrap(),
        vec![string("left10right")]
    );
}

#[test]
fn concat_does_not_clamp_explicit_indices_to_the_sequence_length() {
    let mut state = installed_state();
    let values = indexed_table(
        &mut state,
        [
            (-1, string("negative")),
            (0, string("zero")),
            (1, string("positive")),
        ],
    );

    assert_eq!(
        call_concat(
            &mut state,
            &[values, string(","), Value::Integer(-1), Value::Integer(1),],
        )
        .unwrap(),
        vec![string("negative,zero,positive")]
    );
}

#[test]
fn concat_uses_len_and_index_metamethods() {
    assert_eq!(
        execute_table_test(
            r##"
                local backing = {"first", "second", "third"}
                local proxy
                proxy = setmetatable({}, {
                    __len = function(...)
                        assert(select("#", ...) == 2)
                        local first, second = ...
                        assert(first == proxy and second == proxy)
                        return 3
                    end,
                    __index = backing,
                })
                return table.concat(proxy, ":")
            "##,
        )
        .unwrap(),
        vec![string("first:second:third")]
    );
}

#[test]
fn concat_uses_the_intrinsic_length_of_strings() {
    let mut state = installed_loader_state();
    let target = string("abc");
    let metatable = state.get_metatable(&target).unwrap().unwrap();
    let Value::Table(index) = state.raw_get(&metatable, &string("__index")).unwrap() else {
        panic!("the string metatable did not contain an index table");
    };

    for (position, value) in [(1, "first"), (2, "second"), (3, "third")] {
        state
            .raw_set(&index, &Value::Integer(position), &string(value))
            .unwrap();
    }

    let misleading_length = execute_in_state(&mut state, "return function() return 1 end")
        .unwrap()
        .remove(0);
    state
        .raw_set(&metatable, &string("__len"), &misleading_length)
        .unwrap();

    assert_eq!(
        call_concat(&mut state, &[target, string(":")]).unwrap(),
        vec![string("first:second:third")]
    );
}

#[test]
fn concat_accepts_a_non_table_with_read_and_length_capabilities() {
    let mut state = installed_loader_state();
    let proxy = Value::Boolean(false);
    let backing = indexed_table(
        &mut state,
        [
            (1, string("first")),
            (2, string("second")),
            (3, string("third")),
        ],
    );
    let length = execute_in_state(&mut state, "return function() return 3 end")
        .unwrap()
        .remove(0);
    let metatable = state.create_table(0, 2).unwrap();

    state
        .raw_set(&metatable, &string("__index"), &backing)
        .unwrap();
    state
        .raw_set(&metatable, &string("__len"), &length)
        .unwrap();
    state.set_metatable(&proxy, Some(&metatable)).unwrap();

    assert_eq!(
        call_concat(&mut state, &[proxy, string(":")]).unwrap(),
        vec![string("first:second:third")]
    );
}

#[test]
fn concat_validates_proxy_capabilities_only_once() {
    let mut state = installed_loader_state();
    let proxy = Value::Boolean(false);
    let backing = indexed_table(&mut state, [(1, string("value"))]);

    let length = execute_in_state(
        &mut state,
        r#"
            return function(value)
                getmetatable(value).__len = nil
                return 1
            end
        "#,
    )
    .unwrap()
    .remove(0);
    let metatable = state.create_table(0, 2).unwrap();
    state
        .raw_set(&metatable, &string("__index"), &backing)
        .unwrap();
    state
        .raw_set(&metatable, &string("__len"), &length)
        .unwrap();
    state.set_metatable(&proxy, Some(&metatable)).unwrap();

    assert_eq!(
        call_concat(&mut state, std::slice::from_ref(&proxy)).unwrap(),
        vec![string("value")]
    );

    let index = execute_in_state(
        &mut state,
        r#"
            return function(value)
                getmetatable(value).__index = nil
                return "value"
            end
        "#,
    )
    .unwrap()
    .remove(0);
    let length = execute_in_state(&mut state, "return function() return 1 end")
        .unwrap()
        .remove(0);
    let metatable = state.create_table(0, 2).unwrap();
    state
        .raw_set(&metatable, &string("__index"), &index)
        .unwrap();
    state
        .raw_set(&metatable, &string("__len"), &length)
        .unwrap();
    state.set_metatable(&proxy, Some(&metatable)).unwrap();

    assert_eq!(
        call_concat(&mut state, &[proxy]).unwrap(),
        vec![string("value")]
    );
}

#[test]
fn concat_rejects_non_tables_missing_a_required_capability() {
    let mut state = installed_loader_state();
    let proxy = Value::Boolean(false);
    let backing = indexed_table(&mut state, [(1, string("value"))]);
    let length = execute_in_state(&mut state, "return function() return 1 end")
        .unwrap()
        .remove(0);

    for (metamethod, value) in [("__index", backing), ("__len", length)] {
        let metatable = state.create_table(0, 1).unwrap();
        state
            .raw_set(&metatable, &string(metamethod), &value)
            .unwrap();
        state.set_metatable(&proxy, Some(&metatable)).unwrap();

        assert_concat_error(
            call_concat(&mut state, std::slice::from_ref(&proxy)).unwrap_err(),
            "bad argument #1 to 'concat' (table expected, got boolean)",
        );
    }
}

#[test]
fn concat_reports_first_argument_errors() {
    let mut state = installed_state();

    for (arguments, expected) in [
        (
            vec![],
            "bad argument #1 to 'concat' (table expected, got no value)",
        ),
        (
            vec![Value::Boolean(true)],
            "bad argument #1 to 'concat' (table expected, got boolean)",
        ),
        (
            vec![string("not a table")],
            "bad argument #1 to 'concat' (table expected, got string)",
        ),
    ] {
        assert_concat_error(call_concat(&mut state, &arguments).unwrap_err(), expected);
    }
}

#[test]
fn concat_reports_separator_argument_errors() {
    let mut state = installed_state();
    let table = indexed_table(&mut state, []);

    for (value, expected) in [
        (
            Value::Boolean(false),
            "bad argument #2 to 'concat' (string expected, got boolean)",
        ),
        (
            table.clone(),
            "bad argument #2 to 'concat' (string expected, got table)",
        ),
    ] {
        assert_concat_error(
            call_concat(&mut state, &[table.clone(), value]).unwrap_err(),
            expected,
        );
    }
}

#[test]
fn concat_reports_index_argument_errors() {
    let mut state = installed_state();
    let table = indexed_table(&mut state, []);

    for (arguments, expected) in [
        (
            vec![table.clone(), string(""), Value::Boolean(false)],
            "bad argument #3 to 'concat' (number expected, got boolean)",
        ),
        (
            vec![table.clone(), string(""), Value::Float(1.5)],
            "bad argument #3 to 'concat' (number has no integer representation)",
        ),
        (
            vec![
                table.clone(),
                string(""),
                Value::Integer(1),
                Value::Boolean(false),
            ],
            "bad argument #4 to 'concat' (number expected, got boolean)",
        ),
        (
            vec![
                table.clone(),
                string(""),
                Value::Integer(1),
                Value::Float(1.5),
            ],
            "bad argument #4 to 'concat' (number has no integer representation)",
        ),
    ] {
        assert_concat_error(call_concat(&mut state, &arguments).unwrap_err(), expected);
    }
}

#[test]
fn concat_reports_the_type_and_index_of_invalid_elements() {
    let mut state = installed_state();

    for (value, type_name) in [
        (Value::Nil, "nil"),
        (Value::Boolean(false), "boolean"),
        (indexed_table(&mut state, []), "table"),
    ] {
        let values = indexed_table(
            &mut state,
            [(1, string("first")), (2, value), (3, string("third"))],
        );
        let error = call_concat(
            &mut state,
            &[values, string(","), Value::Integer(1), Value::Integer(3)],
        )
        .unwrap_err();

        assert_concat_error(
            error,
            &format!("invalid value ({type_name}) at index 2 in table for 'concat'"),
        );
    }
}

#[test]
fn install_registers_table_unpack() {
    let mut state = installed_state();
    let Value::Table(table_library) = state.get_global(b"table").unwrap() else {
        panic!("table was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&table_library, &string("unpack")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn unpack_returns_default_and_explicit_ranges() {
    let mut state = installed_state();
    let values = indexed_table(
        &mut state,
        [
            (1, string("first")),
            (2, string("second")),
            (3, string("third")),
        ],
    );

    assert_eq!(
        call_unpack(&mut state, std::slice::from_ref(&values)).unwrap(),
        vec![string("first"), string("second"), string("third")]
    );
    assert_eq!(
        call_unpack(&mut state, &[values, Value::Integer(2), Value::Integer(3)],).unwrap(),
        vec![string("second"), string("third")]
    );
}

#[test]
fn unpack_preserves_nil_values_and_accepts_empty_ranges() {
    let mut state = installed_state();
    let values = indexed_table(&mut state, [(1, string("first")), (3, string("third"))]);

    assert_eq!(
        call_unpack(&mut state, &[values, Value::Integer(1), Value::Integer(3)],).unwrap(),
        vec![string("first"), Value::Nil, string("third")]
    );
    assert_eq!(
        call_unpack(
            &mut state,
            &[Value::Boolean(false), Value::Integer(2), Value::Integer(1)],
        )
        .unwrap(),
        Vec::<Value>::new()
    );
}

#[test]
fn unpack_uses_len_and_index_metamethods() {
    assert_eq!(
        execute_table_test(
            r##"
                local backing = {"first", "second", "third"}
                local proxy
                proxy = setmetatable({}, {
                    __len = function(...)
                        assert(select("#", ...) == 2)
                        local first, second = ...
                        assert(first == proxy and second == proxy)
                        return 3
                    end,
                    __index = backing,
                })
                return table.unpack(proxy)
            "##,
        )
        .unwrap(),
        vec![string("first"), string("second"), string("third")]
    );
}

#[test]
fn unpack_accepts_non_tables_with_index_and_length_metamethods() {
    let mut state = installed_loader_state();
    let proxy = Value::Boolean(false);
    let backing = indexed_table(
        &mut state,
        [
            (1, string("first")),
            (2, string("second")),
            (3, string("third")),
        ],
    );
    let length = execute_in_state(&mut state, "return function() return 3 end")
        .unwrap()
        .remove(0);
    let metatable = state.create_table(0, 2).unwrap();

    state
        .raw_set(&metatable, &string("__index"), &backing)
        .unwrap();
    state
        .raw_set(&metatable, &string("__len"), &length)
        .unwrap();
    state.set_metatable(&proxy, Some(&metatable)).unwrap();

    assert_eq!(
        call_unpack(&mut state, &[proxy]).unwrap(),
        vec![string("first"), string("second"), string("third")]
    );
}

#[test]
fn unpack_with_an_explicit_end_only_requires_indexing() {
    let mut state = installed_state();
    let proxy = Value::Boolean(false);
    let backing = indexed_table(&mut state, [(1, string("first")), (2, string("second"))]);
    let metatable = state.create_table(0, 1).unwrap();

    state
        .raw_set(&metatable, &string("__index"), &backing)
        .unwrap();
    state.set_metatable(&proxy, Some(&metatable)).unwrap();

    assert_eq!(
        call_unpack(&mut state, &[proxy, Value::Integer(1), Value::Integer(2)],).unwrap(),
        vec![string("first"), string("second")]
    );
}

#[test]
fn unpack_uses_the_intrinsic_length_of_strings() {
    let mut state = installed_state();

    assert_eq!(
        call_unpack(&mut state, &[string("abc")]).unwrap(),
        vec![Value::Nil, Value::Nil, Value::Nil]
    );
}

#[test]
fn unpack_reports_index_argument_errors() {
    let mut state = installed_state();
    let table = indexed_table(&mut state, []);

    for (arguments, expected) in [
        (
            vec![table.clone(), Value::Boolean(false)],
            "bad argument #2 to 'unpack' (number expected, got boolean)",
        ),
        (
            vec![table.clone(), Value::Float(1.5)],
            "bad argument #2 to 'unpack' (number has no integer representation)",
        ),
        (
            vec![table.clone(), Value::Integer(1), Value::Boolean(false)],
            "bad argument #3 to 'unpack' (number expected, got boolean)",
        ),
        (
            vec![table.clone(), Value::Integer(1), Value::Float(1.5)],
            "bad argument #3 to 'unpack' (number has no integer representation)",
        ),
    ] {
        assert_unpack_error(call_unpack(&mut state, &arguments).unwrap_err(), expected);
    }
}

#[test]
fn unpack_reports_default_length_errors() {
    let mut state = installed_state();

    assert_unpack_error(
        call_unpack(&mut state, &[Value::Boolean(false)]).unwrap_err(),
        "attempt to get length of a boolean value",
    );
}

#[test]
fn unpack_rejects_excessive_result_counts() {
    let mut state = installed_state();
    let table = indexed_table(&mut state, []);

    assert_unpack_error(
        call_unpack(
            &mut state,
            &[table, Value::Integer(0), Value::Integer(1_000_000)],
        )
        .unwrap_err(),
        "too many results to unpack",
    );
}
