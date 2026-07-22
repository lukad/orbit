use crate::{
    error::{FaultResult, VmErrorKind},
    table::{
        TableData,
        key::{KeyNormalization, normalize_key},
    },
    value::RawValue,
};

pub(super) fn next(
    table: &TableData,
    previous: &RawValue,
) -> FaultResult<Option<(RawValue, RawValue)>> {
    if previous.is_nil() {
        return Ok(next_after_array_position(table, 0));
    }

    let key = match normalize_key(previous) {
        KeyNormalization::Key(key) => key,
        KeyNormalization::Nil | KeyNormalization::NaN => {
            return Err(VmErrorKind::InvalidKeyToNext);
        }
    };

    if let Some(one_based) = key.positive_integer_index() {
        let zero_based = one_based - 1;

        if table.is_array_cursor(zero_based) {
            return Ok(next_after_array_position(
                table,
                zero_based.saturating_add(1),
            ));
        }
    }

    let position = table
        .hash
        .position(&key)
        .ok_or(VmErrorKind::InvalidKeyToNext)?;

    Ok(table
        .hash
        .next_live_from(position.saturating_add(1))
        .map(|(_, key, value)| (key.to_raw_value(), value.clone())))
}

fn next_after_array_position(table: &TableData, start: usize) -> Option<(RawValue, RawValue)> {
    for (zero_based, value) in table.array.iter().enumerate().skip(start) {
        if value.is_nil() {
            continue;
        }

        let one_based = i64::try_from(zero_based).ok()?.checked_add(1)?;

        return Some((RawValue::Integer(one_based), value.clone()));
    }

    table
        .hash
        .next_live_from(0)
        .map(|(_, key, value)| (key.to_raw_value(), value.clone()))
}
