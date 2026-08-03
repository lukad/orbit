use orbit_vm::{
    LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmError, VmResult,
};

use crate::{
    argument, error,
    table::access::{LengthDispatch, call_length_metamethod, metamethod, resolve_length},
};

const FUNCTION_NAME: &str = "unpack";
const MAX_UNPACK_RESULTS: usize = 1_000_000;

const LENGTH_RESULT: NativeToken = NativeToken::new(1);
const ELEMENT_RESULT: NativeToken = NativeToken::new(2);

const CURRENT_INDEX: usize = 0;
const END_INDEX: usize = 1;
const RESULTS: usize = 2;
const RESULT_COUNT: usize = 3;

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume {
            token: LENGTH_RESULT,
        } => resume_length(context),
        NativeEvent::Resume {
            token: ELEMENT_RESULT,
        } => resume_element(context),
        NativeEvent::ResumeError {
            token: LENGTH_RESULT | ELEMENT_RESULT,
        } => Err(context
            .resume_error()
            .expect("resume error contains an error")
            .clone()),
        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => {
            Err(invalid_token(token))
        }
    }
}

fn start(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let target = context.argument(0).unwrap_or_else(|| context.nil());
    let start = optional_integer(context, 1, 1)?;

    match context.argument(2) {
        None => resolve_default_end(context, target, start),
        Some(value) if value.is_nil() => resolve_default_end(context, target, start),
        Some(value) => {
            let end = argument::check_integer(&value, FUNCTION_NAME, 3)?;
            unpack_range(context, target, start, end)
        }
    }
}

fn resolve_default_end<'context>(
    context: &mut NativeContext<'context>,
    target: LocalValue<'context>,
    start: i64,
) -> VmResult<NativeAction> {
    match resolve_length(context, &target)? {
        LengthDispatch::Immediate(length) => unpack_range(context, target, start, length),
        LengthDispatch::Metamethod(metamethod) => Ok(call_length_metamethod(
            context,
            metamethod,
            target,
            [context.integer(start)],
            LENGTH_RESULT,
        )),
    }
}

fn resume_length(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let target = context.argument(0).unwrap_or_else(|| context.nil());
    let start = continuation_integer(context, 0)?;
    let length = context
        .resume_value(0)
        .and_then(|value| value.to_integer())
        .ok_or_else(|| error::failure("object length is not an integer"))?;

    unpack_range(context, target, start, length)
}

fn unpack_range<'context>(
    context: &mut NativeContext<'context>,
    target: LocalValue<'context>,
    start: i64,
    end: i64,
) -> VmResult<NativeAction> {
    if start > end {
        return Ok(context.return_values([]));
    }

    let count = result_count(start, end)?;

    if target.type_name() == "table" && metamethod(context, &target, "__index")?.is_none() {
        return unpack_raw(context, &target, start, end, count);
    }

    let results = context.create_table(count, 0)?;
    Ok(read_element(context, target, start, end, results, 0))
}

fn result_count(start: i64, end: i64) -> VmResult<usize> {
    let count = i128::from(end) - i128::from(start) + 1;
    usize::try_from(count)
        .ok()
        .filter(|count| *count <= MAX_UNPACK_RESULTS)
        .ok_or_else(|| error::failure("too many results to unpack"))
}

fn unpack_raw<'context>(
    context: &mut NativeContext<'context>,
    table: &LocalValue<'context>,
    start: i64,
    end: i64,
    count: usize,
) -> VmResult<NativeAction> {
    let mut results = Vec::new();
    results
        .try_reserve_exact(count)
        .map_err(|_| error::failure("too many results to unpack"))?;

    for current in start..=end {
        results.push(context.raw_get(table, &context.integer(current))?);
    }

    Ok(context.return_values(results))
}

fn resume_element(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let target = context.argument(0).unwrap_or_else(|| context.nil());
    let current = continuation_integer(context, CURRENT_INDEX)?;
    let end = continuation_integer(context, END_INDEX)?;
    let results = continuation_table(context, RESULTS)?;
    let result_count = continuation_usize(context, RESULT_COUNT)?;
    let value = context.resume_value(0).unwrap_or_else(|| context.nil());

    let next_result_count = result_count
        .checked_add(1)
        .ok_or_else(invalid_continuation)?;
    let result_key = i64::try_from(next_result_count).map_err(|_| invalid_continuation())?;
    context.raw_set(&results, context.integer(result_key), value)?;

    if current == end {
        return return_results(context, &results, next_result_count);
    }

    let next = current
        .checked_add(1)
        .expect("an unpack index below the end cannot overflow");

    Ok(read_element(
        context,
        target,
        next,
        end,
        results,
        next_result_count,
    ))
}

fn read_element<'context>(
    context: &NativeContext<'context>,
    target: LocalValue<'context>,
    current: i64,
    end: i64,
    results: LocalValue<'context>,
    result_count: usize,
) -> NativeAction {
    context.get_with_continuation(
        target,
        context.integer(current),
        [
            context.integer(current),
            context.integer(end),
            results,
            context.integer(i64::try_from(result_count).expect("unpack result count fits in i64")),
        ],
        ELEMENT_RESULT,
    )
}

fn return_results<'context>(
    context: &mut NativeContext<'context>,
    results: &LocalValue<'context>,
    result_count: usize,
) -> VmResult<NativeAction> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(result_count)
        .map_err(|_| error::failure("too many results to unpack"))?;

    for index in 1..=result_count {
        let index = i64::try_from(index).expect("unpack result index fits in i64");
        values.push(context.raw_get(results, &context.integer(index))?);
    }

    Ok(context.return_values(values))
}

fn optional_integer(context: &NativeContext<'_>, index: usize, default: i64) -> VmResult<i64> {
    match context.argument(index) {
        None => Ok(default),
        Some(value) if value.is_nil() => Ok(default),
        Some(value) => argument::check_integer(&value, FUNCTION_NAME, index + 1),
    }
}

fn continuation_integer(context: &NativeContext<'_>, index: usize) -> VmResult<i64> {
    context
        .continuation_value(index)
        .and_then(|value| value.as_integer())
        .ok_or_else(invalid_continuation)
}

fn continuation_usize(context: &NativeContext<'_>, index: usize) -> VmResult<usize> {
    continuation_integer(context, index)
        .and_then(|value| usize::try_from(value).map_err(|_| invalid_continuation()))
}

fn continuation_table<'context>(
    context: &NativeContext<'context>,
    index: usize,
) -> VmResult<LocalValue<'context>> {
    let value = context
        .continuation_value(index)
        .ok_or_else(invalid_continuation)?;

    if value.type_name() != "table" {
        return Err(invalid_continuation());
    }

    Ok(value)
}

fn invalid_continuation() -> VmError {
    error::failure("invalid table.unpack continuation")
}

fn invalid_token(token: NativeToken) -> VmError {
    error::failure(format!(
        "invalid continuation token {} in 'table.unpack'",
        token.get(),
    ))
}
