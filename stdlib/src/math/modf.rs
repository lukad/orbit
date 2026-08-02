use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

pub(crate) const FUNCTION: &str = "modf";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION, 1, "number", None))?;

    if let Some(integer) = value.as_integer() {
        return Ok(context.return_values([context.integer(integer), context.float(0.0)]));
    }

    let number = value
        .to_float()
        .ok_or_else(|| error::type_error(FUNCTION, 1, "number", Some(value.type_name())))?;

    let truncated = number.trunc();

    const LIMIT: f64 = 9_223_372_036_854_775_808.0;

    let integral = if truncated.is_finite() && (-LIMIT..LIMIT).contains(&truncated) {
        context.integer(truncated as i64)
    } else {
        context.float(truncated)
    };

    let fractional = if number == truncated {
        context.float(0.0)
    } else {
        context.float(number - truncated)
    };

    Ok(context.return_values([integral, fractional]))
}
