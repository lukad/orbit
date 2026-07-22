use super::TableData;

pub(super) fn raw_len(table: &TableData) -> i64 {
    if !table.contains_integer(1) {
        return 0;
    }

    let mut lower = 1_i64;
    let mut upper = 2_i64;

    while table.contains_integer(upper) {
        lower = upper;

        let Some(next) = upper.checked_mul(2) else {
            return i64::MAX;
        };

        upper = next;
    }

    while upper - lower > 1 {
        let middle = lower + (upper - lower) / 2;

        if table.contains_integer(middle) {
            lower = middle;
        } else {
            upper = middle;
        }
    }

    lower
}
