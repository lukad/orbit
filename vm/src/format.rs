pub(crate) fn format_lua_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }

    if value == f64::INFINITY {
        return "inf".to_owned();
    }

    if value == f64::NEG_INFINITY {
        return "-inf".to_owned();
    }

    let mut formatted = format_general_float(value, 15);

    if formatted
        .parse::<f64>()
        .expect("generated float representation must parse")
        != value
    {
        formatted = format_general_float(value, 17);
    }

    if !formatted.contains('.') && !formatted.contains('e') {
        formatted.push_str(".0");
    }

    formatted
}

fn format_general_float(value: f64, significant_digits: usize) -> String {
    debug_assert!(value.is_finite());
    debug_assert!(significant_digits > 0);

    let scientific = format!("{value:.precision$e}", precision = significant_digits - 1,);

    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("LowerExp output always has an exponent");

    let exponent = exponent
        .parse::<i32>()
        .expect("LowerExp output always has a decimal exponent");

    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');

    if exponent < -4 || exponent >= significant_digits as i32 {
        return format!("{mantissa}e{exponent:+03}");
    }

    let (sign, coefficient) = mantissa
        .strip_prefix('-')
        .map_or(("", mantissa), |coefficient| ("-", coefficient));

    let digits = coefficient.replace('.', "");
    let decimal_position = exponent + 1;
    let mut formatted = String::from(sign);

    if decimal_position <= 0 {
        formatted.push_str("0.");
        formatted.extend(std::iter::repeat_n('0', (-decimal_position) as usize));
        formatted.push_str(&digits);
    } else if decimal_position as usize >= digits.len() {
        formatted.push_str(&digits);
        formatted.extend(std::iter::repeat_n(
            '0',
            decimal_position as usize - digits.len(),
        ));
    } else {
        let decimal_position = decimal_position as usize;

        formatted.push_str(&digits[..decimal_position]);
        formatted.push('.');
        formatted.push_str(&digits[decimal_position..]);
    }

    formatted
}

#[cfg(test)]
mod tests {
    use super::format_lua_float;

    #[test]
    fn formats_lua_floats() {
        assert_eq!(format_lua_float(0.0), "0.0");
        assert_eq!(format_lua_float(-0.0), "-0.0");
        assert_eq!(format_lua_float(1.0), "1.0");
        assert_eq!(format_lua_float(3.5), "3.5");
        assert_eq!(format_lua_float(f64::INFINITY), "inf");
        assert_eq!(format_lua_float(f64::NEG_INFINITY), "-inf");
        assert_eq!(format_lua_float(f64::NAN), "nan");
    }

    #[test]
    fn formatted_finite_values_round_trip() {
        for value in [
            f64::MIN,
            -1.2345678901234567,
            -0.00001,
            0.00001,
            1.2345678901234567,
            f64::MAX,
        ] {
            let formatted = format_lua_float(value);
            assert_eq!(formatted.parse::<f64>().unwrap(), value);
        }
    }
}
