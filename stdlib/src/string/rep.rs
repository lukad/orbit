use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{
    argument::{required_integer, required_string},
    error,
};

pub(crate) const FUNCTION: &str = "rep";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let string = required_string(context, FUNCTION, 0)?;
    let bytes = string.as_bytes();

    let n = required_integer(context, FUNCTION, 1)?.max(0) as usize;

    if n == 0 {
        return Ok(context.return_values([context.string([])]));
    }

    let separator = match context.argument(2) {
        Some(value) if !value.is_nil() => Some(required_string(context, FUNCTION, 2)?),
        _ => None,
    };
    let separator = separator
        .as_ref()
        .map(|value| value.as_bytes())
        .unwrap_or(&[]);

    let total = bytes
        .len()
        .checked_mul(n)
        .and_then(|body| {
            separator
                .len()
                .checked_mul(n - 1)
                .and_then(|separators| body.checked_add(separators))
        })
        .filter(|&total| total <= isize::MAX as usize)
        .ok_or_else(|| error::failure("resulting string too large"))?;

    let mut result = Vec::with_capacity(total);

    result.extend_from_slice(bytes);

    for _ in 1..n {
        result.extend_from_slice(separator);
        result.extend_from_slice(bytes);
    }

    Ok(context.return_values([context.string(result)]))
}
