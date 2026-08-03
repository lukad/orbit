use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::argument::required_string;

pub const FUNCTION_NAME: &str = "reverse";

pub fn callback(context: &mut NativeContext) -> VmResult<NativeAction> {
    let mut string = required_string(context, FUNCTION_NAME, 0)?
        .as_string()
        .expect("required string is a string")
        .as_bytes()
        .to_vec();
    string.reverse();
    Ok(context.return_values([context.string(string)]))
}
