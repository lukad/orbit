use orbit_vm::{NativeAction, NativeContext, NativeEvent, NativeToken, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "pairs";
const PAIRS_METAMETHOD: &[u8] = b"__pairs";
const PAIRS_CALL: NativeToken = NativeToken::new(1);

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume { token: PAIRS_CALL } => finish_metamethod(context),
        NativeEvent::ResumeError { token: PAIRS_CALL } => Err(context
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

    if let Some(metatable) = context.get_metatable(&value)? {
        let key = context.string(PAIRS_METAMETHOD);
        let metamethod = context.raw_get(&metatable, &key)?;

        if !metamethod.is_nil() {
            return Ok(context.call(metamethod, [value], PAIRS_CALL));
        }
    }

    let next = context
        .capture(0)
        .expect("pairs must capture the built-in next function");

    let nil = context.nil();

    Ok(context.return_values([next, value, nil]))
}

fn finish_metamethod(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let iterator = context.resume_value(0).unwrap_or_else(|| context.nil());
    let state = context.resume_value(1).unwrap_or_else(|| context.nil());
    let control = context.resume_value(2).unwrap_or_else(|| context.nil());

    Ok(context.return_values([iterator, state, control]))
}
