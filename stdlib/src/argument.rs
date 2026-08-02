use orbit_vm::{LocalValue, NativeContext, VmResult};

use crate::error;

pub(crate) fn check_integer(
    value: &LocalValue<'_>,
    function: &'static str,
    argument: usize,
) -> VmResult<i64> {
    match value.to_integer() {
        Some(value) => Ok(value),
        None if value.is_number() => Err(error::number_has_no_integer_representation(
            function, argument,
        )),
        None => Err(error::type_error(
            function,
            argument,
            "number",
            Some(value.type_name()),
        )),
    }
}

pub(crate) fn check_float(
    value: &LocalValue<'_>,
    function: &'static str,
    argument: usize,
) -> VmResult<f64> {
    match value.to_float() {
        Some(value) => Ok(value),
        None => Err(error::type_error(
            function,
            argument,
            "number",
            Some(value.type_name()),
        )),
    }
}

pub(crate) fn required_integer(
    context: &NativeContext<'_>,
    function: &'static str,
    index: usize,
) -> VmResult<i64> {
    let value = context
        .argument(index)
        .ok_or_else(|| error::type_error(function, index + 1, "number", None))?;

    check_integer(&value, function, index + 1)
}

pub(crate) fn required_number(
    context: &NativeContext<'_>,
    function: &'static str,
    index: usize,
) -> VmResult<f64> {
    let value = context
        .argument(index)
        .ok_or_else(|| error::type_error(function, index + 1, "number", None))?;

    value
        .to_float()
        .ok_or_else(|| error::type_error(function, index + 1, "number", Some(value.type_name())))
}

pub(crate) fn required_string<'context>(
    context: &NativeContext<'context>,
    function: &'static str,
    index: usize,
) -> VmResult<LocalValue<'context>> {
    let value = context
        .argument(index)
        .ok_or_else(|| error::type_error(function, index + 1, "string", None))?;

    match value.type_name() {
        "string" => Ok(value),
        "number" => Ok(context.default_tostring(&value, None)),
        _ => Err(error::type_error(
            function,
            index + 1,
            "string",
            Some(value.type_name()),
        )),
    }
}
