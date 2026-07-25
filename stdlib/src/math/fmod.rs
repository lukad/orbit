use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

pub(crate) const FUNCTION: &str = "fmod";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let dividend = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION, 1, "number", None))?;

    let dividend = dividend
        .to_number()
        .ok_or_else(|| error::type_error(FUNCTION, 1, "number", Some(dividend.type_name())))?;

    let divisor = context
        .argument(1)
        .ok_or_else(|| error::type_error(FUNCTION, 2, "number", None))?;

    let divisor = divisor
        .to_number()
        .ok_or_else(|| error::type_error(FUNCTION, 2, "number", Some(divisor.type_name())))?;

    match (dividend.as_integer(), divisor.as_integer()) {
        (Some(_), Some(0)) => Err(error::argument_error(FUNCTION, 2, "zero")),
        (Some(dividend), Some(divisor)) => {
            let result = context.integer(dividend.wrapping_rem(divisor));
            Ok(context.return_values([result]))
        }
        _ => {
            let dividend = dividend.to_float().expect("value is a number");
            let divisor = divisor.to_float().expect("value is a number");
            let result = context.float(dividend % divisor);
            Ok(context.return_values([result]))
        }
    }
}
