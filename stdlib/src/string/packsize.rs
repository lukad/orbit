use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

use super::packing;

const FUNCTION_NAME: &str = "packsize";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let format = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "string", None))?;

    let format = format
        .as_string()
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "string", Some(format.type_name())))?;

    let size = packing::pack_size(format.as_bytes()).map_err(|error| {
        let message = error.message();
        if error.is_argument_error() {
            error::failure(format!("bad argument #1 to '{FUNCTION_NAME}' ({message})"))
        } else {
            error::failure(message)
        }
    })?;

    let size = context.integer(i64::try_from(size).expect("type size fits in i64"));

    Ok(context.return_values([size]))
}
