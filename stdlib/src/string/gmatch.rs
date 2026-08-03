use orbit_vm::{LocalValue, NativeAction, NativeContext, VmResult};

use crate::{
    argument::{check_integer, required_string},
    offsets::start_offset,
    string::pattern::{self, CaptureValue, Match},
};

pub(crate) const FUNCTION: &str = "gmatch";

const ITERATOR_NAME: &str = "string.gmatch iterator";

const SUBJECT_CAPTURE: usize = 0;
const PATTERN_CAPTURE: usize = 1;
const STATE_CAPTURE: usize = 2;

const CURSOR_KEY: i64 = 1;
const LAST_END_KEY: i64 = 2;
const NO_OFFSET: i64 = -1;

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let subject = required_string(context, FUNCTION, 0)?;
    let pattern = required_string(context, FUNCTION, 1)?;

    let requested_start = match context.argument(2) {
        None => 1,
        Some(value) if value.is_nil() => 1,
        Some(value) => check_integer(&value, FUNCTION, 3)?,
    };

    let subject_len = subject.len();

    let past_end =
        requested_start > 0 && (requested_start as u64) > (subject_len as u64).saturating_add(1);

    let cursor = if past_end {
        NO_OFFSET
    } else {
        offset_to_integer(start_offset(requested_start, subject_len))
    };

    let state = context.create_table(2, 0)?;
    context.raw_set(&state, context.integer(CURSOR_KEY), context.integer(cursor))?;
    context.raw_set(
        &state,
        context.integer(LAST_END_KEY),
        context.integer(NO_OFFSET),
    )?;

    let iterator = context.create_native_function(
        ITERATOR_NAME,
        iterator,
        [subject.into_value(), pattern.into_value(), state],
    )?;

    Ok(context.return_values([iterator]))
}

fn iterator(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let subject_value = context
        .capture(SUBJECT_CAPTURE)
        .expect("gmatch iterator has a subject capture");
    let pattern_value = context
        .capture(PATTERN_CAPTURE)
        .expect("gmatch iterator has a pattern capture");
    let state = context
        .capture(STATE_CAPTURE)
        .expect("gmatch iterator has a state capture");

    let cursor_key = context.integer(CURSOR_KEY);
    let last_end_key = context.integer(LAST_END_KEY);
    let cursor = read_state_integer(context, &state, &cursor_key)?;

    if cursor == NO_OFFSET {
        return Ok(context.return_values([]));
    }

    let last_end = read_state_integer(context, &state, &last_end_key)?;
    let last_end = (last_end != NO_OFFSET)
        .then(|| usize::try_from(last_end).expect("gmatch last-match offset is non-negative"));

    let subject = subject_value
        .as_string()
        .expect("gmatch subject capture is a string")
        .as_bytes();
    let pattern = pattern_value
        .as_string()
        .expect("gmatch pattern capture is a string")
        .as_bytes();
    let cursor = usize::try_from(cursor).expect("gmatch cursor is non-negative");

    for start in cursor..=subject.len() {
        let Some(found) = pattern::match_at(subject, pattern, start)? else {
            continue;
        };

        if Some(found.end) == last_end {
            continue;
        }

        let next_cursor = context.integer(offset_to_integer(found.end));
        context.raw_set(&state, cursor_key, next_cursor.clone())?;
        context.raw_set(&state, last_end_key, next_cursor)?;

        return Ok(context.return_values(match_values(context, subject, found)));
    }

    context.raw_set(&state, cursor_key, context.integer(NO_OFFSET))?;
    Ok(context.return_values([]))
}

fn read_state_integer<'context>(
    context: &mut NativeContext<'context>,
    state: &LocalValue<'context>,
    key: &LocalValue<'context>,
) -> VmResult<i64> {
    Ok(context
        .raw_get(state, key)?
        .as_integer()
        .expect("gmatch state value is an integer"))
}

fn match_values<'context>(
    context: &NativeContext<'context>,
    subject: &[u8],
    found: Match,
) -> Vec<LocalValue<'context>> {
    if found.captures.is_empty() {
        return vec![context.string(&subject[found.start..found.end])];
    }

    found
        .captures
        .into_iter()
        .map(|capture| match capture {
            CaptureValue::Text { start, end } => context.string(&subject[start..end]),
            CaptureValue::Position(position) => context.integer(offset_to_integer(position)),
        })
        .collect()
}

fn offset_to_integer(offset: usize) -> i64 {
    i64::try_from(offset).expect("Lua string offsets fit in i64")
}
