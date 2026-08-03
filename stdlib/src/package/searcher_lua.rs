use orbit_vm::{
    LoadSource, LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmError,
    VmResult,
};

use super::path;
use crate::{argument::required_string, error};

const FUNCTION_NAME: &str = "package Lua searcher";
const PATH_LOOKUP: NativeToken = NativeToken::new(1);

#[cfg(windows)]
const DIRECTORY_SEPARATOR: &[u8] = b"\\";

#[cfg(not(windows))]
const DIRECTORY_SEPARATOR: &[u8] = b"/";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume { token: PATH_LOOKUP } => finish_path_lookup(context),
        NativeEvent::ResumeError { token: PATH_LOOKUP } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => {
            Err(invalid_token(token))
        }
    }
}

fn start(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = required_string(context, FUNCTION_NAME, 0)?.into_value();

    let package = context
        .capture(0)
        .expect("Lua searcher captures the package table");
    let path_key = context.string("path");

    Ok(context.get_with_continuation(package, path_key, [name], PATH_LOOKUP))
}

fn finish_path_lookup(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = context
        .continuation_value(0)
        .ok_or_else(|| error::failure("missing Lua searcher continuation value"))?;
    let package_path = context.resume_value(0).unwrap_or_else(|| context.nil());
    let package_path = package_path_string(context, package_path)?;

    let name_bytes = name
        .as_string()
        .expect("checked module name is a string")
        .as_bytes();

    let path_bytes = package_path
        .as_string()
        .expect("checked package path is a string")
        .as_bytes();

    let filename = match path::search(context, name_bytes, path_bytes, b".", DIRECTORY_SEPARATOR) {
        Ok(filename) => filename,
        Err(message) => {
            return Ok(context.return_values([context.string(message)]));
        }
    };

    let loader = context.load_source(
        LoadSource::File {
            filename: &filename,
        },
        None,
    )?;

    let loader_data = context.string(filename);

    Ok(context.return_values([loader, loader_data]))
}

fn invalid_token(token: NativeToken) -> VmError {
    error::failure(format!(
        "invalid continuation token {} in '{FUNCTION_NAME}'",
        token.get(),
    ))
}

fn package_path_string<'context>(
    context: &NativeContext<'context>,
    value: LocalValue<'context>,
) -> VmResult<LocalValue<'context>> {
    match value.type_name() {
        "string" => Ok(value),
        "number" => Ok(context.default_tostring(&value, None)),
        _ => Err(error::failure("'package.path' must be a string")),
    }
}
