use orbit_vm::{LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmResult};

use crate::{
    argument::{check_integer, required_string},
    error,
    string::pattern::{self, CaptureValue, Match},
};

pub(crate) const FUNCTION: &str = "gsub";

const FUNCTION_REPLACEMENT: NativeToken = NativeToken::new(1);
const TABLE_REPLACEMENT: NativeToken = NativeToken::new(2);

const CHUNKS: usize = 0;
const CHUNK_COUNT: usize = 1;
const CURSOR: usize = 2;
const SUBSTITUTION_COUNT: usize = 3;
const CHANGED: usize = 4;
const MATCH_START: usize = 5;
const MATCH_END: usize = 6;

struct Progress<'context> {
    chunks: LocalValue<'context>,
    chunk_count: usize,
    cursor: usize,
    last_end: Option<usize>,
    substitution_count: i64,
    changed: bool,
}

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume {
            token: FUNCTION_REPLACEMENT | TABLE_REPLACEMENT,
        } => resume_replacement(context),
        NativeEvent::ResumeError {
            token: FUNCTION_REPLACEMENT | TABLE_REPLACEMENT,
        } => Err(context
            .resume_error()
            .expect("replacement error contains an error")
            .clone()),
        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => Err(error::failure(
            format!("invalid continuation token {} in '{FUNCTION}'", token.get(),),
        )),
    }
}

fn start<'context>(context: &mut NativeContext<'context>) -> VmResult<NativeAction> {
    checked_arguments(context)?;

    let progress = Progress {
        chunks: context.create_table(0, 0)?,
        chunk_count: 0,
        cursor: 0,
        last_end: None,
        substitution_count: 0,
        changed: false,
    };
    run(context, progress, Vec::new())
}

fn checked_arguments<'context>(
    context: &NativeContext<'context>,
) -> VmResult<(
    LocalValue<'context>,
    LocalValue<'context>,
    LocalValue<'context>,
    i64,
)> {
    let subject = required_string(context, FUNCTION, 0)?;
    let pattern = required_string(context, FUNCTION, 1)?;
    let replacement = context
        .argument(2)
        .ok_or_else(|| error::type_error(FUNCTION, 3, "string/function/table", None))?;

    if !matches!(
        replacement.type_name(),
        "string" | "number" | "function" | "table"
    ) {
        return Err(error::type_error(
            FUNCTION,
            3,
            "string/function/table",
            Some(replacement.type_name()),
        ));
    }

    let subject_len = subject
        .as_string()
        .expect("required string is a string")
        .len();
    let default_limit = i64::try_from(subject_len)
        .unwrap_or(i64::MAX)
        .saturating_add(1);
    let limit = match context.argument(3) {
        None => default_limit,
        Some(value) if value.is_nil() => default_limit,
        Some(value) => check_integer(&value, FUNCTION, 4)?,
    };

    Ok((subject, pattern, replacement, limit))
}

fn run<'context>(
    context: &mut NativeContext<'context>,
    mut progress: Progress<'context>,
    mut output: Vec<u8>,
) -> VmResult<NativeAction> {
    let (subject_value, pattern_value, replacement, limit) = checked_arguments(context)?;
    let subject = subject_value
        .as_string()
        .expect("required string is a string")
        .as_bytes();
    let full_pattern = pattern_value
        .as_string()
        .expect("required pattern is a string")
        .as_bytes();
    let anchored = full_pattern.first() == Some(&b'^');
    let pattern = if anchored {
        &full_pattern[1..]
    } else {
        full_pattern
    };

    while progress.substitution_count < limit {
        match pattern::match_at(subject, pattern, progress.cursor)? {
            Some(found) if Some(found.end) != progress.last_end => {
                progress.substitution_count += 1;
                progress.cursor = found.end;
                progress.last_end = Some(found.end);

                match replacement.type_name() {
                    "string" | "number" => {
                        let replacement = if replacement.type_name() == "string" {
                            replacement.clone()
                        } else {
                            context.default_tostring(&replacement, None)
                        };
                        let replacement = replacement
                            .as_string()
                            .expect("string replacement is a string")
                            .as_bytes();
                        append_string_replacement(&mut output, replacement, subject, &found)?;
                        progress.changed = true;
                    }
                    "function" => {
                        let arguments = capture_values(context, subject, &found);
                        push_output_chunk(
                            context,
                            &progress.chunks,
                            &mut progress.chunk_count,
                            output,
                        )?;
                        let continuation = continuation(context, progress, found.start, found.end);

                        return Ok(context.call_with_continuation(
                            replacement,
                            arguments,
                            continuation,
                            FUNCTION_REPLACEMENT,
                        ));
                    }
                    "table" => {
                        let key = first_capture(context, subject, &found);
                        push_output_chunk(
                            context,
                            &progress.chunks,
                            &mut progress.chunk_count,
                            output,
                        )?;
                        let continuation = continuation(context, progress, found.start, found.end);

                        return Ok(context.get_with_continuation(
                            replacement,
                            key,
                            continuation,
                            TABLE_REPLACEMENT,
                        ));
                    }
                    _ => unreachable!("replacement type was validated"),
                }
            }
            _ if progress.cursor < subject.len() => {
                output.push(subject[progress.cursor]);
                progress.cursor += 1;
            }
            _ => break,
        }

        if anchored {
            break;
        }
    }

    output.extend_from_slice(&subject[progress.cursor..]);
    finish(
        context,
        &subject_value,
        &progress.chunks,
        progress.chunk_count,
        output,
        progress.substitution_count,
        progress.changed,
    )
}

fn continuation<'context>(
    context: &NativeContext<'context>,
    progress: Progress<'context>,
    match_start: usize,
    match_end: usize,
) -> [LocalValue<'context>; 7] {
    [
        progress.chunks,
        context.integer(offset_to_integer(progress.chunk_count)),
        context.integer(offset_to_integer(progress.cursor)),
        context.integer(progress.substitution_count),
        context.boolean(progress.changed),
        context.integer(offset_to_integer(match_start)),
        context.integer(offset_to_integer(match_end)),
    ]
}

fn resume_replacement(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let match_start = continuation_offset(context, MATCH_START)?;
    let match_end = continuation_offset(context, MATCH_END)?;
    let mut progress = Progress {
        chunks: continuation_table(context, CHUNKS)?,
        chunk_count: continuation_offset(context, CHUNK_COUNT)?,
        cursor: continuation_offset(context, CURSOR)?,
        last_end: Some(match_end),
        substitution_count: continuation_count(context, SUBSTITUTION_COUNT)?,
        changed: continuation_boolean(context, CHANGED)?,
    };

    let (subject_value, _, _, _) = checked_arguments(context)?;
    let subject = subject_value
        .as_string()
        .expect("required string is a string")
        .as_bytes();
    if match_start > match_end || match_end > subject.len() || progress.cursor != match_end {
        return Err(invalid_continuation());
    }

    let result = context.resume_value(0).unwrap_or_else(|| context.nil());
    let (chunk, replacement_happened) =
        replacement_result(context, result, &subject[match_start..match_end])?;
    progress.changed |= replacement_happened;
    push_chunk(context, &progress.chunks, &mut progress.chunk_count, chunk)?;

    run(context, progress, Vec::new())
}

fn replacement_result<'context>(
    context: &NativeContext<'context>,
    result: LocalValue<'context>,
    original: &[u8],
) -> VmResult<(LocalValue<'context>, bool)> {
    if !result.is_truthy() {
        return Ok((context.string(original), false));
    }

    match result.type_name() {
        "string" => Ok((result, true)),
        "number" => Ok((context.default_tostring(&result, None), true)),
        actual => Err(error::failure(format!(
            "invalid replacement value (a {actual})"
        ))),
    }
}

fn append_string_replacement(
    output: &mut Vec<u8>,
    replacement: &[u8],
    subject: &[u8],
    found: &Match,
) -> VmResult<()> {
    let mut literal_start = 0;
    let mut cursor = 0;

    while cursor < replacement.len() {
        if replacement[cursor] != b'%' {
            cursor += 1;
            continue;
        }

        output.extend_from_slice(&replacement[literal_start..cursor]);
        let Some(&escape) = replacement.get(cursor + 1) else {
            return Err(invalid_replacement_escape());
        };

        match escape {
            b'%' => output.push(b'%'),
            b'0' => output.extend_from_slice(&subject[found.start..found.end]),
            digit @ b'1'..=b'9' => {
                append_capture(output, subject, found, digit)?;
            }
            _ => return Err(invalid_replacement_escape()),
        }

        cursor += 2;
        literal_start = cursor;
    }

    output.extend_from_slice(&replacement[literal_start..]);
    Ok(())
}

fn append_capture(output: &mut Vec<u8>, subject: &[u8], found: &Match, digit: u8) -> VmResult<()> {
    let index = usize::from(digit - b'1');

    if found.captures.is_empty() {
        if index == 0 {
            output.extend_from_slice(&subject[found.start..found.end]);
            return Ok(());
        }
        return Err(invalid_capture_index(digit));
    }

    let capture = found
        .captures
        .get(index)
        .ok_or_else(|| invalid_capture_index(digit))?;
    match capture {
        CaptureValue::Text { start, end } => output.extend_from_slice(&subject[*start..*end]),
        CaptureValue::Position(position) => {
            output.extend_from_slice(position.to_string().as_bytes());
        }
    }

    Ok(())
}

fn capture_values<'context>(
    context: &NativeContext<'context>,
    subject: &[u8],
    found: &Match,
) -> Vec<LocalValue<'context>> {
    if found.captures.is_empty() {
        return vec![context.string(&subject[found.start..found.end])];
    }

    found
        .captures
        .iter()
        .map(|capture| capture_value(context, subject, capture))
        .collect()
}

fn first_capture<'context>(
    context: &NativeContext<'context>,
    subject: &[u8],
    found: &Match,
) -> LocalValue<'context> {
    found.captures.first().map_or_else(
        || context.string(&subject[found.start..found.end]),
        |capture| capture_value(context, subject, capture),
    )
}

fn capture_value<'context>(
    context: &NativeContext<'context>,
    subject: &[u8],
    capture: &CaptureValue,
) -> LocalValue<'context> {
    match capture {
        CaptureValue::Text { start, end } => context.string(&subject[*start..*end]),
        CaptureValue::Position(position) => context.integer(offset_to_integer(*position)),
    }
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

    push_chunk(context, chunks, chunk_count, context.string(output))
}

fn push_chunk<'context>(
    context: &mut NativeContext<'context>,
    chunks: &LocalValue<'context>,
    chunk_count: &mut usize,
    chunk: LocalValue<'context>,
) -> VmResult<()> {
    if chunk.as_string().is_some_and(|string| string.is_empty()) {
        return Ok(());
    }

    *chunk_count = chunk_count
        .checked_add(1)
        .ok_or_else(|| error::failure("string.gsub result has too many chunks"))?;
    let key = context.integer(offset_to_integer(*chunk_count));
    context.raw_set(chunks, key, chunk)
}

fn finish<'context>(
    context: &mut NativeContext<'context>,
    subject: &LocalValue<'context>,
    chunks: &LocalValue<'context>,
    chunk_count: usize,
    tail: Vec<u8>,
    substitution_count: i64,
    changed: bool,
) -> VmResult<NativeAction> {
    if !changed {
        return Ok(context.return_values([subject.clone(), context.integer(substitution_count)]));
    }

    if chunk_count == 0 {
        return Ok(
            context.return_values([context.string(tail), context.integer(substitution_count)])
        );
    }

    let mut length = tail.len();
    let mut values = Vec::with_capacity(chunk_count);
    for index in 1..=chunk_count {
        let key = context.integer(offset_to_integer(index));
        let value = context.raw_get(chunks, &key)?;
        let string = value.as_string().ok_or_else(invalid_continuation)?;
        length = length
            .checked_add(string.len())
            .filter(|&length| length <= isize::MAX as usize)
            .ok_or_else(|| error::failure("resulting string too large"))?;
        values.push(value);
    }

    let mut output = Vec::with_capacity(length);
    for value in values {
        output.extend_from_slice(
            value
                .as_string()
                .expect("validated gsub chunk is a string")
                .as_bytes(),
        );
    }
    output.extend_from_slice(&tail);

    Ok(context.return_values([context.string(output), context.integer(substitution_count)]))
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

fn continuation_offset(context: &NativeContext<'_>, index: usize) -> VmResult<usize> {
    context
        .continuation_value(index)
        .and_then(|value| value.as_integer())
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(invalid_continuation)
}

fn continuation_count(context: &NativeContext<'_>, index: usize) -> VmResult<i64> {
    context
        .continuation_value(index)
        .and_then(|value| value.as_integer())
        .filter(|&value| value >= 0)
        .ok_or_else(invalid_continuation)
}

fn continuation_boolean(context: &NativeContext<'_>, index: usize) -> VmResult<bool> {
    context
        .continuation_value(index)
        .and_then(|value| value.as_boolean())
        .ok_or_else(invalid_continuation)
}

fn invalid_replacement_escape() -> orbit_vm::VmError {
    error::failure("invalid use of '%' in replacement string")
}

fn invalid_capture_index(digit: u8) -> orbit_vm::VmError {
    error::failure(format!("invalid capture index %{}", char::from(digit)))
}

fn invalid_continuation() -> orbit_vm::VmError {
    error::failure("invalid string.gsub continuation")
}

fn offset_to_integer(offset: usize) -> i64 {
    i64::try_from(offset).expect("Lua string offsets fit in i64")
}
