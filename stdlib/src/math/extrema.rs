use orbit_vm::{
    ComparisonOp, LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmResult,
};

use crate::error;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Minimum,
    Maximum,
}

impl Kind {
    fn function_name(self) -> &'static str {
        match self {
            Self::Minimum => "min",
            Self::Maximum => "max",
        }
    }

    fn comparison_operands<'context>(
        self,
        current: &LocalValue<'context>,
        candidate: LocalValue<'context>,
    ) -> (LocalValue<'context>, LocalValue<'context>) {
        match self {
            Self::Minimum => (candidate, current.clone()),
            Self::Maximum => (current.clone(), candidate),
        }
    }
}

pub(super) fn callback(context: &mut NativeContext<'_>, kind: Kind) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context, kind),
        NativeEvent::Resume { token } => resume(context, kind, token),
        NativeEvent::ResumeError { .. } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
    }
}

fn start(context: &mut NativeContext<'_>, kind: Kind) -> VmResult<NativeAction> {
    let current = context
        .argument(0)
        .ok_or_else(|| error::missing_value(kind.function_name(), 1))?;

    compare_or_return(context, kind, current, 1)
}

fn resume(
    context: &mut NativeContext<'_>,
    kind: Kind,
    token: NativeToken,
) -> VmResult<NativeAction> {
    let function_name = kind.function_name();
    let candidate_index = usize::try_from(token.get()).map_err(|_| {
        error::failure(format!(
            "invalid continuation token {} in '{function_name}'",
            token.get(),
        ))
    })?;

    if candidate_index == 0 || candidate_index >= context.argument_count() {
        return Err(error::failure(format!(
            "invalid continuation token {} in '{function_name}'",
            token.get(),
        )));
    }

    let previous = context
        .continuation_value(0)
        .ok_or_else(|| error::failure("missing extrema value in native continuation"))?;

    let candidate = context
        .argument(candidate_index)
        .expect("validated candidate index must exist");

    let replace = context
        .resume_value(0)
        .and_then(|value| value.as_boolean())
        .expect("comparison action must resume with a boolean");

    let current = if replace { candidate } else { previous };

    compare_or_return(context, kind, current, candidate_index + 1)
}

fn compare_or_return<'context>(
    context: &NativeContext<'context>,
    kind: Kind,
    current: LocalValue<'context>,
    candidate_index: usize,
) -> VmResult<NativeAction> {
    let Some(candidate) = context.argument(candidate_index) else {
        return Ok(context.return_values([current]));
    };

    let token = u64::try_from(candidate_index)
        .map_err(|_| error::failure(format!("too many arguments to '{}'", kind.function_name())))?;

    let (left, right) = kind.comparison_operands(&current, candidate);

    Ok(context.compare_with_continuation(
        ComparisonOp::LessThan,
        left,
        right,
        [current],
        NativeToken::new(token),
    ))
}
