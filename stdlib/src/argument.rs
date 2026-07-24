use orbit_vm::{LocalValue, VmResult};

use crate::error;

pub(crate) fn check_integer(
    value: &LocalValue<'_>,
    function: &'static str,
    argument: usize,
) -> VmResult<i64> {
    match value.to_integer() {
        Some(value) => Ok(value),
        None if value.is_number() => Err(error::number_has_no_integer_representation(
            function, argument,
        )),
        None => Err(error::type_error(
            function,
            argument,
            "number",
            Some(value.type_name()),
        )),
    }
}
