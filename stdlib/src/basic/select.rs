use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "select";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let value = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "number", None))?;

    if let Some(s) = value.as_string()
        && s.as_bytes() == b"#"
    {
        let num_extra_args = context.integer(context.argument_count() as i64 - 1);
        return Ok(context.return_values([num_extra_args]));
    }

    let offset = match value.to_integer() {
        Some(offset) => offset,
        None if value.type_name() == "number" => {
            return Err(error::number_has_no_integer_representation(
                FUNCTION_NAME,
                1,
            ));
        }
        None => {
            return Err(error::type_error(
                FUNCTION_NAME,
                1,
                "number",
                Some(value.type_name()),
            ));
        }
    };

    if offset == 0 {
        return Err(error::index_out_of_range(FUNCTION_NAME, 1));
    }

    let rest_count = context.argument_count() - 1;

    let start = if offset.is_positive() {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if start > rest_count {
            return Ok(context.return_values([]));
        }
        start
    } else {
        let magnitude = offset.unsigned_abs();
        if magnitude > rest_count as u64 {
            return Err(error::index_out_of_range(FUNCTION_NAME, 1));
        }
        rest_count + 1 - magnitude as usize
    };

    let mut returned_args = Vec::with_capacity(context.argument_count() - start);

    for i in start..context.argument_count() {
        returned_args.push(context.argument(i).unwrap());
    }

    Ok(context.return_values(returned_args))
}
