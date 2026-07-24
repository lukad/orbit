use orbit_common::SourceId;
use orbit_compiler::bytecode::Chunk;
use orbit_parser::{lexer::lex, parser::parse_chunk};

use crate::{
    error::{VmError, VmErrorKind, VmResult, VmTraceFrame},
    loading::NoLoadService,
    native::{NativeAction, NativeContext, NativeEvent, NativeToken},
    runtime::Runtime,
    value::RawValue,
};

use super::{Execution, ExecutionOutcome};

const FIRST_CALL: NativeToken = NativeToken::new(1);

const SECOND_CALL: NativeToken = NativeToken::new(2);

const YIELD_TOKEN: NativeToken = NativeToken::new(3);

const GET_ACTION: NativeToken = NativeToken::new(4);

const SET_ACTION: NativeToken = NativeToken::new(5);

fn compile_source(source: &str) -> Chunk {
    let source_id = SourceId::new(0);
    let tokens = lex(source_id, source).unwrap();
    let ast = parse_chunk(source_id, &tokens).unwrap();
    let hir = orbit_resolver::resolve(&ast).unwrap();

    orbit_compiler::compile(hir).unwrap()
}

fn execution<'runtime>(runtime: &'runtime mut Runtime, source: &str) -> Execution<'runtime> {
    let function = runtime.load_chunk_raw(compile_source(source)).unwrap();

    let function = runtime.function_snapshot(function).unwrap();

    Execution::new(runtime, function, Box::new([])).unwrap()
}

fn values(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    Ok(context.return_values([context.integer(20), context.integer(22)]))
}

fn twice(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let callee = context
                .argument(0)
                .ok_or_else(|| native_failure("missing callable"))?;

            let value = context
                .argument(1)
                .ok_or_else(|| native_failure("missing value"))?;

            Ok(context.call(callee, [value], FIRST_CALL))
        }

        NativeEvent::Resume { token: FIRST_CALL } => {
            let callee = context
                .argument(0)
                .ok_or_else(|| native_failure("missing callable"))?;

            let value = context
                .resume_value(0)
                .ok_or_else(|| native_failure("missing first result"))?;

            Ok(context.call(callee, [value], SECOND_CALL))
        }

        NativeEvent::Resume { token: SECOND_CALL } => {
            let value = context
                .resume_value(0)
                .ok_or_else(|| native_failure("missing second result"))?;

            Ok(context.return_values([value]))
        }

        NativeEvent::ResumeError { .. } => Err(context
            .resume_error()
            .expect("error event carries an error")
            .clone()),

        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
}

fn recover(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let callee = context
                .argument(0)
                .ok_or_else(|| native_failure("missing callable"))?;

            Ok(context.call(callee, [], FIRST_CALL))
        }

        NativeEvent::ResumeError { token: FIRST_CALL } => {
            assert!(context.resume_error().is_some());

            Ok(context.return_values([context.string("caught")]))
        }

        NativeEvent::Resume { token: FIRST_CALL } => {
            Ok(context.return_values([context.string("unexpected success")]))
        }

        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
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

fn explode(_: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    Err(native_failure("boom"))
}

fn replace_metatable(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| native_failure("missing value"))?;

    let requested = context
        .argument(1)
        .ok_or_else(|| native_failure("missing metatable"))?;

    if requested.is_nil() {
        context.set_metatable(&value, None)?;
    } else {
        context.set_metatable(&value, Some(&requested))?;
    }

    let actual = context
        .get_metatable(&value)?
        .unwrap_or_else(|| context.nil());

    Ok(context.return_values([actual]))
}

fn native_get(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let target = context
                .argument(0)
                .ok_or_else(|| native_failure("missing get target"))?;
            let key = context
                .argument(1)
                .ok_or_else(|| native_failure("missing get key"))?;

            Ok(context.get_with_continuation(target, key.clone(), [key], GET_ACTION))
        }
        NativeEvent::Resume { token: GET_ACTION } => {
            assert_eq!(context.resume_value_count(), 1);
            assert_eq!(context.continuation_value_count(), 1);

            let value = context
                .resume_value(0)
                .expect("get action always resumes with one value");

            Ok(context.return_values([value]))
        }
        NativeEvent::ResumeError { token: GET_ACTION } => Err(context
            .resume_error()
            .expect("get error event carries an error")
            .clone()),
        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
}

fn native_set(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let target = context
                .argument(0)
                .ok_or_else(|| native_failure("missing set target"))?;
            let key = context
                .argument(1)
                .ok_or_else(|| native_failure("missing set key"))?;
            let value = context
                .argument(2)
                .ok_or_else(|| native_failure("missing set value"))?;

            Ok(context.set_with_continuation(target, key, value.clone(), [value], SET_ACTION))
        }
        NativeEvent::Resume { token: SET_ACTION } => {
            assert_eq!(context.resume_value_count(), 0);
            assert_eq!(context.continuation_value_count(), 1);

            let value = context
                .continuation_value(0)
                .expect("set continuation preserves the assigned value");

            Ok(context.return_values([value]))
        }
        NativeEvent::ResumeError { token: SET_ACTION } => Err(context
            .resume_error()
            .expect("set error event carries an error")
            .clone()),
        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
}

fn recover_get_error(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let target = context
                .argument(0)
                .ok_or_else(|| native_failure("missing get target"))?;
            let key = context
                .argument(1)
                .ok_or_else(|| native_failure("missing get key"))?;
            let marker = context.string("preserved");

            Ok(context.get_with_continuation(target, key, [marker], GET_ACTION))
        }
        NativeEvent::ResumeError { token: GET_ACTION } => {
            assert_eq!(context.continuation_value_count(), 1);
            assert!(context.resume_error().is_some());

            Ok(context.return_values([context.string("caught")]))
        }
        NativeEvent::Resume { token: GET_ACTION } => {
            Ok(context.return_values([context.string("unexpected success")]))
        }
        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
}

fn get_with_heap_continuation(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let target = context
                .argument(0)
                .ok_or_else(|| native_failure("missing get target"))?;
            let key = context
                .argument(1)
                .ok_or_else(|| native_failure("missing get key"))?;
            let marker = context.create_table(0, 1)?;
            let marker_key = context.string("answer");
            let marker_value = context.integer(42);

            context.raw_set(&marker, marker_key, marker_value)?;

            Ok(context.get_with_continuation(target, key, [marker], GET_ACTION))
        }
        NativeEvent::Resume { token: GET_ACTION } => {
            let value = context
                .resume_value(0)
                .expect("get action always resumes with one value");
            let marker = context
                .continuation_value(0)
                .expect("get continuation preserves the marker table");

            Ok(context.return_values([value, marker]))
        }
        NativeEvent::ResumeError { token: GET_ACTION } => Err(context
            .resume_error()
            .expect("get error event carries an error")
            .clone()),
        event => Err(native_failure(format!("unexpected event: {event:?}"))),
    }
}

fn native_failure(message: impl Into<Box<str>>) -> VmError {
    VmErrorKind::NativeFunctionFailure {
        message: message.into(),
    }
    .into()
}

#[test]
fn lua_receives_native_results() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function = runtime
        .allocate_native_function("values", values, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"values", RawValue::Function(function))
        .unwrap();

    let outcome = execution(&mut runtime, "return values()").run().unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("execution unexpectedly yielded");
    };

    assert_eq!(
        values.as_ref(),
        &[RawValue::Integer(20), RawValue::Integer(22),]
    );
}

#[test]
fn native_callback_calls_and_resumes_lua() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function = runtime
        .allocate_native_function("twice", twice, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"twice", RawValue::Function(function))
        .unwrap();

    let outcome = execution(
        &mut runtime,
        r#"
            local function increment(value)
                return value + 1
            end

            return twice(increment, 40)
        "#,
    )
    .run()
    .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("execution unexpectedly yielded");
    };

    assert_eq!(values.as_ref(), &[RawValue::Integer(42)]);
}

#[test]
fn native_callback_can_recover_from_lua_error() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function = runtime
        .allocate_native_function("recover", recover, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"recover", RawValue::Function(function))
        .unwrap();

    let outcome = execution(
        &mut runtime,
        r#"
            local function fail()
                return nil + 1
            end

            return recover(fail)
        "#,
    )
    .run()
    .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("execution unexpectedly yielded");
    };

    assert_eq!(
        values.as_ref(),
        &[RawValue::String(crate::string::LuaString::from("caught",),)]
    );
}

#[test]
fn yielded_native_callback_resumes() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function = runtime
        .allocate_native_function("yield_once", yield_once, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"yield_once", RawValue::Function(function))
        .unwrap();

    let outcome = execution(&mut runtime, "return yield_once()")
        .run()
        .unwrap();

    let ExecutionOutcome::Yielded { values, suspension } = outcome else {
        panic!("execution should yield");
    };

    assert_eq!(values.as_ref(), &[RawValue::Integer(1)]);

    let outcome = suspension
        .resume(vec![RawValue::Integer(41)].into_boxed_slice())
        .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("resumed execution unexpectedly yielded again");
    };

    assert_eq!(values.as_ref(), &[RawValue::Integer(42)]);
}

#[test]
fn native_error_has_native_and_lua_frames() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function = runtime
        .allocate_native_function("explode", explode, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"explode", RawValue::Function(function))
        .unwrap();

    let error = match execution(&mut runtime, "return explode()").run() {
        Err(error) => error,
        Ok(_) => panic!("exploding native function unexpectedly succeeded"),
    };

    assert!(matches!(
        error.frames.first(),
        Some(VmTraceFrame::Native { name })
            if name.as_ref() == "explode"
    ));

    assert!(matches!(
        error.frames.get(1),
        Some(VmTraceFrame::Lua { .. })
    ));
}

#[test]
fn native_context_gets_sets_and_clears_metatables() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function = runtime
        .allocate_native_function("replace_metatable", replace_metatable, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"replace_metatable", RawValue::Function(function))
        .unwrap();

    let outcome = execution(
        &mut runtime,
        r#"
            local table_value = {}
            local table_metatable = { __index = { answer = 41 } }
            local number_metatable = { __index = { answer = 42 } }

            local installed_table = replace_metatable(table_value, table_metatable)
            local table_answer = table_value.answer
            local installed_number = replace_metatable(0, number_metatable)
            local number_value = 1
            local number_answer = number_value.answer
            local cleared_table = replace_metatable(table_value, nil)
            local missing_table_answer = table_value.answer
            local cleared_number = replace_metatable(0, nil)

            return installed_table == table_metatable,
                table_answer,
                installed_number == number_metatable,
                number_answer,
                cleared_table,
                missing_table_answer,
                cleared_number
        "#,
    )
    .run()
    .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("execution unexpectedly yielded");
    };

    assert_eq!(
        values.as_ref(),
        &[
            RawValue::Boolean(true),
            RawValue::Integer(41),
            RawValue::Boolean(true),
            RawValue::Integer(42),
            RawValue::Nil,
            RawValue::Nil,
            RawValue::Nil,
        ]
    );
}

#[test]
fn native_get_and_set_follow_direct_and_redirected_table_access() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let get = runtime
        .allocate_native_function("native_get", native_get, Box::new([]))
        .unwrap();
    let set = runtime
        .allocate_native_function("native_set", native_set, Box::new([]))
        .unwrap();
    let replace = runtime
        .allocate_native_function("replace_metatable", replace_metatable, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"native_get", RawValue::Function(get))
        .unwrap();
    runtime
        .set_global(b"native_set", RawValue::Function(set))
        .unwrap();
    runtime
        .set_global(b"replace_metatable", RawValue::Function(replace))
        .unwrap();

    let outcome = execution(
        &mut runtime,
        r#"
            local direct = { answer = 40 }
            local backing = { answer = 41 }
            local proxy = {}

            replace_metatable(proxy, {
                __index = backing,
                __newindex = backing,
            })

            local direct_answer = native_get(direct, "answer")
            local missing = native_get(direct, "missing")
            local redirected_answer = native_get(proxy, "answer")
            local direct_result = native_set(direct, "other", 42)
            local redirected_result = native_set(proxy, "other", 43)

            return direct_answer,
                missing,
                redirected_answer,
                direct_result,
                direct.other,
                redirected_result,
                backing.other
        "#,
    )
    .run()
    .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("execution unexpectedly yielded");
    };

    assert_eq!(
        values.as_ref(),
        &[
            RawValue::Integer(40),
            RawValue::Nil,
            RawValue::Integer(41),
            RawValue::Integer(42),
            RawValue::Integer(42),
            RawValue::Integer(43),
            RawValue::Integer(43),
        ]
    );
}

#[test]
fn native_get_and_set_normalize_function_metamethod_results() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let get = runtime
        .allocate_native_function("native_get", native_get, Box::new([]))
        .unwrap();
    let set = runtime
        .allocate_native_function("native_set", native_set, Box::new([]))
        .unwrap();
    let replace = runtime
        .allocate_native_function("replace_metatable", replace_metatable, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"native_get", RawValue::Function(get))
        .unwrap();
    runtime
        .set_global(b"native_set", RawValue::Function(set))
        .unwrap();
    runtime
        .set_global(b"replace_metatable", RawValue::Function(replace))
        .unwrap();

    let outcome = execution(
        &mut runtime,
        r#"
            local many = {}
            local none = {}
            local intercepted = {}

            replace_metatable(many, {
                __index = function()
                    return 41, 99
                end,
            })

            replace_metatable(none, {
                __index = function()
                    return
                end,
            })

            replace_metatable(intercepted, {
                __newindex = function(target, key, value)
                    seen_target = target
                    seen_key = key
                    seen_value = value
                    return 98, 99
                end,
            })

            local set_result = native_set(intercepted, "answer", 42)

            return native_get(many, "answer"),
                native_get(none, "answer"),
                set_result,
                seen_target == intercepted,
                seen_key,
                seen_value,
                intercepted.answer
        "#,
    )
    .run()
    .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("execution unexpectedly yielded");
    };

    assert_eq!(
        values.as_ref(),
        &[
            RawValue::Integer(41),
            RawValue::Nil,
            RawValue::Integer(42),
            RawValue::Boolean(true),
            RawValue::String(crate::string::LuaString::from("answer")),
            RawValue::Integer(42),
            RawValue::Nil,
        ]
    );
}

#[test]
fn native_get_and_set_wait_for_yielding_metamethods() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let get = runtime
        .allocate_native_function("native_get", native_get, Box::new([]))
        .unwrap();
    let set = runtime
        .allocate_native_function("native_set", native_set, Box::new([]))
        .unwrap();
    let replace = runtime
        .allocate_native_function("replace_metatable", replace_metatable, Box::new([]))
        .unwrap();
    let metamethod = runtime
        .allocate_native_function("yielding metamethod", yield_once, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"native_get", RawValue::Function(get))
        .unwrap();
    runtime
        .set_global(b"native_set", RawValue::Function(set))
        .unwrap();
    runtime
        .set_global(b"replace_metatable", RawValue::Function(replace))
        .unwrap();
    runtime
        .set_global(b"metamethod", RawValue::Function(metamethod))
        .unwrap();

    let outcome = execution(
        &mut runtime,
        r#"
            local target = {}
            replace_metatable(target, {
                __index = metamethod,
                __newindex = metamethod,
            })

            local read = native_get(target, "answer")
            local written = native_set(target, "other", 7)
            return read, written
        "#,
    )
    .run()
    .unwrap();

    let ExecutionOutcome::Yielded { values, suspension } = outcome else {
        panic!("get metamethod should yield");
    };
    assert_eq!(values.as_ref(), &[RawValue::Integer(1)]);

    let outcome = suspension
        .resume(vec![RawValue::Integer(41)].into_boxed_slice())
        .unwrap();

    let ExecutionOutcome::Yielded { values, suspension } = outcome else {
        panic!("set metamethod should yield");
    };
    assert_eq!(values.as_ref(), &[RawValue::Integer(1)]);

    let outcome = suspension
        .resume(vec![RawValue::Integer(98)].into_boxed_slice())
        .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("resumed execution unexpectedly yielded again");
    };
    assert_eq!(
        values.as_ref(),
        &[RawValue::Integer(42), RawValue::Integer(7)]
    );
}

#[test]
fn native_get_delivers_access_errors_to_the_callback() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let recover = runtime
        .allocate_native_function("recover_get_error", recover_get_error, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"recover_get_error", RawValue::Function(recover))
        .unwrap();

    let outcome = execution(&mut runtime, r#"return recover_get_error(false, "answer")"#)
        .run()
        .unwrap();

    let ExecutionOutcome::Returned { values, .. } = outcome else {
        panic!("execution unexpectedly yielded");
    };

    assert_eq!(
        values.as_ref(),
        &[RawValue::String(crate::string::LuaString::from("caught"))]
    );
}

#[test]
fn native_get_uses_the_shared_metamethod_chain_limit() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let get = runtime
        .allocate_native_function("native_get", native_get, Box::new([]))
        .unwrap();
    let replace = runtime
        .allocate_native_function("replace_metatable", replace_metatable, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"native_get", RawValue::Function(get))
        .unwrap();
    runtime
        .set_global(b"replace_metatable", RawValue::Function(replace))
        .unwrap();

    let error = match execution(
        &mut runtime,
        r#"
            local target = {}
            replace_metatable(target, { __index = target })
            return native_get(target, "missing")
        "#,
    )
    .run()
    {
        Err(error) => error,
        Ok(_) => panic!("cyclic native get unexpectedly succeeded"),
    };

    assert_eq!(
        error.kind,
        VmErrorKind::MetamethodChainTooLong {
            metamethod: "__index",
        }
    );
}

#[test]
fn native_get_continuation_values_are_roots_while_a_metamethod_yields() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let get = runtime
        .allocate_native_function(
            "get_with_heap_continuation",
            get_with_heap_continuation,
            Box::new([]),
        )
        .unwrap();
    let replace = runtime
        .allocate_native_function("replace_metatable", replace_metatable, Box::new([]))
        .unwrap();
    let metamethod = runtime
        .allocate_native_function("yielding metamethod", yield_once, Box::new([]))
        .unwrap();

    runtime
        .set_global(b"get_with_heap_continuation", RawValue::Function(get))
        .unwrap();
    runtime
        .set_global(b"replace_metatable", RawValue::Function(replace))
        .unwrap();
    runtime
        .set_global(b"metamethod", RawValue::Function(metamethod))
        .unwrap();

    let outcome = execution(
        &mut runtime,
        r#"
            local target = {}
            replace_metatable(target, { __index = metamethod })
            return get_with_heap_continuation(target, "answer")
        "#,
    )
    .run()
    .unwrap();

    let ExecutionOutcome::Yielded {
        values,
        mut suspension,
    } = outcome
    else {
        panic!("get metamethod should yield");
    };
    assert_eq!(values.as_ref(), &[RawValue::Integer(1)]);

    suspension.collect_garbage().unwrap();

    let outcome = suspension
        .resume(vec![RawValue::Integer(41)].into_boxed_slice())
        .unwrap();

    let ExecutionOutcome::Returned { values, runtime } = outcome else {
        panic!("resumed execution unexpectedly yielded again");
    };

    assert_eq!(values.first(), Some(&RawValue::Integer(42)));

    let marker = values
        .get(1)
        .and_then(RawValue::as_table)
        .expect("second result is the continuation marker table");
    let marker_key = RawValue::String(crate::string::LuaString::from("answer"));

    assert_eq!(
        runtime.raw_get(marker, &marker_key).unwrap(),
        RawValue::Integer(42)
    );
}
