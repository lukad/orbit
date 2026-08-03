use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::argument::required_string;

pub const FUNCTION_NAME: &str = "len";

pub fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let string = required_string(context, FUNCTION_NAME, 0)?;
    let length = string.len();
    Ok(context.return_values([context.integer(length as i64)]))
}
