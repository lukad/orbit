use std::ops::Deref;

use orbit_vm::{LocalValue, LuaString, NativeContext, VmResult};

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

#[derive(Debug, Clone)]
pub(crate) struct CheckedString<'context>(LocalValue<'context>);

impl<'context> CheckedString<'context> {
    pub(crate) fn into_value(self) -> LocalValue<'context> {
        self.0
    }
}

impl Deref for CheckedString<'_> {
    type Target = LuaString;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_string()
            .expect("CheckedString always contains a string")
    }
}

pub(crate) fn check_string<'context>(
    context: &NativeContext<'context>,
    value: LocalValue<'context>,
    function: &'static str,
    index: usize,
) -> VmResult<CheckedString<'context>> {
    let value = match value.type_name() {
        "string" => value,
        "number" => context.default_tostring(&value, None),
        _ => {
            return Err(error::type_error(
                function,
                index + 1,
                "string",
                Some(value.type_name()),
            ));
        }
    };

    Ok(CheckedString(value))
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
) -> VmResult<CheckedString<'context>> {
    let value = context
        .argument(index)
        .ok_or_else(|| error::type_error(function, index + 1, "string", None))?;

    check_string(context, value, function, index)
}
