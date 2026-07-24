use orbit_vm::{LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmResult};

use crate::{argument, error};

use super::formatting::{Conversion, FormatSpec, quoted_float};

pub(crate) const FUNCTION_NAME: &str = "format";

const TOSTRING_CALL: NativeToken = NativeToken::new(1);

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume {
            token: TOSTRING_CALL,
        } => resume(context),
        NativeEvent::ResumeError {
            token: TOSTRING_CALL,
        } => Err(context
            .resume_error()
            .expect("resume error contains an error")
            .clone()),
        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => {
            Err(error::failure(format!(
                "invalid continuation token {} in '{FUNCTION_NAME}'",
                token.get(),
            )))
        }
    }
}

fn start<'context>(context: &mut NativeContext<'context>) -> VmResult<NativeAction> {
    run(context, None, 0, Vec::new(), 0, 1)
}

fn resume(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let chunks = continuation_chunks(context, 0)?;
    let mut chunk_count = continuation_index(context, 1)?;
    let cursor = continuation_index(context, 2)?;
    let argument_index = continuation_index(context, 3)?;

    let result = context
        .resume_value(0)
        .ok_or_else(|| error::failure("'tostring' returned no value"))?;

    let string = result
        .as_string()
        .ok_or_else(|| error::failure("'tostring' must return a string"))?;

    let format_value = argument::required_string(context, FUNCTION_NAME, 0)?;
    let format = format_value
        .as_string()
        .expect("required_string returns a string")
        .as_bytes();

    let (spec, consumed) =
        FormatSpec::parse(&format[cursor..]).map_err(|error| error::failure(error.to_string()))?;

    if spec.conversion_byte() != b's' {
        return Err(error::failure("invalid string.format continuation"));
    }

    let has_modifiers = spec.has_modifiers();

    if has_modifiers && string.as_bytes().contains(&0) {
        return Err(error::argument_error(
            FUNCTION_NAME,
            argument_index + 1,
            "string contains zeros",
        ));
    }

    let Conversion::String {
        left_align,
        width,
        precision,
    } = validate(spec)?
    else {
        return Err(error::failure("invalid string.format continuation"));
    };

    let mut output = Vec::new();
    if has_modifiers {
        append_string(&mut output, string.as_bytes(), left_align, width, precision);
    } else {
        push_chunk(context, &chunks, &mut chunk_count, result)?;
    }

    run(
        context,
        Some(chunks),
        chunk_count,
        output,
        cursor + consumed,
        argument_index + 1,
    )
}

fn continuation_chunks<'context>(
    context: &NativeContext<'context>,
    index: usize,
) -> VmResult<LocalValue<'context>> {
    let value = context
        .continuation_value(index)
        .ok_or_else(|| error::failure("invalid string.format continuation"))?;

    if value.type_name() != "table" {
        return Err(error::failure("invalid string.format continuation"));
    }

    Ok(value)
}

fn continuation_index(context: &NativeContext<'_>, index: usize) -> VmResult<usize> {
    let value = context
        .continuation_value(index)
        .and_then(|value| value.as_integer())
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| error::failure("invalid string.format continuation"))?;

    Ok(value)
}

fn run<'context>(
    context: &mut NativeContext<'context>,
    chunks: Option<LocalValue<'context>>,
    mut chunk_count: usize,
    mut output: Vec<u8>,
    mut cursor: usize,
    mut argument_index: usize,
) -> VmResult<NativeAction> {
    let format_value = argument::required_string(context, FUNCTION_NAME, 0)?;
    let format = format_value
        .as_string()
        .expect("required_string returns a string")
        .as_bytes();

    while cursor < format.len() {
        if format[cursor] != b'%' {
            output.push(format[cursor]);
            cursor += 1;
            continue;
        }

        let conversion_start = cursor;
        if format.get(cursor + 1) == Some(&b'%') {
            output.push(b'%');
            cursor += 2;
            continue;
        }

        if context.argument(argument_index).is_none() {
            return Err(error::argument_error(
                FUNCTION_NAME,
                argument_index + 1,
                "no value",
            ));
        }

        let (spec, consumed) = FormatSpec::parse(&format[cursor..])
            .map_err(|error| error::failure(error.to_string()))?;

        let value = context
            .argument(argument_index)
            .ok_or_else(|| error::argument_error(FUNCTION_NAME, argument_index + 1, "no value"))?;

        if spec.conversion_byte() == b's' {
            let tostring = context.capture(0).expect("format captures tostring");
            let chunks = match chunks {
                Some(chunks) => chunks,
                None => context.create_table(0, 0)?,
            };
            push_output_chunk(context, &chunks, &mut chunk_count, output)?;

            return Ok(context.call_with_continuation(
                tostring,
                [value],
                [
                    chunks,
                    context.integer(i64::try_from(chunk_count).expect("chunk count fits in i64")),
                    context.integer(
                        i64::try_from(conversion_start).expect("format cursor fits in i64"),
                    ),
                    context.integer(
                        i64::try_from(argument_index).expect("argument index fits in i64"),
                    ),
                ],
                TOSTRING_CALL,
            ));
        }

        render_immediate(context, &mut output, spec, argument_index, &value)?;

        argument_index += 1;
        cursor += consumed;
    }

    finish(context, chunks.as_ref(), chunk_count, output)
}

fn push_output_chunk<'context>(
    context: &mut NativeContext<'context>,
    chunks: &LocalValue<'context>,
    chunk_count: &mut usize,
    output: Vec<u8>,
) -> VmResult<()> {
    if output.is_empty() {
        return Ok(());
    }

    let output = context.string(output);
    push_chunk(context, chunks, chunk_count, output)
}

fn push_chunk<'context>(
    context: &mut NativeContext<'context>,
    chunks: &LocalValue<'context>,
    chunk_count: &mut usize,
    chunk: LocalValue<'context>,
) -> VmResult<()> {
    *chunk_count = chunk_count
        .checked_add(1)
        .ok_or_else(|| error::failure("string.format result has too many chunks"))?;
    let index = i64::try_from(*chunk_count)
        .map_err(|_| error::failure("string.format result has too many chunks"))?;
    let index = context.integer(index);
    context.raw_set(chunks, index, chunk)
}

fn finish<'context>(
    context: &mut NativeContext<'context>,
    chunks: Option<&LocalValue<'context>>,
    chunk_count: usize,
    tail: Vec<u8>,
) -> VmResult<NativeAction> {
    let Some(chunks) = chunks else {
        debug_assert_eq!(chunk_count, 0);
        return Ok(context.return_values([context.string(tail)]));
    };
    if chunk_count == 0 {
        return Ok(context.return_values([context.string(tail)]));
    }

    let mut length = tail.len();
    let mut values = Vec::with_capacity(chunk_count);

    for index in 1..=chunk_count {
        let index = i64::try_from(index)
            .map_err(|_| error::failure("invalid string.format continuation"))?;
        let key = context.integer(index);
        let value = context.raw_get(chunks, &key)?;
        let string = value
            .as_string()
            .ok_or_else(|| error::failure("invalid string.format continuation"))?;
        length = length
            .checked_add(string.len())
            .ok_or_else(|| error::failure("string.format result is too large"))?;
        values.push(value);
    }

    let mut output = Vec::with_capacity(length);
    for value in values {
        let string = value
            .as_string()
            .expect("validated string.format chunk is a string");
        output.extend_from_slice(string.as_bytes());
    }
    output.extend(tail);

    Ok(context.return_values([context.string(output)]))
}

fn append_string(
    output: &mut Vec<u8>,
    value: &[u8],
    left_align: bool,
    width: Option<u8>,
    precision: Option<u8>,
) {
    let value = match precision {
        Some(precision) => &value[..value.len().min(usize::from(precision))],
        None => value,
    };

    let padding = width
        .map(usize::from)
        .unwrap_or_default()
        .saturating_sub(value.len());

    if !left_align {
        output.resize(output.len() + padding, b' ');
    }

    output.extend_from_slice(value);

    if left_align {
        output.resize(output.len() + padding, b' ');
    }
}

fn render_immediate<'context>(
    context: &NativeContext<'context>,
    output: &mut Vec<u8>,
    spec: FormatSpec,
    argument_index: usize,
    value: &LocalValue<'context>,
) -> VmResult<()> {
    match spec.conversion_byte() {
        b'c' => {
            let conversion = validate(spec)?;
            let integer = argument::check_integer(value, FUNCTION_NAME, argument_index + 1)?;

            conversion.append_integer(output, integer);
            Ok(())
        }
        b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
            let integer = argument::check_integer(value, FUNCTION_NAME, argument_index + 1)?;
            let conversion = validate(spec)?;

            conversion.append_integer(output, integer);
            Ok(())
        }
        b'a' | b'A' => {
            let conversion = validate(spec)?;
            let number = check_number(value, argument_index)?;

            conversion.append_float(output, number);
            Ok(())
        }
        b'e' | b'E' | b'f' | b'g' | b'G' => {
            let number = check_number(value, argument_index)?;
            let conversion = validate(spec)?;

            conversion.append_float(output, number);
            Ok(())
        }
        b'p' => {
            let pointer = context.pointer_representation(value);
            let conversion = validate(spec)?;
            let representation = pointer
                .as_string()
                .expect("pointer representation is a string");
            conversion.append_pointer(output, representation.as_bytes());
            Ok(())
        }
        b'q' => {
            validate(spec)?;
            append_quoted(output, value, argument_index)
        }
        b's' => unreachable!("non-immediate string.format conversion"),
        _ => {
            validate(spec)?;
            unreachable!("unknown format conversion passed validation")
        }
    }
}

fn validate(spec: FormatSpec) -> VmResult<Conversion> {
    spec.validate()
        .map_err(|error| error::failure(error.to_string()))
}

fn check_number(value: &LocalValue<'_>, argument_index: usize) -> VmResult<f64> {
    value.to_float().ok_or_else(|| {
        error::type_error(
            FUNCTION_NAME,
            argument_index + 1,
            "number",
            Some(value.type_name()),
        )
    })
}

fn append_quoted(
    output: &mut Vec<u8>,
    value: &LocalValue<'_>,
    argument_index: usize,
) -> VmResult<()> {
    if value.is_nil() {
        output.extend_from_slice(b"nil");
    } else if let Some(value) = value.as_boolean() {
        output.extend_from_slice(if value { b"true" } else { b"false" });
    } else if let Some(value) = value.as_integer() {
        if value == i64::MIN {
            output.extend_from_slice(b"0x8000000000000000");
        } else {
            output.extend_from_slice(value.to_string().as_bytes());
        }
    } else if let Some(value) = value.as_float() {
        output.extend(quoted_float(value));
    } else if let Some(value) = value.as_string() {
        Conversion::append_quoted_string(output, value.as_bytes());
    } else {
        return Err(error::argument_error(
            FUNCTION_NAME,
            argument_index + 1,
            "value has no literal form",
        ));
    }

    Ok(())
}
