use orbit_common::SourceId;
use orbit_compiler::bytecode::Chunk;
use orbit_parser::{lexer::lex, parser::parse_chunk};

use crate::{
    error::{VmErrorKind, VmResult, VmTraceFrame},
    loading::NoLoadService,
    runtime::Runtime,
    string::LuaString,
    value::RawValue,
};

use super::{Execution, ExecutionOutcome, FrameBoundary};

fn compile_source(source: &str) -> Chunk {
    let source_id = SourceId::new(0);
    let tokens = lex(source_id, source).unwrap();

    let ast = parse_chunk(source_id, tokens).unwrap();

    let hir = orbit_resolver::resolve(&ast).unwrap();

    orbit_compiler::compile(hir).unwrap()
}

fn run_source(source: &str) -> VmResult<Box<[RawValue]>> {
    let chunk = compile_source(source);
    let mut runtime = Runtime::new(Box::new(NoLoadService)).map_err(crate::VmError::from)?;

    let function = runtime
        .load_chunk_raw(chunk)
        .map_err(crate::VmError::from)?;

    let function = runtime
        .function_snapshot(function)
        .map_err(crate::VmError::from)?;

    let outcome = Execution::new(&mut runtime, function, Box::new([]))
        .map_err(crate::VmError::from)?
        .run()?;

    match outcome {
        super::ExecutionOutcome::Returned { values, .. } => Ok(values),
        super::ExecutionOutcome::Yielded { .. } => {
            panic!("ordinary Lua execution unexpectedly yielded")
        }
    }
}

fn assert_run(source: &str, expected: &[RawValue]) {
    let actual = run_source(source).unwrap();

    assert_eq!(actual.as_ref(), expected, "source:\n{source}",);
}

#[test]
fn executes_data_and_arithmetic_instructions() {
    assert_run(
        r#"
            local value = 20
            local copy = value

            return nil, true, false, copy + 22,
                -7 // 3, -7 % 3, 3.0 & 1
        "#,
        &[
            RawValue::Nil,
            RawValue::Boolean(true),
            RawValue::Boolean(false),
            RawValue::Integer(42),
            RawValue::Integer(-3),
            RawValue::Integer(2),
            RawValue::Integer(1),
        ],
    );
}

#[test]
fn executes_control_flow_and_numeric_for() {
    assert_run(
        r#"
            local total = 0

            for value = 1, 5 do
                if value == 4 then
                    break
                end

                total = total + value
            end

            local index = 0

            while index < 3 do
                index = index + 1
                total = total + index
            end

            return total
        "#,
        &[RawValue::Integer(12)],
    );
}

#[test]
fn executes_table_instructions() {
    assert_run(
        r#"
            local values = {
                10,
                20,
                name = "orbit",
            }

            values[1.0] = 11
            values[3] = values[1] + 30
            local length = #values
            values[2] = nil

            return values[1], values[2],
                values[3], values.name, length
        "#,
        &[
            RawValue::Integer(11),
            RawValue::Nil,
            RawValue::Integer(41),
            RawValue::String(LuaString::from("orbit")),
            RawValue::Integer(3),
        ],
    );
}

#[test]
fn executes_lua_calls_and_fixed_results() {
    assert_run(
        r#"
            local function add(left, right)
                return left + right
            end

            local function one()
                return 7
            end

            local a, b, c = one()

            return add(20, 22), a, b, c
        "#,
        &[
            RawValue::Integer(42),
            RawValue::Integer(7),
            RawValue::Nil,
            RawValue::Nil,
        ],
    );
}

#[test]
fn executes_varargs_and_open_results() {
    assert_run(
        r#"
            local function identity(...)
                return ...
            end

            local function collect(...)
                return identity(...)
            end

            return collect(1, 2, 3, 4)
        "#,
        &[
            RawValue::Integer(1),
            RawValue::Integer(2),
            RawValue::Integer(3),
            RawValue::Integer(4),
        ],
    );
}

#[test]
fn lua_calls_transfer_open_arguments_and_results_between_frames() {
    assert_run(
        r#"
            local function produce()
                return 10, nil, 30
            end

            local function consume(prefix, ...)
                local first, second, third = ...
                return prefix + first, second, third
            end

            local function invoke()
                local first, second, third = consume(2, produce())
                return first, second, third
            end

            return invoke()
        "#,
        &[RawValue::Integer(12), RawValue::Nil, RawValue::Integer(30)],
    );
}

#[test]
fn tail_calls_forward_all_results_to_the_original_lua_target() {
    assert_run(
        r#"
            local function values()
                return 1, nil, 3
            end

            local function relay()
                return values()
            end

            local first, second, third = relay()
            return first, second, third
        "#,
        &[RawValue::Integer(1), RawValue::Nil, RawValue::Integer(3)],
    );
}

#[test]
fn tail_calls_preserve_captured_arguments_after_replacing_their_frame() {
    assert_run(
        r#"
            local function invoke(callback)
                return callback()
            end

            local function wrapper()
                local value = 42
                return invoke(function()
                    return value
                end)
            end

            return wrapper()
        "#,
        &[RawValue::Integer(42)],
    );
}

#[test]
fn method_tail_calls_reuse_the_activation_stack() {
    assert_run(
        r#"
            local object = {}

            function object:descend(depth)
                if depth == 0 then
                    return 42
                end

                return self:descend(depth - 1)
            end

            return object:descend(30000)
        "#,
        &[RawValue::Integer(42)],
    );
}

#[test]
fn tail_invoke_replaces_the_active_activation() {
    let chunk = compile_source("return (function(value) return value end)(42)");
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function = runtime.load_chunk_raw(chunk).unwrap();
    let function = runtime.function_snapshot(function).unwrap();
    let mut execution = Execution::new(&mut runtime, function, Box::new([])).unwrap();

    assert_eq!(execution.stack.len(), 1);

    let FrameBoundary::TailInvoke { callee, arguments } = execution.run_until_boundary().unwrap()
    else {
        panic!("expected tail invocation");
    };

    execution.replace_callable(callee, arguments).unwrap();

    assert_eq!(execution.stack.len(), 1);

    let ExecutionOutcome::Returned { values, .. } = execution.run().unwrap() else {
        panic!("tail-called Lua function unexpectedly yielded");
    };

    assert_eq!(values.as_ref(), &[RawValue::Integer(42)]);
}

#[test]
fn failed_dispatch_keeps_gc_pc_on_the_failing_instruction() {
    let chunk = compile_source("return {} + nil");
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();
    let function = runtime.load_chunk_raw(chunk).unwrap();
    let function = runtime.function_snapshot(function).unwrap();
    let mut execution = Execution::new(&mut runtime, function, Box::new([])).unwrap();

    let error = match execution.run_until_boundary() {
        Err(error) => error,
        Ok(_) => panic!("invalid addition unexpectedly reached a frame boundary"),
    };

    assert!(matches!(error, VmErrorKind::InvalidAddOperands { .. }));

    let frame = execution.active_lua_frame();
    let VmTraceFrame::Lua { pc, .. } = frame.trace_frame() else {
        panic!("failing instruction should produce a Lua trace frame");
    };

    assert_eq!(frame.gc_pc_for_test(), pc);
}

#[test]
fn closures_share_and_close_upvalues() {
    assert_run(
        r#"
            local value = 0

            local function increment()
                value = value + 1
                return value
            end

            local functions = {}

            for index = 1, 3 do
                functions[index] = function()
                    return index
                end
            end

            return increment(), increment(),
                functions[1](), functions[2](),
                functions[3]()
        "#,
        &[
            RawValue::Integer(1),
            RawValue::Integer(2),
            RawValue::Integer(1),
            RawValue::Integer(2),
            RawValue::Integer(3),
        ],
    );
}

#[test]
fn globals_use_the_runtime_global_table() {
    assert_run(
        r#"
            answer = 41

            local function read()
                return answer + 1
            end

            return read(), missing
        "#,
        &[RawValue::Integer(42), RawValue::Nil],
    );
}

#[test]
fn deeply_nested_calls_do_not_recurse_in_rust() {
    assert_run(
        r#"
            local function descend(depth)
                if depth == 0 then
                    return 42
                end

                return descend(depth - 1)
            end

            return descend(5000)
        "#,
        &[RawValue::Integer(42)],
    );
}
