use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{
    argument::{check_integer, required_string},
    string::offsets::{end_offset, start_offset},
};

pub const FUNCTION_NAME: &str = "byte";

pub fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let string = required_string(context, FUNCTION_NAME, 0)?
        .as_string()
        .expect("required string is a string")
        .as_bytes()
        .to_vec();

    let start = match context.argument(1) {
        None => 1,
        Some(value) if value.is_nil() => 1,
        Some(value) => check_integer(&value, FUNCTION_NAME, 2)?,
    };

    let end = match context.argument(2) {
        None => start,
        Some(value) if value.is_nil() => start,
        Some(value) => check_integer(&value, FUNCTION_NAME, 3)?,
    };

    let len = string.len();
    let start = start_offset(start, len);
    let end = end_offset(end, len);

    let bytes = string
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .map(|byte| context.integer(*byte as i64))
        .collect::<Vec<_>>();

    Ok(context.return_values(bytes))
}
