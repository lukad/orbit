pub(super) fn start_offset(position: i64, len: usize) -> usize {
    if position > 0 {
        usize::try_from(position - 1).unwrap_or(usize::MAX).min(len)
    } else if position == 0 {
        0
    } else {
        let distance = usize::try_from(position.unsigned_abs()).unwrap_or(usize::MAX);
        len.saturating_sub(distance)
    }
}

pub(super) fn end_offset(position: i64, len: usize) -> usize {
    if position >= 0 {
        usize::try_from(position).unwrap_or(usize::MAX).min(len)
    } else {
        let distance = usize::try_from(position.unsigned_abs()).unwrap_or(usize::MAX);
        if distance > len {
            0
        } else {
            len - distance + 1
        }
    }
}
