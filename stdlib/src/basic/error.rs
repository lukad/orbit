use orbit_vm::{NativeAction, NativeContext, VmError, VmResult};

use crate::argument;

pub(crate) const FUNCTION: &str = "error";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let object = context.argument(0).unwrap_or_default();

    let level = match context.argument(1) {
        None => 1,
        Some(value) if value.is_nil() => 1,
        Some(value) => argument::check_integer(&value, FUNCTION, 2)?,
    };

    let object = context.export(&object)?;

    Err(VmError::raised(object, level))
}
