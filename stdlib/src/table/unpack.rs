use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{argument, error};

const FUNCTION_NAME: &str = "unpack";
const MAX_UNPACK_RESULTS: usize = 1_000_000;

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let table = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "table", None))?;

    let start = optional_integer(context, 1, 1)?;
    let end = match context.argument(2) {
        None => context.raw_len(&table)?,
        Some(value) if value.is_nil() => context.raw_len(&table)?,
        Some(value) => argument::check_integer(&value, FUNCTION_NAME, 3)?,
    };

    if start > end {
        return Ok(context.return_values([]));
    }

    let count = i128::from(end) - i128::from(start) + 1;
    let count = usize::try_from(count)
        .ok()
        .filter(|count| *count <= MAX_UNPACK_RESULTS)
        .ok_or_else(|| error::failure("too many results to unpack"))?;

    let mut results = Vec::new();
    results
        .try_reserve_exact(count)
        .map_err(|_| error::failure("too many results to unpack"))?;

    let mut index = start;
    loop {
        let key = context.integer(index);
        results.push(context.raw_get(&table, &key)?);

        if index == end {
            break;
        }
        index += 1;
    }

    Ok(context.return_values(results))
}

fn optional_integer(context: &NativeContext<'_>, index: usize, default: i64) -> VmResult<i64> {
    match context.argument(index) {
        None => Ok(default),
        Some(value) if value.is_nil() => Ok(default),
        Some(value) => argument::check_integer(&value, FUNCTION_NAME, index + 1),
    }
}
