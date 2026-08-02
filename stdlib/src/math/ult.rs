use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::argument::required_integer;

pub const FUNCTION_NAME: &str = "ult";

pub fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let m = required_integer(context, FUNCTION_NAME, 0)?;
    let n = required_integer(context, FUNCTION_NAME, 1)?;
    let result = (m as u64) < (n as u64);
    Ok(context.return_values([context.boolean(result)]))
}
