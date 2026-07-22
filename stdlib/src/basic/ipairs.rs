use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "ipairs";
const ITERATOR_NAME: &str = "ipairs iterator";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let iterator = context
        .capture(0)
        .expect("ipairs is installed with its iterator as capture 0");

    let initial_control = context.integer(0);

    Ok(context.return_values([iterator, value, initial_control]))
}

pub(crate) fn iterator(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let state = context
        .argument(0)
        .ok_or_else(|| error::missing_value(ITERATOR_NAME, 1))?;

    let control = context
        .argument(1)
        .ok_or_else(|| error::type_error(ITERATOR_NAME, 2, "number", None))?;

    let Some(control) = control.to_integer() else {
        return Err(error::type_error(
            ITERATOR_NAME,
            2,
            "number",
            Some(control.type_name()),
        ));
    };

    let index = control.wrapping_add(1);
    let key = context.integer(index);
    let value = context.raw_get(&state, &key)?;

    if value.is_nil() {
        return Ok(context.return_values([value]));
    }

    Ok(context.return_values([key, value]))
}
