use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "tointeger";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let result = match value.to_integer() {
        Some(integer) => context.integer(integer),
        None => context.nil(),
    };

    Ok(context.return_values([result]))
}
