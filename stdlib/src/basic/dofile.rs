use orbit_vm::{NativeAction, NativeContext, NativeEvent, NativeToken, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "dofile";
const CHUNK_CALL: NativeToken = NativeToken::new(1);

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume { token: CHUNK_CALL } => finish(context),
        NativeEvent::ResumeError { token: CHUNK_CALL } => Err(context
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
    let chunk = match context.argument(0) {
        None => context.load_stdin()?,
        Some(filename) if filename.is_nil() => context.load_stdin()?,
        Some(filename) => {
            let filename = match filename.type_name() {
                "string" => filename,
                "number" => context.default_tostring(&filename, None),
                _ => {
                    return Err(error::type_error(
                        FUNCTION_NAME,
                        1,
                        "string",
                        Some(filename.type_name()),
                    ));
                }
            };

            let bytes = filename
                .as_string()
                .expect("string and converted number must produce a string")
                .as_bytes()
                .to_vec();

            context.load_file(bytes)?
        }
    };

    Ok(context.call(chunk, [], CHUNK_CALL))
}

fn finish(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let results = (0..context.resume_value_count())
        .filter_map(|index| context.resume_value(index))
        .collect::<Vec<_>>();

    Ok(context.return_values(results))
}
