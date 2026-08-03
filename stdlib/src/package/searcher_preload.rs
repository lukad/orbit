use orbit_vm::{NativeAction, NativeContext, NativeEvent, NativeToken, VmError, VmResult};

use crate::{argument::required_string, error};

const FUNCTION_NAME: &str = "package preload searcher";
const PRELOAD_LOOKUP: NativeToken = NativeToken::new(1);

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume {
            token: PRELOAD_LOOKUP,
        } => finish_lookup(context),
        NativeEvent::ResumeError {
            token: PRELOAD_LOOKUP,
        } => Err(context
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

    let preload = context
        .capture(0)
        .expect("preload searcher captures package.preload");

    Ok(context.get_with_continuation(preload, name.clone(), [name], PRELOAD_LOOKUP))
}

fn finish_lookup(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = context
        .continuation_value(0)
        .ok_or_else(|| error::failure("missing preload searcher continuation value"))?;
    let loader = context.resume_value(0).unwrap_or_else(|| context.nil());

    if loader.is_nil() {
        let mut message = b"no field package.preload['".to_vec();

        message.extend_from_slice(
            name.as_string()
                .expect("checked module name is a string")
                .as_bytes(),
        );

        message.extend_from_slice(b"']");

        return Ok(context.return_values([context.string(message)]));
    }

    Ok(context.return_values([loader, context.string(":preload:")]))
}

fn invalid_token(token: NativeToken) -> VmError {
    error::failure(format!(
        "invalid continuation token {} in '{FUNCTION_NAME}'",
        token.get(),
    ))
}
