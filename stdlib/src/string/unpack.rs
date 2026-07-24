use orbit_vm::{NativeAction, NativeContext, VmResult};

use crate::{
    argument::{check_integer, required_string},
    error,
    string::packing::{Endian, FormatParser, ItemKind, read_integer},
};

const FUNCTION_NAME: &str = "unpack";

pub(crate) fn callback(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    let format_value = required_string(context, FUNCTION_NAME, 0)?;
    let format = format_value
        .as_string()
        .expect("required_string always returns a string")
        .as_bytes();

    let data_value = required_string(context, FUNCTION_NAME, 1)?;
    let data = data_value
        .as_string()
        .expect("required_string always returns a string")
        .as_bytes();

    let requested_position = match context.argument(2) {
        None => 1,
        Some(value) if value.is_nil() => 1,
        Some(value) => check_integer(&value, FUNCTION_NAME, 3)?,
    };

    let mut position = initial_offset(requested_position, data.len())
        .filter(|position| *position <= data.len())
        .ok_or_else(|| error::argument_error(FUNCTION_NAME, 3, "initial position out of string"))?;

    let mut parser = FormatParser::new(format);
    let mut results = Vec::new();

    while let Some(item) = parser.next_item(position).map_err(|format_error| {
        let message = format_error.message();
        if format_error.is_argument_error() {
            error::argument_error(FUNCTION_NAME, 1, message)
        } else {
            error::failure(message)
        }
    })? {
        let required = item
            .padding
            .checked_add(item.size)
            .filter(|required| *required <= data.len() - position)
            .ok_or_else(|| error::argument_error(FUNCTION_NAME, 2, "data string too short"))?;

        debug_assert!(required <= data.len() - position);
        position += item.padding;
        let field = &data[position..position + item.size];

        match item.kind {
            ItemKind::SignedInteger | ItemKind::UnsignedInteger => {
                let signed = item.kind == ItemKind::SignedInteger;
                let value = read_integer(field, item.endian, signed)
                    .map_err(|format_error| error::failure(format_error.message()))?;
                results.push(context.integer(value));
            }
            ItemKind::Float => {
                let bytes: [u8; size_of::<f32>()] =
                    field.try_into().expect("float format size matches f32");
                let value = match item.endian {
                    Endian::Little => f32::from_le_bytes(bytes),
                    Endian::Big => f32::from_be_bytes(bytes),
                };
                results.push(context.float(f64::from(value)));
            }
            ItemKind::Double | ItemKind::LuaNumber => {
                let bytes: [u8; size_of::<f64>()] =
                    field.try_into().expect("double format size matches f64");
                let value = match item.endian {
                    Endian::Little => f64::from_le_bytes(bytes),
                    Endian::Big => f64::from_be_bytes(bytes),
                };
                results.push(context.float(value));
            }
            ItemKind::FixedString => {
                results.push(context.string(field));
            }
            ItemKind::LengthString => {
                let length = read_integer(field, item.endian, false)
                    .map_err(|format_error| error::failure(format_error.message()))?
                    as usize;
                let start = position + item.size;
                let end = start
                    .checked_add(length)
                    .filter(|end| *end <= data.len())
                    .ok_or_else(|| {
                        error::argument_error(FUNCTION_NAME, 2, "data string too short")
                    })?;

                results.push(context.string(&data[start..end]));
                position = end;
                continue;
            }
            ItemKind::ZeroString => {
                let remaining = &data[position..];
                let length = remaining
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| {
                        error::argument_error(FUNCTION_NAME, 2, "unfinished string for format 'z'")
                    })?;

                results.push(context.string(&remaining[..length]));
                position += length + 1;
                continue;
            }
            ItemKind::Padding | ItemKind::AlignmentPadding => {}
        }

        position += item.size;
    }

    let next_position = i64::try_from(position + 1).expect("Lua string length fits in i64");
    results.push(context.integer(next_position));

    Ok(context.return_values(results))
}

fn initial_offset(position: i64, length: usize) -> Option<usize> {
    if position > 0 {
        usize::try_from(position - 1).ok()
    } else if position == 0 {
        Some(0)
    } else {
        let distance = usize::try_from(position.unsigned_abs()).unwrap_or(usize::MAX);
        Some(length.saturating_sub(distance))
    }
}
