use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

pub const FUNCTION_NAME: &str = "abs";

pub fn callback(context: &mut NativeContext) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "number", None))?;

    let result = if let Some(integer) = value.as_integer() {
        context.integer(integer.wrapping_abs())
    } else {
        let number = value.to_float().ok_or_else(|| {
            error::type_error(FUNCTION_NAME, 1, "number", Some(value.type_name()))
        })?;

        context.float(number.abs())
    };

    Ok(context.return_values([result]))
}
