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

fn call_rep(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(string_library) = state.get_global(b"string")? else {
        panic!("string was not installed as a table");
    };
    let Value::Function(rep) = state.raw_get(&string_library, &string("rep"))? else {
        panic!("string.rep was not installed as a function");
    };

    match state.call(&rep, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("string.rep unexpectedly yielded"),
    }
}

fn assert_rep_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "string.rep"
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
fn install_registers_string_rep() {
    let mut state = installed_state();
    let Value::Table(string_library) = state.get_global(b"string").unwrap() else {
        panic!("string was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&string_library, &string("rep")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn rep_repeats_the_string_and_treats_non_positive_counts_as_empty() {
    let mut state = installed_state();

    for (arguments, expected) in [
        (vec![string("ab"), Value::Integer(3)], "ababab"),
        (vec![string("ab"), Value::Integer(1)], "ab"),
        (vec![string("ab"), Value::Integer(0)], ""),
        (vec![string("ab"), Value::Integer(-4)], ""),
        (vec![string(""), Value::Integer(5)], ""),
        (vec![string("ab"), Value::Float(2.0)], "abab"),
    ] {
        assert_eq!(
            call_rep(&mut state, &arguments).unwrap(),
            vec![string(expected)]
        );
    }
}

#[test]
fn rep_inserts_the_separator_between_copies_only() {
    let mut state = installed_state();

    for (arguments, expected) in [
        (
            vec![string("ab"), Value::Integer(3), string("-")],
            "ab-ab-ab",
        ),
        (vec![string("ab"), Value::Integer(1), string("-")], "ab"),
        (vec![string("ab"), Value::Integer(3), string("")], "ababab"),
        (vec![string("ab"), Value::Integer(3), Value::Nil], "ababab"),
        (
            vec![string("ab"), Value::Integer(2), string("---")],
            "ab---ab",
        ),
    ] {
        assert_eq!(
            call_rep(&mut state, &arguments).unwrap(),
            vec![string(expected)]
        );
    }
}

#[test]
fn rep_coerces_numbers_to_strings() {
    let mut state = installed_state();

    for (arguments, expected) in [
        (
            vec![Value::Integer(12), Value::Integer(3), Value::Integer(0)],
            "12012012",
        ),
        (vec![Value::Float(1.5), Value::Integer(2)], "1.51.5"),
        (
            vec![string("ab"), Value::Integer(2), Value::Float(2.0)],
            "ab2.0ab",
        ),
    ] {
        assert_eq!(
            call_rep(&mut state, &arguments).unwrap(),
            vec![string(expected)]
        );
    }
}

#[test]
fn rep_preserves_arbitrary_byte_data() {
    let mut state = installed_state();

    assert_eq!(
        call_rep(
            &mut state,
            &[
                string([0xff, 0x00, b'a']),
                Value::Integer(2),
                string([0xc3, 0xa9])
            ]
        )
        .unwrap(),
        vec![string([0xff, 0x00, b'a', 0xc3, 0xa9, 0xff, 0x00, b'a'])]
    );
}

#[test]
fn rep_reports_lua_argument_errors() {
    let mut state = installed_state();
    let cases = [
        (
            vec![],
            "bad argument #1 to 'rep' (string expected, got no value)",
        ),
        (
            vec![Value::Boolean(true), Value::Integer(1)],
            "bad argument #1 to 'rep' (string expected, got boolean)",
        ),
        (
            vec![string("ab")],
            "bad argument #2 to 'rep' (number expected, got no value)",
        ),
        (
            vec![string("ab"), Value::Boolean(true)],
            "bad argument #2 to 'rep' (number expected, got boolean)",
        ),
        (
            vec![string("ab"), Value::Float(1.5)],
            "bad argument #2 to 'rep' (number has no integer representation)",
        ),
        (
            vec![string("ab"), Value::Integer(2), Value::Boolean(true)],
            "bad argument #3 to 'rep' (string expected, got boolean)",
        ),
    ];

    for (arguments, expected) in cases {
        let error = call_rep(&mut state, &arguments).unwrap_err();
        assert_rep_error(error, expected);
    }
}

#[test]
fn rep_rejects_results_that_are_too_large() {
    let mut state = installed_state();

    for arguments in [
        // The multiplication overflows usize.
        vec![string("abc"), Value::Integer(i64::MAX)],
        // The total fits usize but exceeds the maximum allocation size.
        vec![string("ab"), Value::Integer(i64::MAX)],
        // A separator contributes to the total length.
        vec![string("ab"), Value::Integer(i64::MAX), string("-")],
    ] {
        let error = call_rep(&mut state, &arguments).unwrap_err();
        assert_rep_error(error, "resulting string too large");
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

fn call_find(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(string_library) = state.get_global(b"string")? else {
        panic!("string was not installed as a table");
    };
    let Value::Function(find) = state.raw_get(&string_library, &string("find"))? else {
        panic!("string.find was not installed as a function");
    };

    match state.call(&find, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("string.find unexpectedly yielded"),
    }
}

fn find(state: &mut State, subject: impl AsRef<[u8]>, pattern: impl AsRef<[u8]>) -> Vec<Value> {
    call_find(state, &[string(subject), string(pattern)]).unwrap()
}

fn assert_find_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "string.find"
    ));
}

#[test]
fn find_searches_plain_substrings_and_reports_one_based_inclusive_positions() {
    let mut state = installed_state();

    assert_eq!(
        find(&mut state, "hello world", "o w"),
        vec![Value::Integer(5), Value::Integer(7)]
    );
    assert_eq!(find(&mut state, "hello", "xyz"), vec![Value::Nil]);
    assert_eq!(
        call_find(
            &mut state,
            &[string("hello"), string("l"), Value::Integer(4)]
        )
        .unwrap(),
        vec![Value::Integer(4), Value::Integer(4)]
    );
    assert_eq!(
        call_find(
            &mut state,
            &[string("hello"), string("l"), Value::Integer(5)]
        )
        .unwrap(),
        vec![Value::Nil]
    );
}

#[test]
fn find_matches_the_empty_pattern_at_the_initial_position() {
    let mut state = installed_state();

    for (init, expected) in [
        (None, vec![Value::Integer(1), Value::Integer(0)]),
        (Some(3), vec![Value::Integer(3), Value::Integer(2)]),
        (Some(4), vec![Value::Integer(4), Value::Integer(3)]),
        (Some(5), vec![Value::Nil]),
        (Some(-1), vec![Value::Integer(3), Value::Integer(2)]),
    ] {
        let mut arguments = vec![string("abc"), string("")];
        if let Some(init) = init {
            arguments.push(Value::Integer(init));
        }
        assert_eq!(
            call_find(&mut state, &arguments).unwrap(),
            expected,
            "init {init:?}"
        );
    }
}

#[test]
fn find_honours_positive_negative_and_out_of_range_init_positions() {
    let mut state = installed_state();

    for (init, expected) in [
        (-2, vec![Value::Integer(4), Value::Integer(4)]),
        (-100, vec![Value::Integer(3), Value::Integer(3)]),
        (i64::MIN, vec![Value::Integer(3), Value::Integer(3)]),
        (5, vec![Value::Nil]),
        (i64::MAX, vec![Value::Nil]),
    ] {
        assert_eq!(
            call_find(
                &mut state,
                &[string("hello"), string("l"), Value::Integer(init)]
            )
            .unwrap(),
            expected,
            "init {init}"
        );
    }

    // An init past the end fails even for patterns that match empty.
    for init in [5, i64::MAX] {
        assert_eq!(
            call_find(
                &mut state,
                &[string("abc"), string("%a*"), Value::Integer(init)]
            )
            .unwrap(),
            vec![Value::Nil],
            "init {init}"
        );
    }
}

#[test]
fn find_with_plain_flag_treats_the_pattern_as_a_literal() {
    let mut state = installed_state();

    assert_eq!(
        call_find(
            &mut state,
            &[
                string("hello"),
                string("l+"),
                Value::Integer(1),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        vec![Value::Nil]
    );
    assert_eq!(
        call_find(
            &mut state,
            &[
                string("a.c"),
                string("%."),
                Value::Integer(1),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        vec![Value::Nil]
    );
    assert_eq!(
        call_find(
            &mut state,
            &[
                string("a.c"),
                string("."),
                Value::Integer(1),
                Value::Boolean(true)
            ]
        )
        .unwrap(),
        vec![Value::Integer(2), Value::Integer(2)]
    );
}

#[test]
fn find_without_magic_characters_uses_a_literal_search() {
    let mut state = installed_state();

    // ')' and ']' are not in the magic set, so these never reach the
    // pattern engine (where a bare ')' would be an error).
    assert_eq!(
        find(&mut state, "a)b", ")b"),
        vec![Value::Integer(2), Value::Integer(3)]
    );
    assert_eq!(
        find(&mut state, "a]b", "]"),
        vec![Value::Integer(2), Value::Integer(2)]
    );
}

#[test]
fn find_supports_character_classes_and_their_complements() {
    let mut state = installed_state();

    for (subject, pattern, expected) in [
        ("abc123", "%d+", vec![Value::Integer(4), Value::Integer(6)]),
        ("abc123", "%a+", vec![Value::Integer(1), Value::Integer(3)]),
        ("abc def", "%S+", vec![Value::Integer(1), Value::Integer(3)]),
        ("a1", "%A", vec![Value::Integer(2), Value::Integer(2)]),
        (" \t\nx", "%s+", vec![Value::Integer(1), Value::Integer(3)]),
        ("1Fz", "%x+", vec![Value::Integer(1), Value::Integer(2)]),
        ("HELLO", "%u+", vec![Value::Integer(1), Value::Integer(5)]),
        ("hello", "%l+", vec![Value::Integer(1), Value::Integer(5)]),
        ("h3llo", "%w+", vec![Value::Integer(1), Value::Integer(5)]),
        ("(a)", "%p", vec![Value::Integer(1), Value::Integer(1)]),
        ("a\nb", ".", vec![Value::Integer(1), Value::Integer(1)]),
        ("abc", "%d", vec![Value::Nil]),
        ("abc", "%U+", vec![Value::Integer(1), Value::Integer(3)]),
    ] {
        assert_eq!(find(&mut state, subject, pattern), expected, "{pattern:?}");
    }
}

#[test]
fn find_supports_sets_ranges_negation_and_set_escapes() {
    let mut state = installed_state();

    for (subject, pattern, expected) in [
        (
            "abcdefg",
            "[cd]",
            vec![Value::Integer(3), Value::Integer(3)],
        ),
        (
            "xyzabc",
            "[a-c]+",
            vec![Value::Integer(4), Value::Integer(6)],
        ),
        (
            "  abc",
            "[^%s]+",
            vec![Value::Integer(3), Value::Integer(5)],
        ),
        (
            "abc123",
            "[^0-9]+",
            vec![Value::Integer(1), Value::Integer(3)],
        ),
        // ']' as the first set character is a literal.
        ("a]b", "[]]", vec![Value::Integer(2), Value::Integer(2)]),
        // '-' as the last set character is a literal.
        ("a-b", "[c-]", vec![Value::Integer(2), Value::Integer(2)]),
        // Escapes inside sets.
        ("a%b", "[%%]", vec![Value::Integer(2), Value::Integer(2)]),
        ("[x]", "[]%[]", vec![Value::Integer(1), Value::Integer(1)]),
        ("axb", "[^]]+", vec![Value::Integer(1), Value::Integer(3)]),
    ] {
        assert_eq!(find(&mut state, subject, pattern), expected, "{pattern:?}");
    }
}

#[test]
fn find_supports_greedy_lazy_and_optional_repetition() {
    let mut state = installed_state();

    for (subject, pattern, expected) in [
        ("<a><b>", "<.*>", vec![Value::Integer(1), Value::Integer(6)]),
        ("<a><b>", "<.->", vec![Value::Integer(1), Value::Integer(3)]),
        ("hello", "l+", vec![Value::Integer(3), Value::Integer(4)]),
        ("abbbbc", "ab+c", vec![Value::Integer(1), Value::Integer(6)]),
        ("abc", "ab+c", vec![Value::Integer(1), Value::Integer(3)]),
        ("ac", "ab+c", vec![Value::Nil]),
        (
            "color",
            "colou?r",
            vec![Value::Integer(1), Value::Integer(5)],
        ),
        (
            "colour",
            "colou?r",
            vec![Value::Integer(1), Value::Integer(6)],
        ),
        // Zero repetitions match the empty string.
        ("xyz", "a*", vec![Value::Integer(1), Value::Integer(0)]),
        ("aaab", "a*b", vec![Value::Integer(1), Value::Integer(4)]),
    ] {
        assert_eq!(find(&mut state, subject, pattern), expected, "{pattern:?}");
    }
}

#[test]
fn find_supports_anchors() {
    let mut state = installed_state();

    assert_eq!(find(&mut state, "abc", "^b"), vec![Value::Nil]);
    assert_eq!(
        call_find(
            &mut state,
            &[string("abc"), string("^b"), Value::Integer(2)]
        )
        .unwrap(),
        vec![Value::Integer(2), Value::Integer(2)]
    );
    // An anchor with nothing after it matches the empty string at init.
    assert_eq!(
        call_find(&mut state, &[string("abc"), string("^"), Value::Integer(3)]).unwrap(),
        vec![Value::Integer(3), Value::Integer(2)]
    );
    assert_eq!(
        find(&mut state, "abc", "c$"),
        vec![Value::Integer(3), Value::Integer(3)]
    );
    assert_eq!(find(&mut state, "abc", "b$"), vec![Value::Nil]);
    // '$' alone matches the empty string at the end of the subject.
    assert_eq!(
        find(&mut state, "abc", "$"),
        vec![Value::Integer(4), Value::Integer(3)]
    );
    // '$' anywhere but at the end of the pattern is a literal.
    assert_eq!(
        find(&mut state, "a$b", "$b"),
        vec![Value::Integer(2), Value::Integer(3)]
    );
}

#[test]
fn find_supports_escaped_magic_characters() {
    let mut state = installed_state();

    for (subject, pattern, expected) in [
        ("a.c", "%.", vec![Value::Integer(2), Value::Integer(2)]),
        ("100%", "%%", vec![Value::Integer(4), Value::Integer(4)]),
        ("(a)", "%(a%)", vec![Value::Integer(1), Value::Integer(3)]),
        ("a+b", "a%+b", vec![Value::Integer(1), Value::Integer(3)]),
        // Escaped letters that are not classes match literally (PUC behavior).
        ("q", "%q", vec![Value::Integer(1), Value::Integer(1)]),
    ] {
        assert_eq!(find(&mut state, subject, pattern), expected, "{pattern:?}");
    }
}

#[test]
fn find_returns_captures_after_the_match_positions() {
    let mut state = installed_state();

    assert_eq!(
        find(&mut state, "key=value", "(%a+)=(%a+)"),
        vec![
            Value::Integer(1),
            Value::Integer(9),
            string("key"),
            string("value"),
        ]
    );
    // Captures may be empty.
    assert_eq!(
        find(&mut state, "abc", "(%a)(%d*)"),
        vec![
            Value::Integer(1),
            Value::Integer(1),
            string("a"),
            string(""),
        ]
    );
    // Nested captures are returned outermost first.
    assert_eq!(
        find(&mut state, "abc", "((a)(b))"),
        vec![
            Value::Integer(1),
            Value::Integer(2),
            string("ab"),
            string("a"),
            string("b"),
        ]
    );
}

#[test]
fn find_supports_position_captures_relative_to_the_subject_start() {
    let mut state = installed_state();

    assert_eq!(
        find(&mut state, "key=val", "()(=)"),
        vec![
            Value::Integer(4),
            Value::Integer(4),
            Value::Integer(4),
            string("="),
        ]
    );
    // Position captures count from the start of the subject, not from init.
    assert_eq!(
        call_find(
            &mut state,
            &[string("hello world"), string("()o()"), Value::Integer(5)]
        )
        .unwrap(),
        vec![
            Value::Integer(5),
            Value::Integer(5),
            Value::Integer(5),
            Value::Integer(6),
        ]
    );
}

#[test]
fn find_supports_balanced_matches() {
    let mut state = installed_state();

    assert_eq!(
        find(&mut state, "{nested {braces}}", "%b{}"),
        vec![Value::Integer(1), Value::Integer(17)]
    );
    assert_eq!(find(&mut state, "{unclosed", "%b{}"), vec![Value::Nil]);
    assert_eq!(
        find(&mut state, "f(a(b)c)", "%b()"),
        vec![Value::Integer(2), Value::Integer(8)]
    );
}

#[test]
fn find_supports_frontier_patterns() {
    let mut state = installed_state();

    assert_eq!(
        find(&mut state, "THE (quick) fox", "%f[%a]%u+"),
        vec![Value::Integer(1), Value::Integer(3)]
    );
    assert_eq!(
        find(&mut state, "the (quick) brown", "%f[%l]%l+"),
        vec![Value::Integer(1), Value::Integer(3)]
    );
    // The frontier is zero-width and can match at the end of the subject.
    assert_eq!(
        find(&mut state, "hello", "%f[%A]"),
        vec![Value::Integer(6), Value::Integer(5)]
    );
}

#[test]
fn find_supports_back_references() {
    let mut state = installed_state();

    assert_eq!(
        find(&mut state, "abcabc", "(abc)%1"),
        vec![Value::Integer(1), Value::Integer(6), string("abc")]
    );
    assert_eq!(find(&mut state, "abcabd", "(abc)%1"), vec![Value::Nil]);
}

#[test]
fn find_reports_malformed_patterns() {
    let mut state = installed_state();
    let cases = [
        ("%", "malformed pattern (ends with '%')"),
        ("[a", "malformed pattern (missing ']')"),
        ("%a)", "invalid pattern capture"),
        ("(a", "unfinished capture"),
        ("%0", "invalid capture index"),
        ("(a%1)", "invalid capture index"),
        ("%b", "missing arguments to '%b' in pattern"),
        ("%bx", "missing arguments to '%b' in pattern"),
        ("%f", "missing '[' after '%f' in pattern"),
        ("%fa", "missing '[' after '%f' in pattern"),
    ];

    for (pattern, expected) in cases {
        let error = call_find(&mut state, &[string("abc"), string(pattern)]).unwrap_err();
        assert_find_error(error, expected);
    }
}

#[test]
fn find_limits_captures_and_pattern_complexity() {
    let mut state = installed_state();

    let error = call_find(&mut state, &[string("a"), string("(".repeat(33))]).unwrap_err();
    assert_find_error(error, "too many captures");

    let error = call_find(
        &mut state,
        &[string("a".repeat(300)), string("a?".repeat(300))],
    )
    .unwrap_err();
    assert_find_error(error, "pattern too complex");
}

#[test]
fn find_operates_on_raw_bytes() {
    let mut state = installed_state();

    assert_eq!(
        find(&mut state, b"a\0b", b"\0"),
        vec![Value::Integer(2), Value::Integer(2)]
    );
    assert_eq!(
        find(&mut state, b"a\0b", "."),
        vec![Value::Integer(1), Value::Integer(1)]
    );
    assert_eq!(
        find(&mut state, [0xff, 0xfe], [0xfe]),
        vec![Value::Integer(2), Value::Integer(2)]
    );
    // Captures preserve arbitrary bytes.
    assert_eq!(
        find(&mut state, b"a\0b", "a(.)b"),
        vec![Value::Integer(1), Value::Integer(3), string(b"\0")]
    );
}

#[test]
fn arithmetic_coerces_numeric_strings() {
    let values = execute_string_test(
        r#"
            return
                "2" + 1,
                1 + "2",
                "10" - "3",
                "6" * "7",
                "7" / "2",
                "7" // "2",
                "7.0" // "2",
                "7" % "3",
                "2" ^ "3",
                " 3e0 " + "2",
                " -0xa " + 1
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(3),
            Value::Integer(3),
            Value::Integer(7),
            Value::Integer(42),
            Value::Float(3.5),
            Value::Integer(3),
            Value::Float(3.0),
            Value::Integer(1),
            Value::Float(8.0),
            Value::Float(5.0),
            Value::Integer(-9),
        ]
    );

    assert_eq!(
        execute_string_test(r#"return -"10", -"10.5", -" -0xa ""#).unwrap(),
        vec![Value::Integer(-10), Value::Float(-10.5), Value::Integer(10),]
    );

    let error = execute_string_test(r#"return -"not numeric""#).unwrap_err();
    assert_eq!(
        error.kind,
        VmErrorKind::InvalidNegateOperand { kind: "string" }
    );
}

#[test]
fn failed_string_coercion_tries_the_right_metamethod() {
    let values = execute_string_test(
        r#"
            local right
            right = setmetatable({}, {
                __add = function(left, actual_right)
                    assert(left == "not numeric")
                    assert(actual_right == right)
                    return 77, 88
                end,
            })

            return "not numeric" + right
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Integer(77)]);
}

#[test]
fn non_arithmetic_operators_and_invalid_strings_do_not_coerce() {
    assert_eq!(
        execute_string_test(r#"return "1" == 1"#).unwrap(),
        vec![Value::Boolean(false)]
    );

    let error = execute_string_test(r#"return "3" & 1"#).unwrap_err();
    assert_eq!(
        error.kind.to_string(),
        "attempt to perform bitwise operation on a string value (constant '3')"
    );

    let error = execute_string_test(r#"return "1" < 1"#).unwrap_err();
    assert_eq!(
        error.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: "<",
            left: "string",
            right: "number",
        }
    );

    let error = execute_string_test(r#"return "not numeric" + 1"#).unwrap_err();
    assert_eq!(
        error.kind,
        VmErrorKind::InvalidAddOperands {
            left: "string",
            right: "number",
        }
    );
}

fn call_gmatch(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Table(string_library) = state.get_global(b"string")? else {
        panic!("string was not installed as a table");
    };
    let Value::Function(gmatch) = state.raw_get(&string_library, &string("gmatch"))? else {
        panic!("string.gmatch was not installed as a function");
    };

    match state.call(&gmatch, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("string.gmatch unexpectedly yielded"),
    }
}

fn assert_gmatch_error(error: VmError, expected: &str) {
    assert_eq!(
        error.kind,
        VmErrorKind::NativeFunctionFailure {
            message: expected.into(),
        }
    );
    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "string.gmatch"
    ));
}

#[test]
fn install_registers_string_gmatch() {
    let mut state = installed_state();
    let Value::Table(string_library) = state.get_global(b"string").unwrap() else {
        panic!("string was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&string_library, &string("gmatch")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn gmatch_iterators_return_whole_matches_and_keep_independent_state() {
    assert_eq!(
        execute_string_test(
            r##"
                local first = string.gmatch("one two", "%w+")
                local second = string.gmatch("x y", "%w+")

                local first_one = first()
                local second_one = second()
                local first_two = first()
                local second_two = second()
                local first_done = select("#", first())
                local second_done = select("#", second())

                return
                    first ~= second,
                    first_one, second_one,
                    first_two, second_two,
                    first_done, second_done
            "##,
        )
        .unwrap(),
        vec![
            Value::Boolean(true),
            string("one"),
            string("x"),
            string("two"),
            string("y"),
            Value::Integer(0),
            Value::Integer(0),
        ]
    );
}

#[test]
fn gmatch_works_as_a_generic_for_iterator() {
    assert_eq!(
        execute_string_test(
            r#"
                local result = ""
                for word in string.gmatch("first second word", "%w+") do
                    result = result .. "[" .. word .. "]"
                end
                return result
            "#,
        )
        .unwrap(),
        vec![string("[first][second][word]")]
    );
}

#[test]
fn gmatch_returns_captures_instead_of_the_whole_match() {
    assert_eq!(
        execute_string_test(
            r##"
                local iterator = string.gmatch(
                    "from=world, to=Lua",
                    "(%w+)=(%w+)"
                )
                local key_one, value_one = iterator()
                local key_two, value_two = iterator()
                local done = select("#", iterator())

                return key_one, value_one, key_two, value_two, done
            "##,
        )
        .unwrap(),
        vec![
            string("from"),
            string("world"),
            string("to"),
            string("Lua"),
            Value::Integer(0),
        ]
    );
}

#[test]
fn gmatch_position_and_empty_matches_make_progress_without_duplicates() {
    assert_eq!(
        execute_string_test(
            r##"
                local positions = string.gmatch("abc", "()")
                local p1, p2, p3, p4 =
                    positions(), positions(), positions(), positions()
                local positions_done = select("#", positions())

                local stars = string.gmatch("ba", "a*")
                local first, second = stars(), stars()
                local stars_done = select("#", stars())

                return
                    p1, p2, p3, p4, positions_done,
                    first, second, stars_done
            "##,
        )
        .unwrap(),
        vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(4),
            Value::Integer(0),
            string(""),
            string("a"),
            Value::Integer(0),
        ]
    );
}

#[test]
fn gmatch_honours_positive_negative_and_past_end_init_positions() {
    assert_eq!(
        execute_string_test(
            r##"
                local positive = string.gmatch("10 20 30", "%d+", 3)
                local p1, p2 = positive(), positive()
                local positive_done = select("#", positive())

                local negative = string.gmatch("11 21 31", "%d+", -4)
                local n1, n2 = negative(), negative()
                local negative_done = select("#", negative())

                local at_end = string.gmatch("11 21 31", "%w*", 9)
                local end_match = at_end()
                local at_end_done = select("#", at_end())

                local past_end = string.gmatch("11 21 31", "%w*", 10)
                local past_end_done = select("#", past_end())

                return
                    p1, p2, positive_done,
                    n1, n2, negative_done,
                    end_match, at_end_done, past_end_done
            "##,
        )
        .unwrap(),
        vec![
            string("20"),
            string("30"),
            Value::Integer(0),
            string("1"),
            string("31"),
            Value::Integer(0),
            string(""),
            Value::Integer(0),
            Value::Integer(0),
        ]
    );
}

#[test]
fn gmatch_treats_a_leading_caret_as_a_literal() {
    assert_eq!(
        execute_string_test(
            r##"
                local absent = string.gmatch("ab", "^.")
                local literal = string.gmatch("^a x ^b", "^.")

                return
                    select("#", absent()),
                    literal(),
                    literal(),
                    select("#", literal())
            "##,
        )
        .unwrap(),
        vec![
            Value::Integer(0),
            string("^a"),
            string("^b"),
            Value::Integer(0),
        ]
    );
}

#[test]
fn gmatch_operates_on_raw_bytes() {
    assert_eq!(
        execute_string_test(
            r##"
                local iterator = string.gmatch("\255\0a", "(.)")
                local first, second, third =
                    iterator(), iterator(), iterator()
                local done = select("#", iterator())
                return first, second, third, done
            "##,
        )
        .unwrap(),
        vec![
            string([0xff]),
            string([0x00]),
            string("a"),
            Value::Integer(0),
        ]
    );
}

#[test]
fn gmatch_reports_constructor_argument_errors() {
    let mut state = installed_state();

    for (arguments, expected) in [
        (
            vec![],
            "bad argument #1 to 'gmatch' (string expected, got no value)",
        ),
        (
            vec![string("abc")],
            "bad argument #2 to 'gmatch' (string expected, got no value)",
        ),
        (
            vec![Value::Table(state.create_table(0, 0).unwrap()), string("a")],
            "bad argument #1 to 'gmatch' (string expected, got table)",
        ),
        (
            vec![
                string("abc"),
                Value::Table(state.create_table(0, 0).unwrap()),
            ],
            "bad argument #2 to 'gmatch' (string expected, got table)",
        ),
        (
            vec![
                string("abc"),
                string("a"),
                Value::Table(state.create_table(0, 0).unwrap()),
            ],
            "bad argument #3 to 'gmatch' (number expected, got table)",
        ),
        (
            vec![string("abc"), string("a"), Value::Float(1.5)],
            "bad argument #3 to 'gmatch' (number has no integer representation)",
        ),
    ] {
        assert_gmatch_error(call_gmatch(&mut state, &arguments).unwrap_err(), expected);
    }
}

#[test]
fn gmatch_reports_malformed_patterns_when_the_iterator_runs() {
    let values = execute_string_test(
        r#"
            local iterator = string.gmatch("abc", "%")
            local ok, message = pcall(iterator)
            return type(iterator), ok, message
        "#,
    )
    .unwrap();

    assert_eq!(values[0], string("function"));
    assert_eq!(values[1], Value::Boolean(false));
    assert_string_contains(&values[2], b"malformed pattern (ends with '%')");
}

#[test]
fn gmatch_iterator_keeps_its_captures_alive_across_collection() {
    assert_eq!(
        execute_string_test(
            r#"
                local iterator
                do
                    local subject = "left right"
                    local pattern = "%w+"
                    iterator = string.gmatch(subject, pattern)
                end

                collectgarbage("collect")
                return iterator(), iterator()
            "#,
        )
        .unwrap(),
        vec![string("left"), string("right")]
    );
}
