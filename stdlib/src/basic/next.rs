use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::error;

const FUNCTION_NAME: &str = "next";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let table = context
        .argument(0)
        .ok_or_else(|| error::type_error(FUNCTION_NAME, 1, "table", None))?;

    if table.type_name() != "table" {
        return Err(error::type_error(
            FUNCTION_NAME,
            1,
            "table",
            Some(table.type_name()),
        ));
    }

    let previous = match context.argument(1) {
        Some(previous) => previous,
        None => context.nil(),
    };

    match context.next(&table, &previous)? {
        Some((key, value)) => Ok(context.return_values([key, value])),
        None => {
            let nil = context.nil();
            Ok(context.return_values([nil]))
        }
    }
}
