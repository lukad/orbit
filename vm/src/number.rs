pub(crate) fn float_to_integer(value: f64) -> Option<i64> {
    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if value.is_finite() && value.fract() == 0.0 && value >= minimum && value < exclusive_maximum {
        Some(value as i64)
    } else {
        None
    }
}

pub(crate) fn integer_floor_divide(left: i64, right: i64) -> i64 {
    debug_assert_ne!(right, 0);

    if right == -1 {
        return left.wrapping_neg();
    }

    let quotient = left / right;
    let remainder = left % right;

    if remainder != 0 && (remainder < 0) != (right < 0) {
        quotient - 1
    } else {
        quotient
    }
}

pub(crate) fn integer_modulo(left: i64, right: i64) -> i64 {
    debug_assert_ne!(right, 0);

    if right == -1 {
        return 0;
    }

    let remainder = left % right;

    if remainder != 0 && (remainder < 0) != (right < 0) {
        remainder + right
    } else {
        remainder
    }
}

pub(crate) fn float_modulo(left: f64, right: f64) -> f64 {
    let mut remainder = left % right;

    if (remainder > 0.0 && right < 0.0) || (remainder < 0.0 && right > 0.0) {
        remainder += right;
    }

    remainder
}

pub(crate) fn shift_left(value: i64, distance: i64) -> i64 {
    if !(-63..=63).contains(&distance) {
        return 0;
    }

    if distance < 0 {
        ((value as u64) >> distance.unsigned_abs() as u32) as i64
    } else {
        ((value as u64) << distance as u32) as i64
    }
}

pub(crate) fn shift_right(value: i64, distance: i64) -> i64 {
    if !(-63..=63).contains(&distance) {
        return 0;
    }

    if distance < 0 {
        ((value as u64) << distance.unsigned_abs() as u32) as i64
    } else {
        ((value as u64) >> distance as u32) as i64
    }
}

pub(crate) fn integer_less_float(integer: i64, float: f64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float >= exclusive_maximum {
        true
    } else if float < minimum {
        false
    } else if float.fract() == 0.0 {
        integer < float as i64
    } else {
        integer <= float.floor() as i64
    }
}

pub(crate) fn integer_less_equal_float(integer: i64, float: f64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float >= exclusive_maximum {
        true
    } else if float < minimum {
        false
    } else {
        integer <= float.floor() as i64
    }
}

pub(crate) fn float_less_integer(float: f64, integer: i64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float < minimum {
        true
    } else if float >= exclusive_maximum {
        false
    } else {
        (float.floor() as i64) < integer
    }
}

pub(crate) fn float_less_equal_integer(float: f64, integer: i64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float < minimum {
        true
    } else if float >= exclusive_maximum {
        false
    } else {
        float.ceil() as i64 <= integer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_only_exact_representable_integers() {
        assert_eq!(float_to_integer(0.0), Some(0));
        assert_eq!(float_to_integer(-0.0), Some(0));
        assert_eq!(float_to_integer(42.0), Some(42));
        assert_eq!(float_to_integer(i64::MIN as f64), Some(i64::MIN));

        assert_eq!(float_to_integer(1.5), None);
        assert_eq!(float_to_integer(f64::NAN), None);
        assert_eq!(float_to_integer(f64::INFINITY), None);
        assert_eq!(float_to_integer(i64::MAX as f64), None);
    }

    #[test]
    fn floor_division_rounds_toward_negative_infinity() {
        assert_eq!(integer_floor_divide(7, 3), 2);
        assert_eq!(integer_floor_divide(-7, 3), -3);
        assert_eq!(integer_floor_divide(7, -3), -3);
        assert_eq!(integer_floor_divide(-7, -3), 2);

        assert_eq!(integer_floor_divide(i64::MIN, -1), i64::MIN);
    }

    #[test]
    fn modulo_has_the_divisor_sign() {
        assert_eq!(integer_modulo(7, 3), 1);
        assert_eq!(integer_modulo(-7, 3), 2);
        assert_eq!(integer_modulo(7, -3), -2);
        assert_eq!(integer_modulo(-7, -3), -1);
        assert_eq!(integer_modulo(i64::MIN, -1), 0);
    }

    #[test]
    fn negative_shift_distances_reverse_direction() {
        assert_eq!(shift_left(8, -1), 4);
        assert_eq!(shift_right(8, -1), 16);
        assert_eq!(shift_left(1, 64), 0);
        assert_eq!(shift_right(1, -64), 0);
    }
}
