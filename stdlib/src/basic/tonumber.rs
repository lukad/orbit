use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{argument, error};

const FUNCTION_NAME: &str = "tonumber";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::missing_value(FUNCTION_NAME, 1))?;

    let Some(base) = context.argument(1).filter(|base| !base.is_nil()) else {
        let number = value.to_number().unwrap_or_default();
        return Ok(context.return_values([number]));
    };

    let base = argument::check_integer(&base, FUNCTION_NAME, 2)?;

    if value.as_string().is_none() {
        return Err(error::type_error(
            FUNCTION_NAME,
            1,
            "string",
            Some(value.type_name()),
        ));
    }

    let base = u32::try_from(base)
        .ok()
        .filter(|base| (2..=36).contains(base))
        .ok_or_else(|| error::argument_error(FUNCTION_NAME, 2, "base out of range"))?;

    let number = value
        .to_integer_with_base(base)
        .map(|number| context.integer(number))
        .unwrap_or_default();

    Ok(context.return_values([number]))
}
