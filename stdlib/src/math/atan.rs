use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::argument::required_number;

pub(crate) const FUNCTION: &str = "atan";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let x = required_number(context, FUNCTION, 0)?;

    let y = match context.argument(1) {
        None => 1.0,
        Some(value) if value.is_nil() => 1.0,
        Some(value) => value.to_float().ok_or_else(|| {
            crate::error::type_error(FUNCTION, 2, "number", Some(value.type_name()))
        })?,
    };

    Ok(context.return_values([context.float(x.atan2(y))]))
}
