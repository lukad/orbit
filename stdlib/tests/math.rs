use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{CallOutcome, LuaString, State, Value, VmErrorKind, VmResult, VmTraceFrame};

fn string(value: &str) -> Value {
    Value::String(LuaString::from(value))
}

fn installed_state() -> State {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state
}

fn execute(state: &mut State, source: &str) -> VmResult<Vec<Value>> {
    let function = state.load_buffer("math-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("math test unexpectedly yielded"),
    }
}

#[test]
fn install_registers_math_extrema() {
    let mut state = installed_state();

    let Value::Table(math) = state.get_global(b"math").unwrap() else {
        panic!("math was not installed as a table");
    };

    assert!(matches!(
        state.raw_get(&math, &string("max")).unwrap(),
        Value::Function(_)
    ));
    assert!(matches!(
        state.raw_get(&math, &string("min")).unwrap(),
        Value::Function(_)
    ));
}

#[test]
fn extrema_return_primitive_values_and_preserve_their_types() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            return
                math.max(7),
                math.max(-5, 2, 1),
                math.max(1.5, 2, 1.75),
                math.max("alpha", "omega", "beta"),
                math.min(7),
                math.min(-5, 2, 1),
                math.min(1.5, 2, 1.75),
                math.min("alpha", "omega", "beta")
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(7),
            Value::Integer(2),
            Value::Integer(2),
            string("omega"),
            Value::Integer(7),
            Value::Integer(-5),
            Value::Float(1.5),
            string("alpha"),
        ]
    );
}

#[test]
fn max_dispatches_to_lt_in_candidate_order_and_keeps_the_first_tie() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local calls = {}
            local metatable = {
                __lt = function(left, right)
                    calls[#calls + 1] = { left = left, right = right }
                    return left.rank < right.rank
                end,
            }

            local first = setmetatable({ rank = 1 }, metatable)
            local largest = setmetatable({ rank = 3 }, metatable)
            local last = setmetatable({ rank = 2 }, metatable)
            local tied = setmetatable({ rank = 3 }, metatable)

            local maximum = math.max(first, largest, last, tied)

            return
                maximum == largest,
                #calls,
                calls[1].left == first and calls[1].right == largest,
                calls[2].left == largest and calls[2].right == last,
                calls[3].left == largest and calls[3].right == tied
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Integer(3),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn max_uses_the_right_lt_metamethod_and_booleanizes_its_result() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local left = setmetatable({}, {})
            local right
            local called_with_original_operands = false

            right = setmetatable({}, {
                __lt = function(actual_left, actual_right)
                    called_with_original_operands =
                        actual_left == left and actual_right == right
                    return "truthy"
                end,
            })

            return math.max(left, right) == right, called_with_original_operands
        "#,
    )
    .unwrap();

    assert_eq!(values, vec![Value::Boolean(true), Value::Boolean(true)]);
}

#[test]
fn min_dispatches_to_lt_with_the_candidate_first_and_keeps_the_first_tie() {
    let mut state = installed_state();

    let values = execute(
        &mut state,
        r#"
            local calls = {}
            local metatable = {
                __lt = function(left, right)
                    calls[#calls + 1] = { left = left, right = right }
                    return left.rank < right.rank
                end,
            }

            local first = setmetatable({ rank = 3 }, metatable)
            local smallest = setmetatable({ rank = 1 }, metatable)
            local last = setmetatable({ rank = 2 }, metatable)
            local tied = setmetatable({ rank = 1 }, metatable)

            local minimum = math.min(first, smallest, last, tied)

            return
                minimum == smallest,
                #calls,
                calls[1].left == smallest and calls[1].right == first,
                calls[2].left == last and calls[2].right == smallest,
                calls[3].left == tied and calls[3].right == smallest
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Integer(3),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}

#[test]
fn extrema_report_missing_and_incomparable_arguments() {
    let mut state = installed_state();

    let missing = execute(&mut state, "return math.max()").unwrap_err();
    assert_eq!(
        missing.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'max' (value expected)".into(),
        }
    );
    assert!(matches!(
        missing.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.max"
    ));

    let incomparable = execute(&mut state, "return math.max(1, false)").unwrap_err();
    assert!(matches!(
        incomparable.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: "<",
            left: "number",
            right: "boolean",
        }
    ));
    assert!(incomparable.frames.iter().any(
        |frame| matches!(frame, VmTraceFrame::Native { name } if name.as_ref() == "math.max")
    ));

    let missing = execute(&mut state, "return math.min()").unwrap_err();
    assert_eq!(
        missing.kind,
        VmErrorKind::NativeFunctionFailure {
            message: "bad argument #1 to 'min' (value expected)".into(),
        }
    );
    assert!(matches!(
        missing.frames.first(),
        Some(VmTraceFrame::Native { name }) if name.as_ref() == "math.min"
    ));

    let incomparable = execute(&mut state, "return math.min(1, false)").unwrap_err();
    assert!(matches!(
        incomparable.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: "<",
            left: "boolean",
            right: "number",
        }
    ));
    assert!(incomparable.frames.iter().any(
        |frame| matches!(frame, VmTraceFrame::Native { name } if name.as_ref() == "math.min")
    ));
}
