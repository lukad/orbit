use orbit_vm::{
    ComparisonOp, LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmResult,
};

use crate::error;

const FUNCTION_NAME: &str = "max";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume { token } => resume(context, token),
        NativeEvent::ResumeError { .. } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
    }
}

fn start(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let max = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;
    compare_or_return(context, max, 1)
}

fn resume(context: &mut NativeContext<'_>, token: NativeToken) -> VmResult<NativeAction> {
    let candidate_index = usize::try_from(token.get()).map_err(|_| {
        error::failure(format!(
            "invalid continuation token {} in '{FUNCTION_NAME}'",
            token.get(),
        ))
    })?;

    if candidate_index == 0 || candidate_index >= context.argument_count() {
        return Err(error::failure(format!(
            "invalid continuation token {} in '{FUNCTION_NAME}'",
            token.get(),
        )));
    }

    let previous_maximum = context
        .continuation_value(0)
        .ok_or_else(|| error::failure("missing maximum value in native continuation"))?;

    let candidate = context
        .argument(candidate_index)
        .expect("validated candidate index must exist");

    let replace_maximum = context
        .resume_value(0)
        .and_then(|value| value.as_boolean())
        .expect("comparison action must resume with a boolean");

    let maximum = if replace_maximum {
        candidate
    } else {
        previous_maximum
    };

    compare_or_return(context, maximum, candidate_index + 1)
}

fn compare_or_return<'context>(
    context: &NativeContext<'context>,
    maximum: LocalValue<'context>,
    candidate_index: usize,
) -> VmResult<NativeAction> {
    let Some(candidate) = context.argument(candidate_index) else {
        return Ok(context.return_values([maximum]));
    };

    let token = u64::try_from(candidate_index)
        .map_err(|_| error::failure("too many arguments to 'max'"))?;

    Ok(context.compare_with_continuation(
        ComparisonOp::LessThan,
        maximum.clone(),
        candidate,
        [maximum],
        NativeToken::new(token),
    ))
}
