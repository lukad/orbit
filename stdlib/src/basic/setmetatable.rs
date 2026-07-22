use orbit_vm::{LocalValue, NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "setmetatable";
const PROTECTION_FIELD: &[u8] = b"__metatable";

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

    let metatable = context
        .argument(1)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 2, "nil or table", None))?;

    let metatable = requested_metatable(&metatable)?;

    if let Some(current) = context.get_metatable(&table)? {
        let protection_key = context.string(PROTECTION_FIELD);
        let protection = context.raw_get(&current, &protection_key)?;

        if !protection.is_nil() {
            return Err(error::failure("cannot change a protected metatable"));
        }
    }

    context.set_metatable(&table, metatable)?;

    Ok(context.return_values([table]))
}

fn requested_metatable<'value, 'context>(
    value: &'value LocalValue<'context>,
) -> VmResult<Option<&'value LocalValue<'context>>> {
    if value.is_nil() {
        return Ok(None);
    }

    if value.type_name() != "table" {
        return Err(error::type_error(
            FUNCTION_NAME,
            2,
            "nil or table",
            Some(value.type_name()),
        ));
    }

    Ok(Some(value))
}
