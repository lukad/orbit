use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

pub(crate) const FUNCTION: &str = "type";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let Some(value) = context.argument(0) else {
        return Err(error::missing_value(FUNCTION, 1));
    };

    if value.is_float() {
        Ok(context.return_values([context.string("float")]))
    } else if value.is_integer() {
        Ok(context.return_values([context.string("integer")]))
    } else {
        Ok(context.return_values([context.nil()]))
    }
}
