use orbit_vm::{LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "tostring";

const TOSTRING_METAMETHOD: &[u8] = b"__tostring";
const NAME_FIELD: &[u8] = b"__name";

const TOSTRING_CALL: NativeToken = NativeToken::new(1);

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume {
            token: TOSTRING_CALL,
        } => finish_metamethod(context),
        NativeEvent::ResumeError {
            token: TOSTRING_CALL,
        } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => {
            Err(error::failure(format!(
                "invalid continuation token {} in '{FUNCTION_NAME}'",
                token.get(),
            )))
        }
    }
}

fn start(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let metatable = context.get_metatable(&value)?;

    if let Some(metatable) = &metatable {
        let key = context.string(TOSTRING_METAMETHOD);
        let metamethod = context.raw_get(metatable, &key)?;

        if !metamethod.is_nil() {
            return Ok(context.call(metamethod, [value], TOSTRING_CALL));
        }
    }

    let name = object_name(context, &value, metatable.as_ref())?;
    let name = name
        .as_ref()
        .and_then(LocalValue::as_string)
        .map(|name| name.as_bytes());

    let result = context.default_tostring(&value, name);

    Ok(context.return_values([result]))
}

fn finish_metamethod(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let result = context
        .resume_value(0)
        .ok_or_else(invalid_metamethod_result)?;

    let result = match result.type_name() {
        "string" => result,
        "number" => context.default_tostring(&result, None),
        _ => return Err(invalid_metamethod_result()),
    };

    Ok(context.return_values([result]))
}

fn object_name<'context>(
    context: &mut NativeContext<'context>,
    value: &LocalValue<'context>,
    metatable: Option<&LocalValue<'context>>,
) -> VmResult<Option<LocalValue<'context>>> {
    if !matches!(value.type_name(), "table" | "function") {
        return Ok(None);
    }

    let Some(metatable) = metatable else {
        return Ok(None);
    };

    let key = context.string(NAME_FIELD);
    let name = context.raw_get(metatable, &key)?;

    if name.as_string().is_some() {
        Ok(Some(name))
    } else {
        Ok(None)
    }
}

fn invalid_metamethod_result() -> orbit_vm::VmError {
    error::failure("'__tostring' must return a string")
}
