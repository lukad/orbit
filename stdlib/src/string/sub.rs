use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{
    argument::{self, required_integer, required_string},
    string::offsets::{end_offset, start_offset},
};

pub(crate) const FUNCTION_NAME: &str = "sub";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let string = required_string(context, FUNCTION_NAME, 0)?;
    let string = string
        .as_string()
        .expect("required string is a string")
        .as_bytes();

    let start = required_integer(context, FUNCTION_NAME, 1)?;

    let end = match context.argument(2) {
        None => -1,
        Some(value) if value.is_nil() => -1,
        Some(value) => argument::check_integer(&value, FUNCTION_NAME, 3)?,
    };

    let len = string.len();
    let start = start_offset(start, len);
    let end = end_offset(end, len);

    let result = if start < end {
        &string[start..end]
    } else {
        &[]
    };

    Ok(context.return_values([context.string(result)]))
}
