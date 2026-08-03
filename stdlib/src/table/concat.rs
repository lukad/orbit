use orbit_vm::{
    LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmError, VmResult,
};

use crate::{
    argument::{check_integer, check_string},
    error,
    table::access::{
        LengthDispatch, TableCapabilities, call_length_metamethod, metamethod, required_table_like,
        resolve_length,
    },
};

pub const FUNCTION_NAME: &str = "concat";

const LENGTH_RESULT: NativeToken = NativeToken::new(1);
const ELEMENT_RESULT: NativeToken = NativeToken::new(2);

const CURRENT_INDEX: usize = 0;
const END_INDEX: usize = 1;
const SEPARATOR: usize = 2;
const OUTPUT: usize = 3;

const REQUIRED_CAPABILITIES: TableCapabilities =
    TableCapabilities::READ.union(TableCapabilities::LENGTH);

pub fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
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
    let target = required_table_like(context, FUNCTION_NAME, 0, REQUIRED_CAPABILITIES)?;

    match resolve_length(context, &target)? {
        LengthDispatch::Immediate(length) => begin_with_length(context, target, length),
        LengthDispatch::Metamethod(metamethod) => Ok(call_length_metamethod(
            context,
            metamethod,
            target,
            [],
            LENGTH_RESULT,
        )),
    }
}

fn resume_length(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let target = original_target(context)?;
    let length = context
        .resume_value(0)
        .and_then(|value| value.to_integer())
        .ok_or_else(|| error::failure("object length is not an integer"))?;

    begin_with_length(context, target, length)
}

fn begin_with_length<'context>(
    context: &mut NativeContext<'context>,
    target: LocalValue<'context>,
    length: i64,
) -> VmResult<NativeAction> {
    let separator = match context.argument(1) {
        None => context.string([]),
        Some(value) if value.is_nil() => context.string([]),
        Some(value) => check_string(context, value, FUNCTION_NAME, 1)?.into_value(),
    };

    let start = match context.argument(2) {
        None => 1,
        Some(value) if value.is_nil() => 1,
        Some(value) => check_integer(&value, FUNCTION_NAME, 3)?,
    };

    let end = match context.argument(3) {
        None => length,
        Some(value) if value.is_nil() => length,
        Some(value) => check_integer(&value, FUNCTION_NAME, 4)?,
    };

    if start > end {
        return Ok(context.return_values([context.string([])]));
    }

    if target.type_name() == "table" && metamethod(context, &target, "__index")?.is_none() {
        return concat_raw(context, &target, start, end, &separator);
    }

    Ok(read_element(
        context,
        target,
        start,
        end,
        separator,
        Vec::new(),
    ))
}

fn concat_raw<'context>(
    context: &mut NativeContext<'context>,
    table: &LocalValue<'context>,
    start: i64,
    end: i64,
    separator: &LocalValue<'context>,
) -> VmResult<NativeAction> {
    let separator = separator
        .as_string()
        .expect("validated concat separator is a string")
        .as_bytes();
    let mut output = Vec::new();
    let mut current = start;

    loop {
        let value = context.raw_get(table, &context.integer(current))?;
        append_value(context, &mut output, value, current)?;

        if current == end {
            break;
        }

        output.extend_from_slice(separator);
        current = current
            .checked_add(1)
            .expect("a concat index below the end cannot overflow");
    }

    Ok(context.return_values([context.string(output)]))
}

fn resume_element(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let target = original_target(context)?;
    let current = continuation_integer(context, CURRENT_INDEX)?;
    let end = continuation_integer(context, END_INDEX)?;
    let separator = continuation_string(context, SEPARATOR)?;
    let output = continuation_string(context, OUTPUT)?;
    let mut output = output
        .as_string()
        .expect("validated concat output is a string")
        .as_bytes()
        .to_vec();

    let value = context.resume_value(0).unwrap_or_else(|| context.nil());
    append_value(context, &mut output, value, current)?;

    if current == end {
        return Ok(context.return_values([context.string(output)]));
    }

    output.extend_from_slice(
        separator
            .as_string()
            .expect("validated concat separator is a string")
            .as_bytes(),
    );

    let next = current
        .checked_add(1)
        .expect("a concat index below the end cannot overflow");

    Ok(read_element(context, target, next, end, separator, output))
}

fn append_value<'context>(
    context: &NativeContext<'context>,
    output: &mut Vec<u8>,
    value: LocalValue<'context>,
    index: i64,
) -> VmResult<()> {
    let value_type = value.type_name();
    let value = check_string(context, value, FUNCTION_NAME, 0).map_err(|_| {
        error::failure(format!(
            "invalid value ({value_type}) at index {index} in table for 'concat'"
        ))
    })?;
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn read_element<'context>(
    context: &NativeContext<'context>,
    target: LocalValue<'context>,
    current: i64,
    end: i64,
    separator: LocalValue<'context>,
    output: Vec<u8>,
) -> NativeAction {
    context.get_with_continuation(
        target,
        context.integer(current),
        [
            context.integer(current),
            context.integer(end),
            separator,
            context.string(output),
        ],
        ELEMENT_RESULT,
    )
}

fn continuation_integer(context: &NativeContext<'_>, index: usize) -> VmResult<i64> {
    context
        .continuation_value(index)
        .and_then(|value| value.as_integer())
        .ok_or_else(invalid_continuation)
}

fn original_target<'context>(context: &NativeContext<'context>) -> VmResult<LocalValue<'context>> {
    context.argument(0).ok_or_else(invalid_continuation)
}

fn continuation_string<'context>(
    context: &NativeContext<'context>,
    index: usize,
) -> VmResult<LocalValue<'context>> {
    let value = context
        .continuation_value(index)
        .ok_or_else(invalid_continuation)?;

    if value.as_string().is_none() {
        return Err(invalid_continuation());
    }

    Ok(value)
}

fn invalid_continuation() -> VmError {
    error::failure("invalid table.concat continuation")
}

fn invalid_token(token: NativeToken) -> VmError {
    error::failure(format!(
        "invalid continuation token {} in 'table.concat'",
        token.get(),
    ))
}
