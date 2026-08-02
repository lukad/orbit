use std::f64;

use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::argument::{check_float, required_number};

pub(crate) const FUNCTION: &str = "log";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let number = required_number(context, FUNCTION, 0)?;

    let base = match context.argument(1) {
        Some(value) if value.is_nil() => f64::consts::E,
        Some(value) => check_float(&value, FUNCTION, 2)?,
        None => f64::consts::E,
    };

    Ok(context.return_values([context.float(number.log(base))]))
}
