use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::argument::required_number;

pub(crate) const FUNCTION: &str = "deg";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let number = required_number(context, FUNCTION, 0)?;
    Ok(context.return_values([context.float(number.to_degrees())]))
}
