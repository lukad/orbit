use std::mem::size_of;

use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{
    argument::{required_integer, required_number, required_string},
    error,
    string::packing::{
        FormatParser, ItemKind, check_integer_range, double_bytes, float_bytes, integer_bytes,
    },
};

pub(crate) const FUNCTION_NAME: &str = "pack";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let format_value = required_string(context, FUNCTION_NAME, 0)?;
    let format = format_value
        .as_string()
        .expect("required_string always returns a string")
        .as_bytes();

    let mut parser = FormatParser::new(format);
    let mut output = Vec::new();
    let mut argument = 1;

    while let Some(item) = parser.next_item(output.len()).map_err(|format_error| {
        let message = format_error.message();
        if format_error.is_argument_error() {
            error::argument_error(FUNCTION_NAME, 1, message)
        } else {
            error::failure(message)
        }
    })? {
        output.resize(output.len() + item.padding, 0);

        match item.kind {
            ItemKind::SignedInteger | ItemKind::UnsignedInteger => {
                let value = required_integer(context, FUNCTION_NAME, argument)?;
                let signed = item.kind == ItemKind::SignedInteger;

                check_integer_range(value, item.size, signed).map_err(|message| {
                    error::argument_error(FUNCTION_NAME, argument + 1, message)
                })?;

                output.extend(integer_bytes(value, item.size, item.endian, signed));
                argument += 1;
            }
            ItemKind::Float => {
                let value = required_number(context, FUNCTION_NAME, argument)? as f32;
                output.extend(float_bytes(value, item.endian));
                argument += 1;
            }
            ItemKind::Double | ItemKind::LuaNumber => {
                let value = required_number(context, FUNCTION_NAME, argument)?;
                output.extend(double_bytes(value, item.endian));
                argument += 1;
            }
            ItemKind::FixedString => {
                let value = required_string(context, FUNCTION_NAME, argument)?;
                let bytes = value
                    .as_string()
                    .expect("required_string always returns a string")
                    .as_bytes();

                if bytes.len() > item.size {
                    return Err(error::argument_error(
                        FUNCTION_NAME,
                        argument + 1,
                        "string longer than given size",
                    ));
                }

                output.extend_from_slice(bytes);
                output.resize(output.len() + item.size - bytes.len(), 0);
                argument += 1;
            }
            ItemKind::LengthString => {
                let value = required_string(context, FUNCTION_NAME, argument)?;
                let bytes = value
                    .as_string()
                    .expect("required_string always returns a string")
                    .as_bytes();

                if item.size < size_of::<usize>() && bytes.len() >= (1usize << (item.size * 8)) {
                    return Err(error::argument_error(
                        FUNCTION_NAME,
                        argument + 1,
                        "string length does not fit in given size",
                    ));
                }

                output.extend(integer_bytes(
                    bytes.len() as i64,
                    item.size,
                    item.endian,
                    false,
                ));
                output.extend_from_slice(bytes);
                argument += 1;
            }
            ItemKind::ZeroString => {
                let value = required_string(context, FUNCTION_NAME, argument)?;
                let bytes = value
                    .as_string()
                    .expect("required_string always returns a string")
                    .as_bytes();

                if bytes.contains(&0) {
                    return Err(error::argument_error(
                        FUNCTION_NAME,
                        argument + 1,
                        "string contains zeros",
                    ));
                }

                output.extend_from_slice(bytes);
                output.push(0);
                argument += 1;
            }

            ItemKind::Padding => output.push(0),

            ItemKind::AlignmentPadding => {}
        }
    }

    Ok(context.return_values([context.string(output)]))
}
