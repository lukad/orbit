use orbit_vm::{NativeAction, NativeContext, NativeEvent, NativeToken, VmResult};

use crate::error;

const CALL: NativeToken = NativeToken::new(1);

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            let callee = context.argument(0).unwrap_or_default();
            let arguments =
                (1..context.argument_count()).map(|index| context.argument(index).unwrap());
            Ok(context.call(callee, arguments, CALL))
        }
        NativeEvent::Resume { token: CALL } => {
            let results = std::iter::once(context.boolean(true)).chain(
                (0..context.resume_value_count()).map(|index| context.resume_value(index).unwrap()),
            );
            Ok(context.return_values(results))
        }
        NativeEvent::ResumeError { token: CALL } => {
            let (object, fallback) = {
                let failure = context
                    .resume_error()
                    .expect("ResumeError must contain an error");

                (failure.object().cloned(), failure.kind.to_string())
            };

            let object = match object {
                Some(object) => context.import(object)?,
                None => context.string(fallback),
            };

            Ok(context.return_values([context.boolean(false), object]))
        }
        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => Err(error::failure(
            format!("invalid continuation token {} in 'pcall'", token.get()),
        )),
    }
}
