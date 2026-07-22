use crate::{
    error::{FaultResult, VmErrorKind},
    format::format_lua_float,
    string::LuaString,
    value::RawValue,
};

pub(super) fn concat(left: &RawValue, right: &RawValue) -> FaultResult<RawValue> {
    let Some(mut left_bytes) = concat_bytes(left) else {
        return Err(VmErrorKind::InvalidConcatOperands {
            left: left.type_name(),
            right: right.type_name(),
        });
    };

    let Some(right_bytes) = concat_bytes(right) else {
        return Err(VmErrorKind::InvalidConcatOperands {
            left: left.type_name(),
            right: right.type_name(),
        });
    };

    let length = left_bytes
        .len()
        .checked_add(right_bytes.len())
        .ok_or(VmErrorKind::StringTooLong { length: usize::MAX })?;

    left_bytes
        .try_reserve(right_bytes.len())
        .map_err(|_| VmErrorKind::StringTooLong { length })?;

    left_bytes.extend_from_slice(&right_bytes);

    Ok(RawValue::String(LuaString::from(left_bytes)))
}

fn concat_bytes(value: &RawValue) -> Option<Vec<u8>> {
    match value {
        RawValue::String(value) => Some(value.as_bytes().to_vec()),
        RawValue::Integer(value) => Some(value.to_string().into_bytes()),
        RawValue::Float(value) => Some(format_lua_float(*value).into_bytes()),
        RawValue::Nil | RawValue::Boolean(_) | RawValue::Table(_) | RawValue::Function(_) => None,
    }
}
