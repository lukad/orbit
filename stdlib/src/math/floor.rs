use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

pub(crate) const FUNCTION: &str = "floor";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION, 1, "number", None))?;

    let value = value
        .to_number()
        .ok_or_else(|| error::type_error(FUNCTION, 1, "number", Some(value.type_name())))?;

    if value.is_integer() {
        Ok(context.return_values([value]))
    } else {
        let float = value
            .as_float()
            .expect("number is not int and must be float");
        let floored = float.floor();

        const LIMIT: f64 = 9_223_372_036_854_775_808.0;

        if floored.is_finite() && (-LIMIT..LIMIT).contains(&floored) {
            Ok(context.return_values([context.integer(floored as i64)]))
        } else {
            Ok(context.return_values([context.float(floored)]))
        }
    }
}
