use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "assert";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    if !value.is_truthy() {
        todo!()
    }

    Ok(context.return_values([]))
}
