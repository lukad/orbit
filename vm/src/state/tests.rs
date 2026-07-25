use orbit_common::{SourceId, Span};
use orbit_compiler::bytecode::{
    BinaryOp, Chunk, ImmediateOperandSide, Instruction, SourceMapEntry,
};
use orbit_parser::{lexer::lex, parser::parse_chunk};

use crate::{
    error::{LuaTraceFunction, VmError, VmErrorKind, VmResult, VmTraceFrame},
    loading::{LoadError, LoadService, LoadSource, NoLoadService},
    native::{NativeAction, NativeCallback, NativeContext, NativeEvent, NativeToken},
    value::Value,
};

use super::{CallOutcome, State};

const YIELD_TOKEN: NativeToken = NativeToken::new(1);

const CONTINUATION_TOKEN: NativeToken = NativeToken::new(2);

const GARBAGE_TABLE_COUNT: usize = 1_100;

fn compile_source(source_id: SourceId, source: &str) -> Chunk {
    let tokens = lex(source_id, source).unwrap();
    let ast = parse_chunk(source_id, tokens).unwrap();
    let hir = orbit_resolver::resolve(&ast).unwrap();

    orbit_compiler::compile(hir).unwrap()
}

struct WrongSourceIdLoadService;

impl LoadService for WrongSourceIdLoadService {
    fn compile(&mut self, source_id: SourceId, source: LoadSource<'_>) -> Result<Chunk, LoadError> {
        if !matches!(source, LoadSource::Buffer { .. }) {
            return Err(LoadError::DynamicLoadingDisabled { source_id });
        }

        let wrong_source_id = SourceId::new(source_id.get().saturating_add(1));
        Ok(compile_source(wrong_source_id, "return 42"))
    }

    fn file_exists(&self, _filename: &[u8]) -> bool {
        false
    }
}

struct WrongErrorSourceIdLoadService;

impl LoadService for WrongErrorSourceIdLoadService {
    fn compile(&mut self, source_id: SourceId, source: LoadSource<'_>) -> Result<Chunk, LoadError> {
        if !matches!(source, LoadSource::Buffer { .. }) {
            return Err(LoadError::DynamicLoadingDisabled { source_id });
        }

        let actual = SourceId::new(source_id.get().saturating_add(1));
        Err(LoadError::InvalidUtf8 {
            span: Span::new(actual, 0, 1),
        })
    }

    fn file_exists(&self, _filename: &[u8]) -> bool {
        false
    }
}

fn execute_chunk(state: &mut State, chunk: Chunk) -> VmResult<Vec<Value>> {
    let function = state.load_chunk(chunk)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => {
            panic!("ordinary test chunk unexpectedly yielded")
        }
    }
}

fn execute_in_state(state: &mut State, source_id: SourceId, source: &str) -> VmResult<Vec<Value>> {
    execute_chunk(state, compile_source(source_id, source))
}

fn execute_source(source: &str) -> VmResult<Vec<Value>> {
    let mut state = State::new(NoLoadService)?;
    execute_in_state(&mut state, SourceId::new(0), source)
}

fn assert_execute(source: &str, expected: Vec<Value>) {
    let actual = execute_source(source).unwrap();
    assert_eq!(actual, expected, "source:\n{source}");
}

fn string_value(value: &str) -> Value {
    Value::String(crate::string::LuaString::from(value))
}

fn source_span(source_id: SourceId, source: &str, needle: &str) -> Span {
    let start = source.find(needle).unwrap();

    Span::new(
        source_id,
        u32::try_from(start).unwrap(),
        u32::try_from(start + needle.len()).unwrap(),
    )
}

fn yield_once(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => Ok(context.yield_values([context.integer(1)], YIELD_TOKEN)),

        NativeEvent::Resume { token: YIELD_TOKEN } => {
            let value = context
                .resume_value(0)
                .and_then(|value| value.as_integer())
                .ok_or_else(|| native_failure("resume value must be an integer"))?;

            Ok(context.return_values([context.integer(value + 1)]))
        }

        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
}

fn allocate_garbage(context: &mut NativeContext<'_>) -> VmResult<()> {
    for _ in 0..GARBAGE_TABLE_COUNT {
        context.create_table(0, 0)?;
    }

    Ok(())
}

fn continuation_parent(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let callee = context
                .argument(0)
                .ok_or_else(|| native_failure("missing continuation callee"))?;

            let collect_before_call = context
                .argument(1)
                .and_then(|value| value.as_boolean())
                .ok_or_else(|| native_failure("missing collection phase"))?;

            let protected = context.create_table(0, 1)?;
            let key = context.string("answer");
            let answer = context.integer(42);
            context.raw_set(&protected, key, answer)?;

            if collect_before_call {
                allocate_garbage(context)?;
            }

            Ok(context.call_with_continuation(callee, [], [protected], CONTINUATION_TOKEN))
        }

        NativeEvent::Resume {
            token: CONTINUATION_TOKEN,
        }
        | NativeEvent::ResumeError {
            token: CONTINUATION_TOKEN,
        } => {
            let protected = context
                .continuation_value(0)
                .ok_or_else(|| native_failure("missing protected continuation value"))?;

            let key = context.string("answer");
            let answer = context.raw_get(&protected, &key)?;

            Ok(context.return_values([answer]))
        }

        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
}

fn return_immediately(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    Ok(context.return_values([]))
}

fn collect_then_return(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    allocate_garbage(context)?;
    Ok(context.return_values([]))
}

fn collect_then_error(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    allocate_garbage(context)?;
    Err(native_failure("expected nested failure"))
}

fn native_failure(message: impl Into<Box<str>>) -> VmError {
    VmErrorKind::NativeFunctionFailure {
        message: message.into(),
    }
    .into()
}

fn run_automatic_continuation_collection_case(
    child_callback: NativeCallback,
    collect_before_call: bool,
) {
    let mut state = State::new(NoLoadService).unwrap();

    let parent = state
        .create_native_function("continuation parent", continuation_parent, &[])
        .unwrap();

    let child = state
        .create_native_function("continuation child", child_callback, &[])
        .unwrap();

    let outcome = state
        .call(
            &parent,
            &[Value::Function(child), Value::Boolean(collect_before_call)],
        )
        .unwrap();

    let CallOutcome::Returned(values) = outcome else {
        panic!("continuation case unexpectedly yielded");
    };

    assert_eq!(values, vec![Value::Integer(42)]);
}

fn returned_function(state: &mut State, source_id: SourceId, source: &str) -> crate::Function {
    let values = execute_in_state(state, source_id, source).unwrap();

    let [Value::Function(function)] = values.as_slice() else {
        panic!("chunk should return exactly one function");
    };

    function.clone()
}

#[test]
fn loads_and_calls_a_function_through_public_values() {
    let mut state = State::new(NoLoadService).unwrap();

    let function = state
        .load_chunk(compile_source(
            SourceId::new(0),
            "local value = ...; return value + 1",
        ))
        .unwrap();

    let outcome = state.call(&function, &[Value::Integer(41)]).unwrap();

    let CallOutcome::Returned(values) = outcome else {
        panic!("ordinary Lua call unexpectedly yielded");
    };

    assert_eq!(values, vec![Value::Integer(42)]);
}

#[test]
fn rejects_dynamic_chunks_tagged_with_a_different_source_identifier() {
    let mut state = State::new(WrongSourceIdLoadService).unwrap();

    let error = state.load_buffer(b"ignored.lua", b"return 1").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::LoadFailure(LoadError::UnexpectedSourceId {
            expected: SourceId::new(0),
            actual: SourceId::new(1),
        })
    );
}

#[test]
fn rejects_dynamic_errors_tagged_with_a_different_source_identifier() {
    let mut state = State::new(WrongErrorSourceIdLoadService).unwrap();

    let error = state.load_buffer(b"ignored.lua", b"ignored").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::LoadFailure(LoadError::UnexpectedSourceId {
            expected: SourceId::new(0),
            actual: SourceId::new(1),
        })
    );
}

#[test]
fn reads_and_writes_globals_and_tables() {
    let mut state = State::new(NoLoadService).unwrap();
    let table = state.create_table(0, 0).unwrap();

    state
        .raw_set(&table, &Value::Integer(1), &Value::Integer(42))
        .unwrap();

    assert_eq!(
        state.raw_get(&table, &Value::Integer(1)).unwrap(),
        Value::Integer(42),
    );

    assert_eq!(state.raw_len(&table).unwrap(), 1);

    state
        .set_global(b"values", &Value::Table(table.clone()))
        .unwrap();

    assert_eq!(state.get_global(b"values").unwrap(), Value::Table(table),);
}

#[test]
fn exports_and_resumes_a_yielded_call() {
    let mut state = State::new(NoLoadService).unwrap();

    let native = state
        .create_native_function("yield_once", yield_once, &[])
        .unwrap();

    state
        .set_global(b"yield_once", &Value::Function(native))
        .unwrap();

    let function = state
        .load_chunk(compile_source(SourceId::new(0), "return yield_once()"))
        .unwrap();

    let outcome = state.call(&function, &[]).unwrap();

    let CallOutcome::Yielded { values, suspension } = outcome else {
        panic!("call should yield");
    };

    assert_eq!(values, vec![Value::Integer(1)]);

    let outcome = suspension.resume(&[Value::Integer(41)]).unwrap();

    let CallOutcome::Returned(values) = outcome else {
        panic!("resumed call unexpectedly yielded again");
    };

    assert_eq!(values, vec![Value::Integer(42)]);
}

#[test]
fn rejects_a_function_from_another_state() {
    let mut first = State::new(NoLoadService).unwrap();

    let function = first
        .load_chunk(compile_source(SourceId::new(0), "return 1"))
        .unwrap();

    let first_state = function.state_id();

    let mut second = State::new(NoLoadService).unwrap();
    let second_state = second.globals().unwrap().state_id();

    let error = match second.call(&function, &[]) {
        Ok(_) => {
            panic!("foreign function call unexpectedly succeeded")
        }
        Err(error) => error,
    };

    assert_eq!(
        error.kind,
        VmErrorKind::ForeignObject {
            kind: "function",
            expected_state: second_state.get(),
            actual_state: first_state.get(),
        },
    );
}

#[test]
fn executes_unary_operators_with_lua_truthiness() {
    assert_execute(
        r#"
            return not nil, not false, not 0, #"orbit", -(-5), ~5
        "#,
        vec![
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Integer(5),
            Value::Integer(5),
            Value::Integer(-6),
        ],
    );
}

#[test]
fn executes_integer_and_mixed_numeric_arithmetic() {
    assert_execute(
        r#"
            return 7 + 3, 7 - 10, 6 * 7, 7 / 2, -7 // 3, -7 % 3, 2 ^ 3
        "#,
        vec![
            Value::Integer(10),
            Value::Integer(-3),
            Value::Integer(42),
            Value::Float(3.5),
            Value::Integer(-3),
            Value::Integer(2),
            Value::Float(8.0),
        ],
    );
}

#[test]
fn executes_bitwise_operators_and_reversed_shifts() {
    assert_execute(
        r#"
            return 6 & 3, 6 | 3, 6 ~ 3, 1 << 4, 16 >> 2, 8 << -1, 8 >> -1,
                3.0 & 1
        "#,
        vec![
            Value::Integer(2),
            Value::Integer(7),
            Value::Integer(5),
            Value::Integer(16),
            Value::Integer(4),
            Value::Integer(4),
            Value::Integer(16),
            Value::Integer(1),
        ],
    );
}

#[test]
fn concatenates_numbers_and_compares_mixed_numeric_values() {
    assert_execute(
        r#"
            return "orbit" .. 42 .. 3.5, 1 == 1.0, 2 ~= 3, 1 < 1.5,
                2.5 <= 2, "a" < "b", 3 >= 3.0
        "#,
        vec![
            string_value("orbit423.5"),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Boolean(true),
        ],
    );
}

#[test]
fn fused_small_integer_equality_branches_preserve_lua_number_semantics() {
    assert_execute(
        r#"
            local mixed = false
            local different = false

            if 0.0 == 0 then
                mixed = true
            end

            if "0" == 0 then
                different = true
            end

            return mixed, different
        "#,
        vec![Value::Boolean(true), Value::Boolean(false)],
    );
}

#[test]
fn short_circuit_operators_preserve_values_and_skip_operands() {
    assert_execute(
        r#"
            local calls = 0

            local function mark(value)
                calls = calls + 1
                return value
            end

            local a = false and mark(1)
            local b = true or mark(2)
            local c = nil or mark(3)
            local d = 0 and mark(4)

            if false then
                calls = 100
            elseif nil then
                calls = 200
            else
                calls = calls + 10
            end

            return a, b, c, d, calls
        "#,
        vec![
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Integer(3),
            Value::Integer(4),
            Value::Integer(12),
        ],
    );
}

#[test]
fn executes_while_repeat_and_break_control_flow() {
    assert_execute(
        r#"
            local index = 0
            local total = 0

            while index < 10 do
                index = index + 1
                if index == 4 then
                    break
                end
                total = total + index
            end

            repeat
                total = total + 1
            until total >= 10

            return index, total
        "#,
        vec![Value::Integer(4), Value::Integer(10)],
    );
}

#[test]
fn executes_ascending_descending_and_float_numeric_for_loops() {
    assert_execute(
        r#"
            local integer_total = 0
            for value = 1, 5 do
                integer_total = integer_total + value
            end
            for value = 5, 1, -2 do
                integer_total = integer_total + value
            end

            local float_total = 0.0
            for value = 0.5, 1.0, 0.25 do
                float_total = float_total + value
            end

            return integer_total, float_total
        "#,
        vec![Value::Integer(24), Value::Float(2.25)],
    );
}

#[test]
fn executes_generic_for_iterators_with_multiple_visible_values() {
    assert_execute(
        r#"
            local function iterator(limit, control)
                local next_value = control + 1
                if next_value > limit then
                    return nil
                end
                return next_value, next_value * next_value
            end

            local total = 0
            for index, square in iterator, 4, 0 do
                total = total + index + square
            end

            return total
        "#,
        vec![Value::Integer(40)],
    );
}

#[test]
fn table_reads_writes_deletes_and_canonicalizes_numeric_keys() {
    assert_execute(
        r#"
            local values = { 10, 20, name = "orbit", [true] = 30 }
            values[2] = nil
            values[1.0] = 11
            values[3] = values[1] + values[true]

            return values[1], values[2], values[3], values.name, values[0 / 0]
        "#,
        vec![
            Value::Integer(11),
            Value::Nil,
            Value::Integer(41),
            string_value("orbit"),
            Value::Nil,
        ],
    );
}

#[test]
fn tables_use_identity_for_table_keys_and_equality() {
    assert_execute(
        r#"
            local keys = {}
            local first = {}
            local second = {}

            keys[first] = "first"
            keys[second] = "second"

            return keys[first], keys[second], first == first, first == second
        "#,
        vec![
            string_value("first"),
            string_value("second"),
            Value::Boolean(true),
            Value::Boolean(false),
        ],
    );
}

#[test]
fn closes_captured_numeric_for_variables_between_iterations() {
    assert_execute(
        r#"
            local functions = {}

            for index = 1, 3 do
                functions[index] = function()
                    return index
                end
            end

            return functions[1](), functions[2](), functions[3]()
        "#,
        vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)],
    );
}

#[test]
fn globals_are_shared_with_nested_closures() {
    assert_execute(
        r#"
            answer = 41

            local function read_answer()
                return answer + 1
            end

            return read_answer(), missing
        "#,
        vec![Value::Integer(42), Value::Nil],
    );
}

#[test]
fn false_to_be_closed_values_are_noops() {
    assert_execute(
        r#"
            local value <close> = false
            return 42
        "#,
        vec![Value::Integer(42)],
    );
}

#[test]
fn fills_missing_parameters_with_nil() {
    assert_execute(
        r#"
            local function values(a, b, c)
                return a, b, c
            end

            return values(10)
        "#,
        vec![Value::Integer(10), Value::Nil, Value::Nil],
    );
}

#[test]
fn discards_extra_arguments_for_non_vararg_functions() {
    assert_execute(
        r#"
            local function first(value)
                return value
            end

            return first(10, 20, 30)
        "#,
        vec![Value::Integer(10)],
    );
}

#[test]
fn nil_fills_missing_fixed_results() {
    assert_execute(
        r#"
            local function one()
                return 7
            end

            local a, b, c = one()
            return a, b, c
        "#,
        vec![Value::Integer(7), Value::Nil, Value::Nil],
    );
}

#[test]
fn closures_share_mutated_upvalues() {
    assert_execute(
        r#"
            local value = 0

            local function increment()
                value = value + 1
                return value
            end

            return increment(), increment()
        "#,
        vec![Value::Integer(1), Value::Integer(2)],
    );
}

#[test]
fn nested_closures_forward_parent_upvalue_cells() {
    assert_execute(
        r#"
            local value = 42

            local function make_middle()
                return function()
                    return function()
                        return value
                    end
                end
            end

            local middle = make_middle()
            local inner = middle()

            return inner()
        "#,
        vec![Value::Integer(42)],
    );
}

#[test]
fn closing_a_scope_detaches_reused_registers() {
    assert_execute(
        r#"
            local first, second

            do
                local value = 1
                first = function()
                    return value
                end
            end

            do
                local value = 2
                second = function()
                    return value
                end
            end

            return first(), second()
        "#,
        vec![Value::Integer(1), Value::Integer(2)],
    );
}

#[test]
fn fixed_vararg_reads_nil_fill_missing_values() {
    assert_execute(
        r#"
            local function collect(first, ...)
                local a, b, c = ...
                return first, a, b, c
            end

            return collect(1, 2, 3)
        "#,
        vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
            Value::Nil,
        ],
    );
}

#[test]
fn open_vararg_returns_preserve_every_value() {
    assert_execute(
        r#"
            local function identity(...)
                return ...
            end

            return identity(1, 2, 3, 4)
        "#,
        vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
            Value::Integer(4),
        ],
    );
}

#[test]
fn open_results_become_outer_call_arguments() {
    assert_execute(
        r#"
            local function values()
                return 20, 22
            end

            local function add(left, right)
                return left + right
            end

            return add(values())
        "#,
        vec![Value::Integer(42)],
    );
}

#[test]
fn open_results_expand_the_final_table_list_field() {
    assert_execute(
        r#"
            local function values()
                return 2, 3
            end

            local result = { 1, values() }

            return result[1], result[2], result[3]
        "#,
        vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)],
    );
}

#[test]
fn calling_a_non_function_returns_an_error() {
    let error = execute_source("return (42)()").unwrap_err();

    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidCallOperand { kind: "number" }
    ));
}

#[test]
fn invalid_unary_operands_return_typed_errors() {
    let error = execute_source("return #true").unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidLengthOperand { kind: "boolean" }
    ));

    let error = execute_source("return -false").unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidNegateOperand { kind: "boolean" }
    ));

    let error = execute_source("return ~1.5").unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidBitwiseOperand { kind: "number" }
    ));
}

#[test]
fn invalid_binary_operands_return_typed_errors() {
    let error = execute_source(r#"return 1 + "value""#).unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidAddOperands {
            left: "number",
            right: "string"
        }
    ));

    let error = execute_source("return 1 & 1.5").unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidBitwiseOperands {
            operation: "bitwise and",
            left: "number",
            right: "number"
        }
    ));

    let error = execute_source("return true < false").unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: "<",
            left: "boolean",
            right: "boolean"
        }
    ));

    let error = execute_source("return 1 > false").unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: ">",
            left: "number",
            right: "boolean"
        }
    ));

    let error = execute_source("return false >= 1").unwrap_err();
    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidComparisonOperands {
            operation: ">=",
            left: "boolean",
            right: "number"
        }
    ));
}

#[test]
fn integer_floor_division_and_modulo_by_zero_return_errors() {
    let error = execute_source("return 1 // 0").unwrap_err();
    assert!(matches!(error.kind, VmErrorKind::IntegerDivisionByZero));

    let error = execute_source("return 1 % 0").unwrap_err();
    assert!(matches!(error.kind, VmErrorKind::IntegerModuloByZero));
}

#[test]
fn invalid_table_keys_return_errors() {
    let error = execute_source(
        r#"
            local values = {}
            values[nil] = 1
        "#,
    )
    .unwrap_err();
    assert!(matches!(error.kind, VmErrorKind::NilTableKey));

    let error = execute_source(
        r#"
            local values = {}
            values[0 / 0] = 1
        "#,
    )
    .unwrap_err();
    assert!(matches!(error.kind, VmErrorKind::NaNTableKey));
}

#[test]
fn zero_numeric_for_step_returns_an_error() {
    let error = execute_source(
        r#"
            for value = 1, 3, 0 do
            end
        "#,
    )
    .unwrap_err();

    assert!(matches!(error.kind, VmErrorKind::ZeroForStep));
}

#[test]
fn truthy_to_be_closed_values_report_unsupported_metamethods() {
    let error = execute_source(
        r#"
            local value <close> = true
        "#,
    )
    .unwrap_err();

    assert!(matches!(
        error.kind,
        VmErrorKind::UnsupportedToBeClosedLocal
    ));
}

#[test]
fn deep_lua_calls_use_the_explicit_vm_stack() {
    assert_execute(
        r#"
            local function descend(value)
                if value == 0 then
                    return 42
                end

                return descend(value - 1)
            end

            return descend(20000)
        "#,
        vec![Value::Integer(42)],
    );
}

#[test]
fn deep_runtime_errors_retain_only_bounded_traceback_sections() {
    let error = execute_source(
        r#"
            local function descend(depth)
                if depth == 0 then
                    return 1 + true
                end

                return 1 + descend(depth - 1)
            end

            return descend(100)
        "#,
    )
    .unwrap_err();

    let (head, skipped, tail) = error.traceback_sections();

    assert_eq!(error.frames.len(), 21);
    assert_eq!(head.len(), 10);
    assert_eq!(skipped, 80);
    assert_eq!(tail.len(), 11);
}

#[test]
fn runtime_errors_retain_exact_source_maps_across_chunks() {
    let mut state = State::new(NoLoadService).unwrap();
    let failing_source = "function failing()\n    return 1 + true\nend";
    let middle_source = "function middle()\n    return (failing())\nend";
    let calling_source = "return (middle())";

    {
        let defining_chunk = compile_source(SourceId::new(1), failing_source);
        assert!(matches!(
            defining_chunk.entry.children[0].code[1],
            Instruction::BinarySmallInt {
                op: BinaryOp::Add,
                side: ImmediateOperandSide::Left,
                ..
            }
        ));
        execute_chunk(&mut state, defining_chunk).unwrap();
    }

    {
        let middle_chunk = compile_source(SourceId::new(2), middle_source);
        assert!(matches!(
            middle_chunk.entry.children[0].code[3],
            Instruction::Call { .. }
        ));
        execute_chunk(&mut state, middle_chunk).unwrap();
    }

    let calling_chunk = compile_source(SourceId::new(3), calling_source);
    assert!(matches!(
        calling_chunk.entry.code[3],
        Instruction::Call { .. }
    ));
    let error = execute_chunk(&mut state, calling_chunk).unwrap_err();

    assert!(matches!(error.kind, VmErrorKind::InvalidAddOperands { .. }));
    assert_eq!(error.frames.len(), 3);

    let VmTraceFrame::Lua {
        function,
        function_span,
        pc,
        instruction_span,
    } = &error.frames[0]
    else {
        panic!("expected innermost Lua frame");
    };
    assert!(matches!(
        function,
        LuaTraceFunction::Named(name) if name.as_ref() == "failing"
    ));
    assert_eq!(function_span.source, SourceId::new(1));
    assert_eq!(*pc, 1);
    assert_eq!(
        *instruction_span,
        Some(source_span(SourceId::new(1), failing_source, "1 + true"))
    );

    let VmTraceFrame::Lua {
        function,
        function_span,
        pc,
        instruction_span,
    } = &error.frames[1]
    else {
        panic!("expected middle Lua frame");
    };
    assert!(matches!(
        function,
        LuaTraceFunction::Named(name) if name.as_ref() == "middle"
    ));
    assert_eq!(function_span.source, SourceId::new(2));
    assert_eq!(*pc, 3);
    assert_eq!(
        *instruction_span,
        Some(source_span(SourceId::new(2), middle_source, "failing()"))
    );

    let VmTraceFrame::Lua {
        function,
        function_span,
        pc,
        instruction_span,
    } = &error.frames[2]
    else {
        panic!("expected outermost Lua frame");
    };
    assert_eq!(function, &LuaTraceFunction::MainChunk);
    assert_eq!(function_span.source, SourceId::new(3));
    assert_eq!(*pc, 3);
    assert_eq!(
        *instruction_span,
        Some(source_span(SourceId::new(3), calling_source, "middle()"))
    );
}

#[test]
fn tail_calls_erase_intermediate_trace_frames() {
    let source = r#"
        local function failing()
            return 1 + true
        end

        local function middle()
            return failing()
        end

        return middle()
    "#;

    let error = execute_source(source).unwrap_err();

    assert!(matches!(error.kind, VmErrorKind::InvalidAddOperands { .. }));
    assert_eq!(error.frames.len(), 1);
    assert!(matches!(
        &error.frames[0],
        VmTraceFrame::Lua {
            function: LuaTraceFunction::Named(name),
            ..
        } if name.as_ref() == "failing"
    ));
}

#[test]
fn failed_tail_call_resolution_retains_the_calling_trace_frame() {
    let source = r#"
        local function invalid()
            return (nil)()
        end

        return invalid()
    "#;

    let error = execute_source(source).unwrap_err();

    assert!(matches!(
        error.kind,
        VmErrorKind::InvalidCallOperand { kind: "nil" }
    ));
    assert_eq!(error.frames.len(), 1);

    let VmTraceFrame::Lua {
        function,
        instruction_span,
        ..
    } = &error.frames[0]
    else {
        panic!("expected Lua trace frame");
    };

    assert!(matches!(
        function,
        LuaTraceFunction::Named(name) if name.as_ref() == "invalid"
    ));
    assert_eq!(
        *instruction_span,
        Some(source_span(SourceId::new(0), source, "(nil)()"))
    );
}

#[test]
fn empty_code_reports_a_source_map_free_trace_frame() {
    let source_id = SourceId::new(12);
    let span = Span::new(source_id, 0, 6);
    let mut chunk = compile_source(source_id, "return");

    chunk.entry.code = Box::new([]);
    chunk.entry.source_map = vec![SourceMapEntry { pc: 0, span }].into_boxed_slice();

    let mut state = State::new(NoLoadService).unwrap();
    let error = execute_chunk(&mut state, chunk).unwrap_err();

    assert!(matches!(
        error.kind,
        VmErrorKind::ProgramCounterOutOfBounds { pc: 0 }
    ));

    assert!(matches!(
        error.frames.as_ref(),
        [VmTraceFrame::Lua {
            pc: 0,
            instruction_span: None,
            ..
        }]
    ));
}

#[test]
fn debug_formatting_does_not_follow_table_cycles() {
    let values = execute_source(
        r#"
            local table = {}
            table.self = table
            table[table] = table
            return table
        "#,
    )
    .unwrap();

    let rendered = format!("{values:?}");

    assert!(rendered.contains("Table"));
    assert!(rendered.len() < 256);
}

#[test]
fn manual_collection_reclaims_unreachable_table_cycles() {
    let mut state = State::new(NoLoadService).unwrap();
    let first = state.create_table(0, 0).unwrap();
    let second = state.create_table(0, 0).unwrap();

    state
        .raw_set(&first, &string_value("next"), &Value::Table(second.clone()))
        .unwrap();

    state
        .raw_set(&second, &string_value("next"), &Value::Table(first.clone()))
        .unwrap();

    drop(first);
    drop(second);

    assert_eq!(state.collect_garbage().unwrap(), 2);
}

#[test]
fn manual_collection_keeps_live_external_handles() {
    let mut state = State::new(NoLoadService).unwrap();
    let table = state.create_table(0, 0).unwrap();

    assert_eq!(state.collect_garbage().unwrap(), 0);

    state
        .raw_set(&table, &Value::Integer(1), &Value::Integer(42))
        .unwrap();

    assert_eq!(
        state.raw_get(&table, &Value::Integer(1)).unwrap(),
        Value::Integer(42),
    );

    drop(table);

    assert_eq!(state.collect_garbage().unwrap(), 1);
}

#[test]
fn global_values_remain_reachable_until_removed() {
    let mut state = State::new(NoLoadService).unwrap();
    let table = state.create_table(0, 0).unwrap();

    state
        .set_global(b"kept", &Value::Table(table.clone()))
        .unwrap();

    drop(table);

    assert_eq!(state.collect_garbage().unwrap(), 0);
    assert!(matches!(
        state.get_global(b"kept").unwrap(),
        Value::Table(_)
    ));

    state.set_global(b"kept", &Value::Nil).unwrap();

    assert_eq!(state.collect_garbage().unwrap(), 1);
}

#[test]
fn suspended_calls_keep_execution_roots_during_collection() {
    let mut state = State::new(NoLoadService).unwrap();

    let first_garbage = state.create_table(0, 0).unwrap();
    let second_garbage = state.create_table(0, 0).unwrap();

    state
        .raw_set(
            &first_garbage,
            &Value::Integer(1),
            &Value::Table(second_garbage.clone()),
        )
        .unwrap();

    state
        .raw_set(
            &second_garbage,
            &Value::Integer(1),
            &Value::Table(first_garbage.clone()),
        )
        .unwrap();

    drop(first_garbage);
    drop(second_garbage);

    let native = state
        .create_native_function("yield_once", yield_once, &[])
        .unwrap();

    state
        .set_global(b"yield_once", &Value::Function(native))
        .unwrap();

    let function = state
        .load_chunk(compile_source(
            SourceId::new(0),
            r#"
                local protected = {}
                protected[protected] = 42
                yield_once()
                return protected[protected]
            "#,
        ))
        .unwrap();

    let CallOutcome::Yielded {
        values,
        mut suspension,
    } = state.call(&function, &[]).unwrap()
    else {
        panic!("call should yield");
    };

    assert_eq!(values, vec![Value::Integer(1)]);
    assert_eq!(suspension.collect_garbage().unwrap(), 2);

    let CallOutcome::Returned(values) = suspension.resume(&[Value::Integer(0)]).unwrap() else {
        panic!("resumed call unexpectedly yielded again");
    };

    assert_eq!(values, vec![Value::Integer(42)]);
}

#[test]
fn native_continuations_keep_values_alive_during_automatic_collection() {
    // Each case makes collection due in a different continuation-bearing
    // native state: WaitingForAction, Resume, and ResumeError respectively.
    run_automatic_continuation_collection_case(return_immediately, true);
    run_automatic_continuation_collection_case(collect_then_return, false);
    run_automatic_continuation_collection_case(collect_then_error, false);
}

#[test]
fn suspended_native_calls_keep_continuation_values_alive_during_collection() {
    let mut state = State::new(NoLoadService).unwrap();

    let parent = state
        .create_native_function("continuation parent", continuation_parent, &[])
        .unwrap();

    let child = state
        .create_native_function("yield once", yield_once, &[])
        .unwrap();

    let CallOutcome::Yielded {
        values,
        mut suspension,
    } = state
        .call(&parent, &[Value::Function(child), Value::Boolean(false)])
        .unwrap()
    else {
        panic!("nested native call should yield");
    };

    assert_eq!(values, vec![Value::Integer(1)]);
    suspension.collect_garbage().unwrap();

    let CallOutcome::Returned(values) = suspension.resume(&[Value::Integer(41)]).unwrap() else {
        panic!("resumed continuation unexpectedly yielded again");
    };

    assert_eq!(values, vec![Value::Integer(42)]);
}

#[test]
fn execution_collects_automatically_after_crossing_threshold() {
    let mut state = State::new(NoLoadService).unwrap();

    let function = state
        .load_chunk(compile_source(
            SourceId::new(0),
            r#"
                local survivor = {}
                survivor.answer = 42

                for index = 1, 1100 do
                    local garbage = {}
                    garbage.self = garbage
                end

                return survivor.answer
            "#,
        ))
        .unwrap();

    let CallOutcome::Returned(values) = state.call(&function, &[]).unwrap() else {
        panic!("ordinary call unexpectedly yielded");
    };

    assert_eq!(values, vec![Value::Integer(42)]);
    assert!(!state.runtime.collection_due());
}

#[test]
fn gets_sets_replaces_and_clears_table_metatables() {
    let mut state = State::new(NoLoadService).unwrap();
    let table = state.create_table(0, 0).unwrap();
    let first = state.create_table(0, 0).unwrap();
    let second = state.create_table(0, 0).unwrap();
    let table_value = Value::Table(table.clone());

    assert_eq!(state.get_metatable(&table_value).unwrap(), None);
    assert_eq!(
        state.set_metatable(&table_value, Some(&first)).unwrap(),
        None,
    );
    assert_eq!(
        state.get_metatable(&table_value).unwrap(),
        Some(first.clone()),
    );

    assert_eq!(
        state.set_metatable(&table_value, Some(&second)).unwrap(),
        Some(first.clone()),
    );

    assert_eq!(
        state.get_metatable(&table_value).unwrap(),
        Some(second.clone()),
    );

    assert_eq!(
        state.set_metatable(&table_value, None).unwrap(),
        Some(second.clone()),
    );

    assert_eq!(state.get_metatable(&table_value).unwrap(), None);

    drop(first);
    drop(second);

    assert_eq!(state.collect_garbage().unwrap(), 2);
}

#[test]
fn index_function_uses_the_target_key_and_first_result() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 1).unwrap();
    let metatable = state.create_table(0, 1).unwrap();

    state
        .raw_set(&target, &string_value("present"), &Value::Boolean(false))
        .unwrap();
    state
        .set_global(b"target", &Value::Table(target.clone()))
        .unwrap();

    let index = returned_function(
        &mut state,
        SourceId::new(20),
        r#"
            return function(actual_target, key)
                seen_target = actual_target
                seen_key = key

                if key == "nothing" then
                    return
                end

                return 41, 99
            end
        "#,
    );

    state
        .raw_set(
            &metatable,
            &string_value("__index"),
            &Value::Function(index),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(21),
        r#"
            return target.present,
                target.answer,
                target.nothing,
                seen_target == target,
                seen_key
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(false),
            Value::Integer(41),
            Value::Nil,
            Value::Boolean(true),
            string_value("nothing"),
        ]
    );
}

#[test]
fn table_metamethods_redirect_reads_and_writes() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 0).unwrap();
    let proxy = state.create_table(0, 2).unwrap();
    let metatable = state.create_table(0, 2).unwrap();

    state
        .raw_set(&proxy, &string_value("answer"), &Value::Integer(42))
        .unwrap();
    state
        .raw_set(&proxy, &string_value("existing"), &Value::Integer(1))
        .unwrap();
    state
        .raw_set(
            &metatable,
            &string_value("__index"),
            &Value::Table(proxy.clone()),
        )
        .unwrap();
    state
        .raw_set(
            &metatable,
            &string_value("__newindex"),
            &Value::Table(proxy.clone()),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();
    state
        .set_global(b"target", &Value::Table(target.clone()))
        .unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(22),
        r#"
            target.existing = 2
            target.missing = 3

            return target.answer, target.missing, target.absent
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![Value::Integer(42), Value::Integer(3), Value::Nil]
    );

    assert_eq!(
        state.raw_get(&target, &string_value("answer")).unwrap(),
        Value::Nil
    );
    assert_eq!(
        state.raw_get(&target, &string_value("missing")).unwrap(),
        Value::Nil
    );
    assert_eq!(
        state.raw_get(&proxy, &string_value("existing")).unwrap(),
        Value::Integer(2)
    );
    assert_eq!(
        state.raw_get(&proxy, &string_value("missing")).unwrap(),
        Value::Integer(3)
    );
}

#[test]
fn newindex_function_intercepts_missing_and_invalid_keys() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 1).unwrap();
    let metatable = state.create_table(0, 1).unwrap();

    state
        .raw_set(&target, &string_value("existing"), &Value::Boolean(false))
        .unwrap();
    state
        .set_global(b"target", &Value::Table(target.clone()))
        .unwrap();

    let new_index = returned_function(
        &mut state,
        SourceId::new(23),
        r#"
            return function(actual_target, key, value)
                if key == nil then
                    saw_nil = actual_target == target and value == 1
                elseif key ~= key then
                    saw_nan = actual_target == target and value == 2
                else
                    seen_target = actual_target
                    seen_key = key
                    seen_value = value
                end

                return 999, 1000
            end
        "#,
    );

    state
        .raw_set(
            &metatable,
            &string_value("__newindex"),
            &Value::Function(new_index),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(24),
        r#"
            target.existing = 2
            target.missing = 42
            target[nil] = 1
            target[0 / 0] = 2

            return target.existing,
                seen_target == target,
                seen_key,
                seen_value,
                saw_nil,
                saw_nan
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(2),
            Value::Boolean(true),
            string_value("missing"),
            Value::Integer(42),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );

    assert_eq!(
        state.raw_get(&target, &string_value("missing")).unwrap(),
        Value::Nil
    );
}

#[test]
fn function_after_a_redirect_receives_the_redirected_table() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 0).unwrap();
    let proxy = state.create_table(0, 0).unwrap();
    let target_metatable = state.create_table(0, 1).unwrap();
    let proxy_metatable = state.create_table(0, 1).unwrap();

    state
        .set_global(b"proxy", &Value::Table(proxy.clone()))
        .unwrap();

    let index = returned_function(
        &mut state,
        SourceId::new(25),
        r#"
            return function(actual_target, key)
                if actual_target == proxy and key == "answer" then
                    return 42
                end

                return 0
            end
        "#,
    );

    state
        .raw_set(
            &target_metatable,
            &string_value("__index"),
            &Value::Table(proxy.clone()),
        )
        .unwrap();
    state
        .raw_set(
            &proxy_metatable,
            &string_value("__index"),
            &Value::Function(index),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(target.clone()), Some(&target_metatable))
        .unwrap();
    state
        .set_metatable(&Value::Table(proxy.clone()), Some(&proxy_metatable))
        .unwrap();
    state.set_global(b"target", &Value::Table(target)).unwrap();

    assert_eq!(
        execute_in_state(&mut state, SourceId::new(26), "return target.answer").unwrap(),
        vec![Value::Integer(42)]
    );
}

#[test]
fn metamethod_lookup_is_raw_and_false_is_not_absent() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 0).unwrap();
    let metatable_metatable = state.create_table(0, 1).unwrap();
    let provider = state.create_table(0, 1).unwrap();
    let fallback = state.create_table(0, 1).unwrap();

    state
        .raw_set(&fallback, &string_value("answer"), &Value::Integer(42))
        .unwrap();
    state
        .raw_set(&provider, &string_value("__index"), &Value::Table(fallback))
        .unwrap();
    state
        .raw_set(
            &metatable_metatable,
            &string_value("__index"),
            &Value::Table(provider),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(metatable.clone()), Some(&metatable_metatable))
        .unwrap();
    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();
    state
        .set_global(b"target", &Value::Table(target.clone()))
        .unwrap();

    assert_eq!(
        execute_in_state(&mut state, SourceId::new(27), "return target.answer").unwrap(),
        vec![Value::Nil]
    );

    state
        .raw_set(&metatable, &string_value("__index"), &Value::Boolean(false))
        .unwrap();

    let error =
        execute_in_state(&mut state, SourceId::new(28), "return target.answer").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::InvalidTableOperand { kind: "boolean" }
    );
}

#[test]
fn metamethod_cycles_report_the_lua_chain_error() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 2).unwrap();

    state
        .raw_set(
            &metatable,
            &string_value("__index"),
            &Value::Table(target.clone()),
        )
        .unwrap();
    state
        .raw_set(
            &metatable,
            &string_value("__newindex"),
            &Value::Table(target.clone()),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();
    state.set_global(b"target", &Value::Table(target)).unwrap();

    let error =
        execute_in_state(&mut state, SourceId::new(29), "return target.missing").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::MetamethodChainTooLong {
            metamethod: "__index"
        }
    );

    let error = execute_in_state(&mut state, SourceId::new(30), "target.missing = 1").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::MetamethodChainTooLong {
            metamethod: "__newindex"
        }
    );
}

#[test]
fn yielding_index_and_newindex_metamethods_resume_correctly() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 2).unwrap();
    let metamethod = state
        .create_native_function("yielding table metamethod", yield_once, &[])
        .unwrap();

    state
        .raw_set(
            &metatable,
            &string_value("__index"),
            &Value::Function(metamethod.clone()),
        )
        .unwrap();
    state
        .raw_set(
            &metatable,
            &string_value("__newindex"),
            &Value::Function(metamethod),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();
    state.set_global(b"target", &Value::Table(target)).unwrap();

    let function = state
        .load_chunk(compile_source(
            SourceId::new(31),
            r#"
                local value = target.answer
                target.other = 1
                return value, 7
            "#,
        ))
        .unwrap();

    let CallOutcome::Yielded { values, suspension } = state.call(&function, &[]).unwrap() else {
        panic!("__index should yield");
    };

    assert_eq!(values, vec![Value::Integer(1)]);

    let CallOutcome::Yielded { values, suspension } =
        suspension.resume(&[Value::Integer(41)]).unwrap()
    else {
        panic!("__newindex should yield");
    };

    assert_eq!(values, vec![Value::Integer(1)]);

    let CallOutcome::Returned(values) = suspension.resume(&[Value::Integer(98)]).unwrap() else {
        panic!("resumed call should return");
    };

    assert_eq!(values, vec![Value::Integer(42), Value::Integer(7)]);
}

#[test]
fn non_table_metatables_are_shared_by_lua_type() {
    let mut state = State::new(NoLoadService).unwrap();
    let nil_metatable = state.create_table(0, 0).unwrap();
    let boolean_metatable = state.create_table(0, 0).unwrap();
    let number_metatable = state.create_table(0, 0).unwrap();
    let string_metatable = state.create_table(0, 0).unwrap();
    let function_metatable = state.create_table(0, 0).unwrap();
    let function = returned_function(&mut state, SourceId::new(32), "return function() end");

    assert_eq!(state.get_metatable(&Value::Nil).unwrap(), None);
    assert_eq!(
        state
            .set_metatable(&Value::Nil, Some(&nil_metatable))
            .unwrap(),
        None
    );
    assert_eq!(
        state.get_metatable(&Value::Nil).unwrap(),
        Some(nil_metatable)
    );

    assert_eq!(
        state
            .set_metatable(&Value::Boolean(true), Some(&boolean_metatable))
            .unwrap(),
        None
    );
    assert_eq!(
        state.get_metatable(&Value::Boolean(false)).unwrap(),
        Some(boolean_metatable)
    );

    assert_eq!(
        state
            .set_metatable(&Value::Integer(1), Some(&number_metatable))
            .unwrap(),
        None
    );
    assert_eq!(
        state.get_metatable(&Value::Float(1.5)).unwrap(),
        Some(number_metatable)
    );

    assert_eq!(
        state
            .set_metatable(&string_value("first"), Some(&string_metatable))
            .unwrap(),
        None
    );
    assert_eq!(
        state.get_metatable(&string_value("second")).unwrap(),
        Some(string_metatable)
    );

    assert_eq!(
        state
            .set_metatable(
                &Value::Function(function.clone()),
                Some(&function_metatable),
            )
            .unwrap(),
        None
    );
    assert_eq!(
        state.get_metatable(&Value::Function(function)).unwrap(),
        Some(function_metatable)
    );
}

#[test]
fn shared_index_metamethods_handle_non_table_values() {
    let mut state = State::new(NoLoadService).unwrap();
    let string_metatable = state.create_table(0, 1).unwrap();
    let string_fallback = state.create_table(0, 1).unwrap();
    let number_metatable = state.create_table(0, 1).unwrap();

    state
        .raw_set(
            &string_fallback,
            &string_value("answer"),
            &Value::Integer(41),
        )
        .unwrap();
    state
        .raw_set(
            &string_metatable,
            &string_value("__index"),
            &Value::Table(string_fallback),
        )
        .unwrap();

    let number_index = returned_function(
        &mut state,
        SourceId::new(33),
        r#"
            return function(target, key)
                seen_number = target
                seen_number_key = key
                return target + 1, 999
            end
        "#,
    );

    state
        .raw_set(
            &number_metatable,
            &string_value("__index"),
            &Value::Function(number_index),
        )
        .unwrap();
    state
        .set_metatable(&string_value("seed"), Some(&string_metatable))
        .unwrap();
    state
        .set_metatable(&Value::Integer(0), Some(&number_metatable))
        .unwrap();
    state.set_global(b"text", &string_value("hello")).unwrap();
    state.set_global(b"number", &Value::Float(41.0)).unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(34),
        r#"
            return text.answer,
                number.successor,
                seen_number,
                seen_number_key
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Integer(41),
            Value::Float(42.0),
            Value::Float(41.0),
            string_value("successor"),
        ]
    );
}

#[test]
fn shared_newindex_metamethods_handle_non_table_values() {
    let mut state = State::new(NoLoadService).unwrap();
    let boolean_metatable = state.create_table(0, 1).unwrap();
    let string_metatable = state.create_table(0, 1).unwrap();
    let string_target = state.create_table(0, 1).unwrap();

    let boolean_newindex = returned_function(
        &mut state,
        SourceId::new(35),
        r#"
            return function(target, key, value)
                seen_boolean = target
                seen_boolean_key = key
                seen_boolean_value = value
                return 999
            end
        "#,
    );

    state
        .raw_set(
            &boolean_metatable,
            &string_value("__newindex"),
            &Value::Function(boolean_newindex),
        )
        .unwrap();
    state
        .raw_set(
            &string_metatable,
            &string_value("__newindex"),
            &Value::Table(string_target.clone()),
        )
        .unwrap();
    state
        .set_metatable(&Value::Boolean(true), Some(&boolean_metatable))
        .unwrap();
    state
        .set_metatable(&string_value("seed"), Some(&string_metatable))
        .unwrap();
    state.set_global(b"flag", &Value::Boolean(false)).unwrap();
    state.set_global(b"text", &string_value("hello")).unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(36),
        r#"
            flag.answer = 41
            text.answer = 42

            return seen_boolean,
                seen_boolean_key,
                seen_boolean_value
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(false),
            string_value("answer"),
            Value::Integer(41),
        ]
    );

    assert_eq!(
        state
            .raw_get(&string_target, &string_value("answer"))
            .unwrap(),
        Value::Integer(42)
    );
}

#[test]
fn non_table_metamethod_cycles_report_the_lua_chain_error() {
    let mut state = State::new(NoLoadService).unwrap();
    let metatable = state.create_table(0, 2).unwrap();

    state
        .raw_set(&metatable, &string_value("__index"), &Value::Integer(0))
        .unwrap();
    state
        .raw_set(&metatable, &string_value("__newindex"), &Value::Float(0.0))
        .unwrap();
    state
        .set_metatable(&Value::Integer(1), Some(&metatable))
        .unwrap();
    state.set_global(b"number", &Value::Integer(1)).unwrap();

    let error =
        execute_in_state(&mut state, SourceId::new(37), "return number.missing").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::MetamethodChainTooLong {
            metamethod: "__index"
        }
    );

    let error = execute_in_state(&mut state, SourceId::new(38), "number.missing = 1").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::MetamethodChainTooLong {
            metamethod: "__newindex"
        }
    );
}

#[test]
fn shared_type_metatables_are_garbage_collection_roots() {
    let mut state = State::new(NoLoadService).unwrap();
    let metatable = state.create_table(0, 1).unwrap();
    let fallback = state.create_table(0, 1).unwrap();

    state
        .raw_set(&fallback, &string_value("answer"), &Value::Integer(42))
        .unwrap();
    state
        .raw_set(
            &metatable,
            &string_value("__index"),
            &Value::Table(fallback.clone()),
        )
        .unwrap();
    state
        .set_metatable(&string_value("seed"), Some(&metatable))
        .unwrap();

    drop(metatable);
    drop(fallback);

    assert_eq!(state.collect_garbage().unwrap(), 0);

    state.set_global(b"text", &string_value("hello")).unwrap();

    assert_eq!(
        execute_in_state(&mut state, SourceId::new(39), "return text.answer",).unwrap(),
        vec![Value::Integer(42)]
    );

    state.collect_garbage().unwrap();
    state.set_metatable(&string_value("seed"), None).unwrap();

    assert_eq!(state.collect_garbage().unwrap(), 2);
}

#[test]
fn call_metamethod_chains_prepend_each_candidate_and_preserve_open_arguments() {
    let mut state = State::new(NoLoadService).unwrap();
    let outer = state.create_table(0, 0).unwrap();
    let middle = state.create_table(0, 0).unwrap();
    let outer_metatable = state.create_table(0, 1).unwrap();
    let middle_metatable = state.create_table(0, 1).unwrap();

    state
        .set_global(b"outer", &Value::Table(outer.clone()))
        .unwrap();
    state
        .set_global(b"middle", &Value::Table(middle.clone()))
        .unwrap();

    let call = returned_function(
        &mut state,
        SourceId::new(40),
        r#"
            return function(actual_middle, actual_outer, first, second)
                return actual_middle == middle,
                    actual_outer == outer,
                    first,
                    second
            end
        "#,
    );

    state
        .raw_set(
            &middle_metatable,
            &string_value("__call"),
            &Value::Function(call),
        )
        .unwrap();

    state
        .raw_set(
            &outer_metatable,
            &string_value("__call"),
            &Value::Table(middle.clone()),
        )
        .unwrap();

    state
        .set_metatable(&Value::Table(middle), Some(&middle_metatable))
        .unwrap();

    state
        .set_metatable(&Value::Table(outer), Some(&outer_metatable))
        .unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(41),
        r#"
            local function arguments()
                return 20, 22
            end

            return outer(arguments())
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Integer(20),
            Value::Integer(22),
        ]
    );
}

#[test]
fn call_metamethod_lookup_is_raw_and_false_is_not_absent() {
    let mut state = State::new(NoLoadService).unwrap();
    let target = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 1).unwrap();
    let metatable_metatable = state.create_table(0, 1).unwrap();
    let inherited = state.create_table(0, 1).unwrap();

    let inherited_call = returned_function(
        &mut state,
        SourceId::new(42),
        "return function() return 42 end",
    );

    state
        .raw_set(
            &inherited,
            &string_value("__call"),
            &Value::Function(inherited_call),
        )
        .unwrap();

    state
        .raw_set(
            &metatable_metatable,
            &string_value("__index"),
            &Value::Table(inherited),
        )
        .unwrap();

    state
        .set_metatable(&Value::Table(metatable.clone()), Some(&metatable_metatable))
        .unwrap();

    state
        .set_metatable(&Value::Table(target.clone()), Some(&metatable))
        .unwrap();

    state.set_global(b"target", &Value::Table(target)).unwrap();

    let error = execute_in_state(&mut state, SourceId::new(43), "return target()").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::InvalidCallOperand { kind: "table" }
    );

    state
        .raw_set(&metatable, &string_value("__call"), &Value::Boolean(false))
        .unwrap();

    let error = execute_in_state(&mut state, SourceId::new(44), "return target()").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::InvalidCallOperand { kind: "boolean" }
    );
}

#[test]
fn call_metamethods_apply_to_shared_type_metatables_but_not_functions() {
    let mut state = State::new(NoLoadService).unwrap();
    let number_metatable = state.create_table(0, 1).unwrap();
    let function_metatable = state.create_table(0, 1).unwrap();

    let number_call = returned_function(
        &mut state,
        SourceId::new(45),
        "return function(self, value) return self + value end",
    );

    let function_call = returned_function(
        &mut state,
        SourceId::new(46),
        "return function() return 0 end",
    );

    let actual_function = returned_function(
        &mut state,
        SourceId::new(47),
        "return function(value) return value + 1 end",
    );

    state
        .raw_set(
            &number_metatable,
            &string_value("__call"),
            &Value::Function(number_call),
        )
        .unwrap();

    state
        .raw_set(
            &function_metatable,
            &string_value("__call"),
            &Value::Function(function_call),
        )
        .unwrap();

    state
        .set_metatable(&Value::Integer(0), Some(&number_metatable))
        .unwrap();

    state
        .set_metatable(
            &Value::Function(actual_function.clone()),
            Some(&function_metatable),
        )
        .unwrap();

    state.set_global(b"number", &Value::Integer(20)).unwrap();

    state
        .set_global(b"actual_function", &Value::Function(actual_function))
        .unwrap();

    assert_eq!(
        execute_in_state(
            &mut state,
            SourceId::new(48),
            "return number(22), actual_function(41)",
        )
        .unwrap(),
        vec![Value::Integer(42), Value::Integer(42)]
    );
}

#[test]
fn callable_values_work_as_generic_for_iterators() {
    let mut state = State::new(NoLoadService).unwrap();
    let iterator = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 1).unwrap();

    state
        .set_global(b"iterator", &Value::Table(iterator.clone()))
        .unwrap();

    let call = returned_function(
        &mut state,
        SourceId::new(49),
        r#"
            return function(self, limit, control)
                if control < limit then
                    return control + 1, self
                end
            end
        "#,
    );

    state
        .raw_set(&metatable, &string_value("__call"), &Value::Function(call))
        .unwrap();

    state
        .set_metatable(&Value::Table(iterator), Some(&metatable))
        .unwrap();

    assert_eq!(
        execute_in_state(
            &mut state,
            SourceId::new(50),
            r#"
                local total = 0
                local saw_self = true

                for value, self in iterator, 3, 0 do
                    total = total + value
                    saw_self = saw_self and self == iterator
                end

                return total, saw_self
            "#,
        )
        .unwrap(),
        vec![Value::Integer(6), Value::Boolean(true)]
    );
}

#[test]
fn yielding_call_metamethod_resumes_into_the_original_call_target() {
    let mut state = State::new(NoLoadService).unwrap();
    let callable = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 1).unwrap();

    let call = state
        .create_native_function("yielding __call", yield_once, &[])
        .unwrap();

    state
        .raw_set(&metatable, &string_value("__call"), &Value::Function(call))
        .unwrap();

    state
        .set_metatable(&Value::Table(callable.clone()), Some(&metatable))
        .unwrap();

    state
        .set_global(b"callable", &Value::Table(callable))
        .unwrap();

    let function = state
        .load_chunk(compile_source(
            SourceId::new(51),
            r#"
                local value = callable(99)
                return value, 7
            "#,
        ))
        .unwrap();

    let CallOutcome::Yielded { values, suspension } = state.call(&function, &[]).unwrap() else {
        panic!("__call should yield");
    };

    assert_eq!(values, vec![Value::Integer(1)]);

    let CallOutcome::Returned(values) = suspension.resume(&[Value::Integer(41)]).unwrap() else {
        panic!("resumed __call should return");
    };

    assert_eq!(values, vec![Value::Integer(42), Value::Integer(7)]);
}

#[test]
fn call_metamethod_cycles_report_the_lua_chain_error() {
    let mut state = State::new(NoLoadService).unwrap();
    let callable = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 1).unwrap();

    state
        .raw_set(
            &metatable,
            &string_value("__call"),
            &Value::Table(callable.clone()),
        )
        .unwrap();

    state
        .set_metatable(&Value::Table(callable.clone()), Some(&metatable))
        .unwrap();

    state
        .set_global(b"callable", &Value::Table(callable))
        .unwrap();

    let error = execute_in_state(&mut state, SourceId::new(52), "return callable()").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::MetamethodChainTooLong {
            metamethod: "__call"
        }
    );
}

#[test]
fn arithmetic_metamethods_use_the_lua_54_names_and_operand_order() {
    let mut state = State::new(NoLoadService).unwrap();
    let left = state.create_table(0, 0).unwrap();
    let right = state.create_table(0, 0).unwrap();

    state
        .set_global(b"left", &Value::Table(left.clone()))
        .unwrap();
    state
        .set_global(b"right", &Value::Table(right.clone()))
        .unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(53),
        r#"
            local function unary(name)
                return function(first, second)
                    if first == left and second == left then
                        return name
                    end

                    return "bad unary operands"
                end
            end

            local function binary(name)
                return function(first, second)
                    if first == left and second == right then
                        return name
                    end

                    return "bad binary operands"
                end
            end

            return {
                __unm = unary("__unm"),
                __bnot = unary("__bnot"),
                __add = binary("__add"),
                __sub = binary("__sub"),
                __mul = binary("__mul"),
                __div = binary("__div"),
                __idiv = binary("__idiv"),
                __mod = binary("__mod"),
                __pow = binary("__pow"),
                __band = binary("__band"),
                __bor = binary("__bor"),
                __bxor = binary("__bxor"),
                __shl = binary("__shl"),
                __shr = binary("__shr"),
            }
        "#,
    )
    .unwrap();

    let [Value::Table(metatable)] = values.as_slice() else {
        panic!("metatable factory should return one table");
    };

    state
        .set_metatable(&Value::Table(left), Some(metatable))
        .unwrap();

    assert_eq!(
        execute_in_state(
            &mut state,
            SourceId::new(54),
            r#"
                return -left,
                    ~left,
                    left + right,
                    left - right,
                    left * right,
                    left / right,
                    left // right,
                    left % right,
                    left ^ right,
                    left & right,
                    left | right,
                    left ~ right,
                    left << right,
                    left >> right
            "#,
        )
        .unwrap(),
        vec![
            string_value("__unm"),
            string_value("__bnot"),
            string_value("__add"),
            string_value("__sub"),
            string_value("__mul"),
            string_value("__div"),
            string_value("__idiv"),
            string_value("__mod"),
            string_value("__pow"),
            string_value("__band"),
            string_value("__bor"),
            string_value("__bxor"),
            string_value("__shl"),
            string_value("__shr"),
        ]
    );
}

#[test]
fn small_integer_left_operands_preserve_metamethod_operand_order() {
    let mut state = State::new(NoLoadService).unwrap();
    let right = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 1).unwrap();

    state
        .set_global(b"right", &Value::Table(right.clone()))
        .unwrap();

    let subtract = returned_function(
        &mut state,
        SourceId::new(153),
        "return function(first, second) return first == 7 and second == right end",
    );

    state
        .raw_set(
            &metatable,
            &string_value("__sub"),
            &Value::Function(subtract),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(right), Some(&metatable))
        .unwrap();

    assert_eq!(
        execute_in_state(&mut state, SourceId::new(154), "return 7 - right").unwrap(),
        vec![Value::Boolean(true)]
    );
}

#[test]
fn binary_arithmetic_uses_the_right_metamethod_only_when_the_left_is_nil() {
    let mut state = State::new(NoLoadService).unwrap();
    let left = state.create_table(0, 0).unwrap();
    let right = state.create_table(0, 0).unwrap();
    let left_metatable = state.create_table(0, 1).unwrap();
    let right_metatable = state.create_table(0, 1).unwrap();

    state
        .set_global(b"left", &Value::Table(left.clone()))
        .unwrap();
    state
        .set_global(b"right", &Value::Table(right.clone()))
        .unwrap();

    let add = returned_function(
        &mut state,
        SourceId::new(55),
        "return function(first, second) return first == left and second == right end",
    );

    state
        .raw_set(
            &right_metatable,
            &string_value("__add"),
            &Value::Function(add),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(left.clone()), Some(&left_metatable))
        .unwrap();
    state
        .set_metatable(&Value::Table(right), Some(&right_metatable))
        .unwrap();

    assert_eq!(
        execute_in_state(&mut state, SourceId::new(56), "return left + right").unwrap(),
        vec![Value::Boolean(true)]
    );

    state
        .raw_set(
            &left_metatable,
            &string_value("__add"),
            &Value::Boolean(false),
        )
        .unwrap();

    let error = execute_in_state(&mut state, SourceId::new(57), "return left + right").unwrap_err();

    assert_eq!(
        error.kind,
        VmErrorKind::InvalidCallOperand { kind: "boolean" }
    );
}

#[test]
fn primitive_arithmetic_wins_and_primitive_errors_do_not_fall_back() {
    let mut state = State::new(NoLoadService).unwrap();
    let metatable = state.create_table(0, 4).unwrap();

    let fallback = returned_function(
        &mut state,
        SourceId::new(58),
        "return function() return 999 end",
    );
    let non_integral_bnot = returned_function(
        &mut state,
        SourceId::new(59),
        "return function(first, second) return first == 3.5 and second == 3.5 and 88 end",
    );

    for name in ["__add", "__idiv", "__band"] {
        state
            .raw_set(
                &metatable,
                &string_value(name),
                &Value::Function(fallback.clone()),
            )
            .unwrap();
    }

    state
        .raw_set(
            &metatable,
            &string_value("__bnot"),
            &Value::Function(non_integral_bnot),
        )
        .unwrap();
    state
        .set_metatable(&Value::Integer(0), Some(&metatable))
        .unwrap();

    assert_eq!(
        execute_in_state(
            &mut state,
            SourceId::new(60),
            "return 20 + 22, 7 // 2, 3 & 1, ~3.5",
        )
        .unwrap(),
        vec![
            Value::Integer(42),
            Value::Integer(3),
            Value::Integer(1),
            Value::Integer(88),
        ]
    );

    let error = execute_in_state(&mut state, SourceId::new(61), "return 1 // 0").unwrap_err();

    assert_eq!(error.kind, VmErrorKind::IntegerDivisionByZero);
}

#[test]
fn arithmetic_metamethods_use_only_the_first_result_and_nil_fill_no_results() {
    let mut state = State::new(NoLoadService).unwrap();
    let left = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 2).unwrap();

    let add = returned_function(
        &mut state,
        SourceId::new(62),
        "return function() return end",
    );
    let subtract = returned_function(
        &mut state,
        SourceId::new(63),
        "return function() return 41, 99 end",
    );

    state
        .raw_set(&metatable, &string_value("__add"), &Value::Function(add))
        .unwrap();
    state
        .raw_set(
            &metatable,
            &string_value("__sub"),
            &Value::Function(subtract),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(left.clone()), Some(&metatable))
        .unwrap();
    state.set_global(b"left", &Value::Table(left)).unwrap();

    assert_eq!(
        execute_in_state(&mut state, SourceId::new(64), "return left + 1, left - 1").unwrap(),
        vec![Value::Nil, Value::Integer(41)]
    );
}

#[test]
fn arithmetic_metamethod_values_can_use_call_metamethods() {
    let mut state = State::new(NoLoadService).unwrap();
    let left = state.create_table(0, 0).unwrap();
    let right = state.create_table(0, 0).unwrap();
    let operator = state.create_table(0, 0).unwrap();
    let left_metatable = state.create_table(0, 1).unwrap();
    let operator_metatable = state.create_table(0, 1).unwrap();

    state
        .set_global(b"left", &Value::Table(left.clone()))
        .unwrap();
    state
        .set_global(b"right", &Value::Table(right.clone()))
        .unwrap();
    state
        .set_global(b"operator", &Value::Table(operator.clone()))
        .unwrap();

    let call = returned_function(
        &mut state,
        SourceId::new(65),
        r#"
            return function(actual_operator, actual_left, actual_right)
                if actual_operator == operator
                    and actual_left == left
                    and actual_right == right
                then
                    return 42
                end

                return 0
            end
        "#,
    );

    state
        .raw_set(
            &operator_metatable,
            &string_value("__call"),
            &Value::Function(call),
        )
        .unwrap();
    state
        .raw_set(
            &left_metatable,
            &string_value("__mul"),
            &Value::Table(operator.clone()),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(operator), Some(&operator_metatable))
        .unwrap();
    state
        .set_metatable(&Value::Table(left), Some(&left_metatable))
        .unwrap();

    assert_eq!(
        execute_in_state(&mut state, SourceId::new(66), "return left * right").unwrap(),
        vec![Value::Integer(42)]
    );
}

#[test]
fn yielding_arithmetic_metamethods_resume_into_the_destination_register() {
    let mut state = State::new(NoLoadService).unwrap();
    let left = state.create_table(0, 0).unwrap();
    let right = state.create_table(0, 0).unwrap();
    let metatable = state.create_table(0, 1).unwrap();
    let add = state
        .create_native_function("yielding __add", yield_once, &[])
        .unwrap();

    state
        .raw_set(&metatable, &string_value("__add"), &Value::Function(add))
        .unwrap();
    state
        .set_metatable(&Value::Table(left.clone()), Some(&metatable))
        .unwrap();
    state.set_global(b"left", &Value::Table(left)).unwrap();
    state.set_global(b"right", &Value::Table(right)).unwrap();

    let function = state
        .load_chunk(compile_source(
            SourceId::new(67),
            r#"
                local value = left + right
                return value, 7
            "#,
        ))
        .unwrap();

    let CallOutcome::Yielded { values, suspension } = state.call(&function, &[]).unwrap() else {
        panic!("__add should yield");
    };

    assert_eq!(values, vec![Value::Integer(1)]);

    let CallOutcome::Returned(values) = suspension.resume(&[Value::Integer(41)]).unwrap() else {
        panic!("resumed __add should return");
    };

    assert_eq!(values, vec![Value::Integer(42), Value::Integer(7)]);
}

#[test]
fn missing_arithmetic_metamethods_preserve_the_existing_operand_errors() {
    let mut state = State::new(NoLoadService).unwrap();

    let add_error = execute_in_state(&mut state, SourceId::new(68), "return {} + {}").unwrap_err();
    assert_eq!(
        add_error.kind,
        VmErrorKind::InvalidAddOperands {
            left: "table",
            right: "table",
        }
    );

    let negate_error = execute_in_state(&mut state, SourceId::new(69), "return -{}").unwrap_err();
    assert_eq!(
        negate_error.kind,
        VmErrorKind::InvalidNegateOperand { kind: "table" }
    );

    let bitwise_error =
        execute_in_state(&mut state, SourceId::new(70), "return 1 & 1.5").unwrap_err();
    assert!(matches!(
        bitwise_error.kind,
        VmErrorKind::InvalidBitwiseOperands { .. }
    ));
}

#[test]
fn comparison_metamethods_follow_lua_semantics() {
    let mut state = State::new(NoLoadService).unwrap();
    let a = state.create_table(0, 0).unwrap();
    let b = state.create_table(0, 0).unwrap();

    state.set_global(b"a", &Value::Table(a.clone())).unwrap();
    state.set_global(b"b", &Value::Table(b.clone())).unwrap();

    let values = execute_in_state(
        &mut state,
        SourceId::new(71),
        r#"
            calls = {}

            return {
                __lt = function(left, right)
                    calls[#calls + 1] = {
                        name = "__lt",
                        left = left,
                        right = right,
                    }

                    return "truthy"
                end,

                __le = function(left, right)
                    calls[#calls + 1] = {
                        name = "__le",
                        left = left,
                        right = right,
                    }

                    return 123
                end,

                __eq = function(left, right)
                    calls[#calls + 1] = {
                        name = "__eq",
                        left = left,
                        right = right,
                    }

                    return {}
                end,
            }
        "#,
    )
    .unwrap();

    let [Value::Table(metatable)] = values.as_slice() else {
        panic!("metatable factory should return one table");
    };

    let metatable = metatable.clone();

    state
        .set_metatable(&Value::Table(a), Some(&metatable))
        .unwrap();

    state
        .set_metatable(&Value::Table(b), Some(&metatable))
        .unwrap();

    let actual = execute_in_state(
        &mut state,
        SourceId::new(72),
        r#"
            local lt = a < b
            local gt = b > a
            local le = a <= b
            local ge = b >= a
            local eq = a == b
            local ne = a ~= b
            local same = a == a

            return
                lt, gt, le, ge, eq, ne, same,
                #calls,
                calls[1].left == a and calls[1].right == b,
                calls[2].left == a and calls[2].right == b,
                calls[3].left == a and calls[3].right == b,
                calls[4].left == a and calls[4].right == b
        "#,
    )
    .unwrap();

    assert_eq!(
        actual,
        vec![
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Boolean(true),
            Value::Integer(6),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::Boolean(true),
        ]
    );
}
