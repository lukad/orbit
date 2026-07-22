use orbit_vm::{LocalValue, NativeAction, NativeContext, NativeEvent, VmResult};

use super::{check_string, path};
use crate::error;

const FUNCTION_NAME: &str = "searchpath";

#[cfg(windows)]
const DIRECTORY_SEPARATOR: &[u8] = b"\\";

#[cfg(not(windows))]
const DIRECTORY_SEPARATOR: &[u8] = b"/";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    if !matches!(context.event(), NativeEvent::Start) {
        return Err(error::failure(
            "package.searchpath received an unexpected continuation",
        ));
    }

    let name = check_string(context, 0, FUNCTION_NAME)?;
    let search_path = check_string(context, 1, FUNCTION_NAME)?;

    let separator = optional_string(context, 2, b".")?;
    let replacement = optional_string(context, 3, DIRECTORY_SEPARATOR)?;

    let name = name.as_string().unwrap().as_bytes();
    let search_path = search_path.as_string().unwrap().as_bytes();
    let separator = separator.as_string().unwrap().as_bytes();
    let replacement = replacement.as_string().unwrap().as_bytes();

    match path::search(context, name, search_path, separator, replacement) {
        Ok(filename) => Ok(context.return_values([context.string(filename)])),
        Err(message) => Ok(context.return_values([context.nil(), context.string(message)])),
    }
}

fn optional_string<'context>(
    context: &NativeContext<'context>,
    index: usize,
    default: &[u8],
) -> VmResult<LocalValue<'context>> {
    match context.argument(index) {
        None => Ok(context.string(default)),
        Some(value) if value.is_nil() => Ok(context.string(default)),
        Some(value) if value.type_name() == "string" => Ok(value),
        Some(value) if value.type_name() == "number" => Ok(context.default_tostring(&value, None)),
        Some(value) => Err(error::type_error(
            FUNCTION_NAME,
            index + 1,
            "string",
            Some(value.type_name()),
        )),
    }
}
