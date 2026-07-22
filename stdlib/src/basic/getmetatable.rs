use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "getmetatable";
const PROTECTION_FIELD: &[u8] = b"__metatable";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let Some(metatable) = context.get_metatable(&value)? else {
        let nil = context.nil();
        return Ok(context.return_values([nil]));
    };

    let protection_key = context.string(PROTECTION_FIELD);
    let protection = context.raw_get(&metatable, &protection_key)?;

    if protection.is_nil() {
        Ok(context.return_values([metatable]))
    } else {
        Ok(context.return_values([protection]))
    }
}
