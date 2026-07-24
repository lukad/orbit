use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{argument, error};

const FUNCTION_NAME: &str = "sub";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let subject = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let string = subject
        .as_string()
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "string", Some(subject.type_name())))?;

    let start = match context.argument(1) {
        Some(value) => argument::check_integer(&value, FUNCTION_NAME, 2)?,
        None => {
            return Err(error::type_error(FUNCTION_NAME, 2, "number", None));
        }
    };

    let end = match context.argument(2) {
        None => -1,
        Some(value) if value.is_nil() => -1,
        Some(value) => argument::check_integer(&value, FUNCTION_NAME, 3)?,
    };

    let len = string.len();
    let start = start_offset(start, len);
    let end = end_offset(end, len);

    let result = if start < end {
        &string.as_bytes()[start..end]
    } else {
        &[]
    };

    Ok(context.return_values([context.string(result)]))
}

fn start_offset(position: i64, len: usize) -> usize {
    if position > 0 {
        usize::try_from(position - 1).unwrap_or(usize::MAX).min(len)
    } else if position == 0 {
        0
    } else {
        let distance = usize::try_from(position.unsigned_abs()).unwrap_or(usize::MAX);
        len.saturating_sub(distance)
    }
}

fn end_offset(position: i64, len: usize) -> usize {
    if position >= 0 {
        usize::try_from(position).unwrap_or(usize::MAX).min(len)
    } else {
        let distance = usize::try_from(position.unsigned_abs()).unwrap_or(usize::MAX);
        if distance > len {
            0
        } else {
            len - distance + 1
        }
    }
}
