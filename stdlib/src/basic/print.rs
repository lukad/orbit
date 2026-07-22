use std::io::{self, Write};

use orbit_vm::{
    NativeAction, NativeContext, NativeEvent, NativeToken, VmError, VmErrorKind, VmResult,
};

use crate::error;

const FUNCTION_NAME: &str = "print";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => start(context),
        NativeEvent::Resume { token } => resume_after_tostring(context, token),
        NativeEvent::ResumeError { .. } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
    }
}

fn start(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    if context.argument_count() == 0 {
        write_newline()?;
        return Ok(context.return_values([]));
    }

    call_tostring(context, 0)
}

fn resume_after_tostring(
    context: &mut NativeContext<'_>,
    token: NativeToken,
) -> VmResult<NativeAction> {
    let index = usize::try_from(token.get()).map_err(|_| {
        error::failure(format!(
            "invalid continuation token {} in '{FUNCTION_NAME}'",
            token.get(),
        ))
    })?;

    if index >= context.argument_count() {
        return Err(error::failure(format!(
            "invalid continuation token {} in '{FUNCTION_NAME}'",
            token.get(),
        )));
    }

    let result = context
        .resume_value(0)
        .ok_or_else(|| error::failure("'tostring' returned no value while printing"))?;

    let string = result
        .as_string()
        .ok_or_else(|| error::failure("'tostring' must return a string to print"))?;

    write_argument(index, string.as_bytes())?;

    let next = index + 1;

    if next < context.argument_count() {
        call_tostring(context, next)
    } else {
        write_newline()?;
        Ok(context.return_values([]))
    }
}

fn call_tostring(context: &mut NativeContext<'_>, index: usize) -> VmResult<NativeAction> {
    let tostring = context
        .capture(0)
        .expect("print must capture the built-in tostring function");

    let argument = context
        .argument(index)
        .expect("print argument index must be valid");

    let token =
        u64::try_from(index).map_err(|_| error::failure("too many arguments to 'print'"))?;

    Ok(context.call(tostring, [argument], NativeToken::new(token)))
}

fn write_argument(index: usize, bytes: &[u8]) -> VmResult<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();

    if index != 0 {
        output.write_all(b"\t").map_err(native_io_error)?;
    }

    output.write_all(bytes).map_err(native_io_error)?;
    output.flush().map_err(native_io_error)?;

    Ok(())
}

fn write_newline() -> VmResult<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();

    output.write_all(b"\n").map_err(native_io_error)?;
    output.flush().map_err(native_io_error)?;

    Ok(())
}

fn native_io_error(error: io::Error) -> VmError {
    VmErrorKind::NativeFunctionFailure {
        message: error.to_string().into_boxed_str(),
    }
    .into()
}
