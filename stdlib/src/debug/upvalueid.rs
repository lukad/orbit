use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{argument, error};

pub(crate) const FUNCTION: &[u8] = b"upvalueid";
pub(crate) const FUNCTION_NAME: &str = "debug.upvalueid";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let index = argument::required_integer(context, FUNCTION_NAME, 1)?;

    let function = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "function", None))?;

    if function.type_name() != "function" {
        return Err(error::type_error(
            FUNCTION_NAME,
            1,
            "function",
            Some(function.type_name()),
        ));
    }

    let Some(index) = index
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
    else {
        return Ok(context.return_values([context.nil()]));
    };

    let identity = context
        .function_upvalue_id(&function, index)?
        .unwrap_or_else(|| context.nil());

    Ok(context.return_values([identity]))
}
