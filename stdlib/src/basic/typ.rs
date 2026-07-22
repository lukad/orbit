use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "type";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let type_name = context.string(value.type_name());

    Ok(context.return_values([type_name]))
}
