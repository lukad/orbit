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

fn call_unpack(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(string_library) = state.get_global(b"string")? else {
        panic!("string was not installed as a table");
    };
    let Value::Function(unpack) = state.raw_get(&string_library, &string("unpack"))? else {
        panic!("string.unpack was not installed as a function");
    };

    match state.call(&unpack, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("string.unpack unexpectedly yielded"),
    }
}

fn call_format(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(string_library) = state.get_global(b"string")? else {
        panic!("string was not installed as a table");
    };
    let Value::Function(format) = state.raw_get(&string_library, &string("format"))? else {
        panic!("string.format was not installed as a function");
    };

    match state.call(&format, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("string.format unexpectedly yielded"),
    }
}

fn execute_string_test(source: &str) -> VmResult<Vec<Value>> {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    let function = state.load_buffer("string-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("string test unexpectedly yielded"),
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

#[test]
fn unpack_decodes_integer_formats_and_returns_the_next_position() {
    let mut state = installed_state();

    assert_eq!(
        call_unpack(
            &mut state,
            &[
                string("<i2>I2bB"),
                string([0x34, 0x12, 0x12, 0x34, 0xff, 0xfe]),
            ],
        )
        .unwrap(),
        vec![
            Value::Integer(0x1234),
            Value::Integer(0x1234),
            Value::Integer(-1),
            Value::Integer(0xfe),
            Value::Integer(7),
        ],
    );
}

#[test]
fn unpack_decodes_floats_and_strings() {
    let mut state = installed_state();
    let mut numbers = Vec::new();
    numbers.extend_from_slice(&1.5f32.to_le_bytes());
    numbers.extend_from_slice(&(-2.25f64).to_be_bytes());

    assert_eq!(
        call_unpack(&mut state, &[string("<f>d"), string(numbers)]).unwrap(),
        vec![Value::Float(1.5), Value::Float(-2.25), Value::Integer(13),],
    );

    assert_eq!(
        call_unpack(&mut state, &[string("c3s1z"), string(b"abc\x03xyzhi\0")],).unwrap(),
        vec![
            string("abc"),
            string("xyz"),
            string("hi"),
            Value::Integer(11),
        ],
    );
}

#[test]
fn unpack_honors_alignment_and_relative_positions() {
    let mut state = installed_state();

    assert_eq!(
        call_unpack(
            &mut state,
            &[
                string("<!4 i4"),
                string([0, 0, 0, 0, 42, 0, 0, 0]),
                Value::Integer(2),
            ],
        )
        .unwrap(),
        vec![Value::Integer(42), Value::Integer(9)],
    );

    assert_eq!(
        call_unpack(
            &mut state,
            &[
                string("<i2"),
                string([0, 0, 0x34, 0x12]),
                Value::Integer(-2),
            ],
        )
        .unwrap(),
        vec![Value::Integer(0x1234), Value::Integer(5)],
    );
}

#[test]
fn unpack_reports_short_data_unfinished_strings_and_invalid_positions() {
    let mut state = installed_state();
    let cases = [
        (
            vec![string("i4"), string([0, 0, 0])],
            "bad argument #2 to 'unpack' (data string too short)",
        ),
        (
            vec![string("z"), string("unterminated")],
            "bad argument #2 to 'unpack' (unfinished string for format 'z')",
        ),
        (
            vec![string("c0"), string("abc"), Value::Integer(5)],
            "bad argument #3 to 'unpack' (initial position out of string)",
        ),
    ];

    for (arguments, expected) in cases {
        let error = call_unpack(&mut state, &arguments).unwrap_err();
        assert_eq!(
            error.kind,
            VmErrorKind::NativeFunctionFailure {
                message: expected.into(),
            }
        );
        assert!(matches!(
            error.frames.first(),
            Some(VmTraceFrame::Native { name }) if name.as_ref() == "string.unpack"
        ));
    }
}

#[test]
fn format_applies_integer_float_and_string_modifiers() {
    let mut state = installed_state();

    assert_eq!(
        call_format(
            &mut state,
            &[
                string("|%5d|%-5d|%+05d|%#x|%.0d|%u|%.2f|%.3s|"),
                Value::Integer(12),
                Value::Integer(12),
                Value::Integer(12),
                Value::Integer(42),
                Value::Integer(0),
                Value::Integer(-1),
                Value::Float(1.25),
                string("hello"),
            ],
        )
        .unwrap(),
        vec![string(
            "|   12|12   |+0012|0x2a||18446744073709551615|1.25|hel|"
        )],
    );
}

#[test]
fn format_matches_lua_numeric_edge_cases() {
    let mut state = installed_state();

    for (format, value, expected) in [
        ("%#12o", Value::Integer(10), "         012"),
        ("%#10x", Value::Integer(100), "      0x64"),
        ("%#-17X", Value::Integer(100), "0X64             "),
        ("%013i", Value::Integer(-100), "-000000000100"),
        ("%2.5d", Value::Integer(-100), "-00100"),
        ("%.u", Value::Integer(0), ""),
        ("%+#014.0f", Value::Integer(100), "+000000000100."),
        ("%-16c", Value::Integer(97), "a               "),
        ("%+.3G", Value::Float(1.5), "+1.5"),
        ("%a", Value::Float(1.5), "0x1.8p+0"),
        ("%A", Value::Float(1.5), "0X1.8P+0"),
        ("%.2A", Value::Integer(12), "0X1.80P+3"),
        ("%#.0a", Value::Float(0.0), "0x0.p+0"),
        ("%020a", Value::Float(1.5), "0x0000000000001.8p+0"),
        ("%a", Value::Float(f64::from_bits(1)), "0x1p-1074"),
    ] {
        assert_eq!(
            call_format(&mut state, &[string(format), value]).unwrap(),
            vec![string(expected)],
            "format {format}",
        );
    }
}

#[test]
fn format_preserves_binary_strings_without_modifiers() {
    let mut state = installed_state();

    assert_eq!(
        call_format(&mut state, &[string(b"before:%s:after"), string(b"a\0b")],).unwrap(),
        vec![string(b"before:a\0b:after")],
    );

    let error = call_format(&mut state, &[string(b"%3s"), string(b"a\0b")]).unwrap_err();
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #2 to 'format' (string contains zeros)".into(),
        }
    );
}

#[test]
fn format_string_conversion_uses_tostring_and_its_metamethod() {
    assert_eq!(
        execute_string_test(
            r#"
                local value = setmetatable({}, {
                    __tostring = function()
                        return "converted"
                    end
                })
                return string.format("<%12s>", value)
            "#,
        )
        .unwrap(),
        vec![string("<   converted>")],
    );
}

#[test]
fn format_flattens_multiple_tostring_results_in_order() {
    assert_eq!(
        execute_string_test(
            r#"
                local calls = 0
                local metatable = {
                    __tostring = function()
                        calls = calls + 1
                        if calls == 1 then
                            return "first"
                        else
                            return "second"
                        end
                    end
                }
                local first = setmetatable({}, metatable)
                local second = setmetatable({}, metatable)
                return string.format(
                    "prefix:%s:middle:%10s:suffix",
                    first,
                    second
                ), calls
            "#,
        )
        .unwrap(),
        vec![
            string("prefix:first:middle:    second:suffix"),
            Value::Integer(2),
        ],
    );
}

#[test]
fn format_preserves_lua_coercion_and_validation_order() {
    let values = execute_string_test(
        r#"
            local called = false
            local value = setmetatable({}, {
                __tostring = function()
                    called = true
                    return "converted"
                end
            })
            local ok, message = pcall(string.format, "%+s", value)
            return ok, called, message
        "#,
    )
    .unwrap();

    assert_eq!(values[0], Value::Boolean(false));
    assert_eq!(values[1], Value::Boolean(true));
    assert_string_contains(&values[2], b"invalid conversion specification");

    let values = execute_string_test(
        r#"
            local value = setmetatable({}, {
                __tostring = function()
                    return "a\0b"
                end
            })
            local ok, message = pcall(string.format, "%0.s", value)
            return ok, message
        "#,
    )
    .unwrap();

    assert_eq!(values[0], Value::Boolean(false));
    assert_string_contains(&values[1], b"string contains zeros");

    let values = execute_string_test(
        r#"
            local integer_ok, integer_error =
                pcall(string.format, "%+x", {})
            local character_ok, character_error =
                pcall(string.format, "%+c", {})
            local float_ok, float_error =
                pcall(string.format, "%100f", {})
            local hex_float_ok, hex_float_error =
                pcall(string.format, "%100a", {})
            return
                integer_ok, integer_error,
                character_ok, character_error,
                float_ok, float_error,
                hex_float_ok, hex_float_error
        "#,
    )
    .unwrap();

    for index in [0, 2, 4, 6] {
        assert_eq!(values[index], Value::Boolean(false));
    }
    assert_string_contains(&values[1], b"number expected");
    assert_string_contains(&values[3], b"invalid conversion specification");
    assert_string_contains(&values[5], b"number expected");
    assert_string_contains(&values[7], b"invalid conversion specification");
}

#[test]
fn format_quote_produces_loadable_lua_values() {
    assert_eq!(
        execute_string_test(
            r#"
                local source = string.format(
                    "return %q, %q, %q, %q, %q",
                    nil,
                    true,
                    -9223372036854775807 - 1,
                    1.5,
                    "quote=\" slash=\\ newline=\n nul=\0 digit=\0012"
                )
                return load(source)()
            "#,
        )
        .unwrap(),
        vec![
            Value::Nil,
            Value::Boolean(true),
            Value::Integer(i64::MIN),
            Value::Float(1.5),
            string(b"quote=\" slash=\\ newline=\n nul=\0 digit=\x012"),
        ],
    );
}

#[test]
fn format_pointer_is_stable_for_the_same_object_and_null_for_primitives() {
    let values = execute_string_test(
        r#"
            local first = {}
            local second = {}
            local first_pointer = string.format("%p", first)
            return
                first_pointer == string.format("%p", first),
                first_pointer ~= string.format("%p", second),
                string.format("%p", nil),
                string.format("|%20p|", first)
        "#,
    )
    .unwrap();

    assert_eq!(values[0], Value::Boolean(true));
    assert_eq!(values[1], Value::Boolean(true));
    assert_eq!(values[2], string("(null)"));
    let Value::String(padded) = &values[3] else {
        panic!("formatted pointer was not a string");
    };
    assert_eq!(padded.len(), 22);
    assert!(padded.as_bytes().starts_with(b"|  0x"));
    assert!(padded.as_bytes().ends_with(b"|"));
}

#[test]
fn format_pointer_uses_lua_short_string_identity_rules() {
    let mut state = installed_state();
    let first_short = string(vec![b's'; 40]);
    let second_short = string(vec![b's'; 40]);
    let first_long = string(vec![b'l'; 41]);
    let second_long = string(vec![b'l'; 41]);

    let first_short_pointer = call_format(&mut state, &[string("%p"), first_short]).unwrap();
    let second_short_pointer = call_format(&mut state, &[string("%p"), second_short]).unwrap();
    let first_long_pointer = call_format(&mut state, &[string("%p"), first_long]).unwrap();
    let second_long_pointer = call_format(&mut state, &[string("%p"), second_long]).unwrap();

    assert_eq!(first_short_pointer, second_short_pointer);
    assert_ne!(first_long_pointer, second_long_pointer);
}

fn assert_string_contains(value: &Value, expected: &[u8]) {
    let Value::String(value) = value else {
        panic!("expected a string, got {value:?}");
    };

    assert!(
        value
            .as_bytes()
            .windows(expected.len())
            .any(|window| window == expected),
        "{value:?} does not contain {expected:?}",
    );
}
