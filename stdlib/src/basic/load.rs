use orbit_vm::{
    LoadSource, LocalValue, NativeAction, NativeContext, NativeEvent, NativeToken, VmError,
    VmErrorKind, VmResult,
};

use crate::error;

const FUNCTION_NAME: &str = "load";
const READER_CALL: NativeToken = NativeToken::new(1);
const DEFAULT_READER_NAME: &[u8] = b"=(load)";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume { token: READER_CALL } => resume(context),
        NativeEvent::ResumeError { token: READER_CALL } => {
            let error = context
                .resume_error()
                .expect("ResumeError must contain an error");

            Ok(return_failure(context, error.kind.to_string()))
        }
        NativeEvent::Resume { token } | NativeEvent::ResumeError { token } => {
            Err(error::failure(format!(
                "invalid continuation token {} in '{FUNCTION_NAME}'",
                token.get()
            )))
        }
    }
}

fn start(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let chunk = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let mode = optional_string(context, 2, b"bt")?;
    validate_text_mode(context, &mode)?;

    if chunk.type_name() == "string" || chunk.type_name() == "number" {
        let source = if let Some(string) = chunk.as_string() {
            string.as_bytes().to_vec()
        } else {
            context
                .default_tostring(&chunk, None)
                .as_string()
                .unwrap()
                .as_bytes()
                .to_vec()
        };

        let name = optional_string(context, 1, &source)?;
        return compile(context, &name, &source);
    }

    if chunk.type_name() != "function" {
        return Err(error::type_error(
            FUNCTION_NAME,
            1,
            "function",
            Some(chunk.type_name()),
        ));
    }

    let name = optional_string(context, 1, DEFAULT_READER_NAME)?;
    let accumulator = context.string([]);

    Ok(context.call_with_continuation(chunk, [], [context.string(name), accumulator], READER_CALL))
}

fn resume(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let name = context
        .continuation_value(0)
        .expect("reader continuation stores the chunk name");

    let accumulated = context
        .continuation_value(1)
        .expect("reader continuation stores source");

    let piece = context.resume_value(0);

    let Some(piece) = piece else {
        return compile_values(context, name, accumulated);
    };

    if piece.is_nil() {
        return compile_values(context, name, accumulated);
    }

    let piece = if let Some(string) = piece.as_string() {
        string.as_bytes().to_vec()
    } else if piece.type_name() == "number" {
        context
            .default_tostring(&piece, None)
            .as_string()
            .unwrap()
            .as_bytes()
            .to_vec()
    } else {
        return Ok(return_failure(
            context,
            "reader function must return a string",
        ));
    };

    if piece.is_empty() {
        return compile_values(context, name, accumulated);
    }

    let old = accumulated.as_string().unwrap().as_bytes();
    let mut combined = Vec::with_capacity(old.len() + piece.len());
    combined.extend_from_slice(old);
    combined.extend_from_slice(&piece);

    let reader = context.argument(0).unwrap();

    Ok(context.call_with_continuation(reader, [], [name, context.string(combined)], READER_CALL))
}

fn optional_string(context: &NativeContext<'_>, index: usize, default: &[u8]) -> VmResult<Vec<u8>> {
    let Some(value) = context.argument(index) else {
        return Ok(default.to_vec());
    };

    if value.is_nil() {
        return Ok(default.to_vec());
    }

    if let Some(string) = value.as_string() {
        return Ok(string.as_bytes().to_vec());
    }

    if value.type_name() == "number" {
        return Ok(context
            .default_tostring(&value, None)
            .as_string()
            .expect("numeric conversion returns a string")
            .as_bytes()
            .to_vec());
    }

    Err(error::type_error(
        FUNCTION_NAME,
        index + 1,
        "string",
        Some(value.type_name()),
    ))
}

fn compile_values(
    context: &mut NativeContext<'_>,
    name: LocalValue<'_>,
    source: LocalValue<'_>,
) -> VmResult<NativeAction> {
    compile(
        context,
        name.as_string().unwrap().as_bytes(),
        source.as_string().unwrap().as_bytes(),
    )
}

fn compile(context: &mut NativeContext<'_>, name: &[u8], source: &[u8]) -> VmResult<NativeAction> {
    let environment = if context.argument_count() >= 4 {
        Some(context.argument(3).unwrap())
    } else {
        None
    };

    match context.load_source(LoadSource::Buffer { name, source }, environment) {
        Ok(function) => Ok(context.return_values([function])),
        Err(
            error @ VmError {
                kind: VmErrorKind::LoadFailure(_),
                ..
            },
        ) => Ok(return_failure(context, error.kind.to_string())),

        Err(error) => Err(error),
    }
}

fn validate_text_mode(_: &NativeContext<'_>, mode: &[u8]) -> VmResult<()> {
    if mode == b"t" || mode == b"bt" {
        return Ok(());
    }

    if mode == b"b" {
        return Err(error::failure("binary chunks are not supported"));
    }

    Err(error::failure(format!(
        "invalid mode '{}'",
        String::from_utf8_lossy(mode)
    )))
}

fn return_failure(context: &NativeContext<'_>, message: impl AsRef<[u8]>) -> NativeAction {
    context.return_values([context.nil(), context.string(message)])
}
