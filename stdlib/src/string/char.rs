use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::argument::check_integer;

pub const FUNCTION_NAME: &str = "char";

pub fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let mut string = vec![];

    for i in 0..context.argument_count() {
        let Some(value) = context.argument(i) else {
            unreachable!("less arguments than argument_count");
        };

        let int = check_integer(&value, FUNCTION_NAME, i + 1)?;

        match u8::try_from(int) {
            Ok(char) => string.push(char),
            Err(_) => {
                return Err(crate::error::argument_error(
                    FUNCTION_NAME,
                    i + 1,
                    "value out of range",
                ));
            }
        }
    }

    Ok(context.return_values([context.string(string)]))
}
