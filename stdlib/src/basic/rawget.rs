use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "rawget";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let table = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "table", None))?;

    if table.type_name() != "table" {
        return Err(error::type_error(
            FUNCTION_NAME,
            1,
            "table",
            Some(table.type_name()),
        ));
    }

    let key = context
        .argument(1)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 2))?;

    let value = context.raw_get(&table, &key)?;

    Ok(context.return_values([value]))
}
