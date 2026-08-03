use orbit_vm::{NativeAction, NativeContext, NativeEvent, VmResult};

use super::path;
use crate::{
    argument::{CheckedString, check_string, required_string},
    error,
};

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

    let name = required_string(context, FUNCTION_NAME, 0)?;
    let search_path = required_string(context, FUNCTION_NAME, 1)?;

    let separator = optional_string(context, 2)?;
    let replacement = optional_string(context, 3)?;

    let name = name.as_bytes();
    let search_path = search_path.as_bytes();
    let separator = separator
        .as_ref()
        .map_or(b".".as_slice(), |value| value.as_bytes());
    let replacement = replacement
        .as_ref()
        .map_or(DIRECTORY_SEPARATOR, |value| value.as_bytes());

    match path::search(context, name, search_path, separator, replacement) {
        Ok(filename) => Ok(context.return_values([context.string(filename)])),
        Err(message) => Ok(context.return_values([context.nil(), context.string(message)])),
    }
}

fn optional_string<'context>(
    context: &NativeContext<'context>,
    index: usize,
) -> VmResult<Option<CheckedString<'context>>> {
    match context.argument(index) {
        None => Ok(None),
        Some(value) if value.is_nil() => Ok(None),
        Some(value) => check_string(context, value, FUNCTION_NAME, index).map(Some),
    }
}
