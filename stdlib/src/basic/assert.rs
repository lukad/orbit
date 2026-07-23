use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "assert";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    if !value.is_truthy() {
        return Err(error::assertion_failed());
    }

    let mut args = Vec::with_capacity(context.argument_count());
    for i in 0..context.argument_count() {
        args.push(context.argument(i).unwrap());
    }

    Ok(context.return_values(args))
}
