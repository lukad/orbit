#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Integer(i64),
    Float(f64),
}

impl Number {
    pub fn to_integer(self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(value),
            Self::Float(value) => float_to_integer(value),
        }
    }
}

pub fn parse_lua_number(mut source: &[u8]) -> Option<Number> {
    source = trim_lua_whitespace(source);

    let (negative, magnitude) = match source {
        [b'+', rest @ ..] if !rest.is_empty() => (false, rest),
        [b'-', rest @ ..] if !rest.is_empty() => (true, rest),
        [] | [b'+' | b'-'] => return None,
        _ => (false, source),
    };

    if let Some(integer) = parse_integer(magnitude, negative) {
        return Some(Number::Integer(integer));
    }

    let float = if magnitude.starts_with(b"0x") || magnitude.starts_with(b"0X") {
        parse_hex_float(magnitude)?
    } else {
        parse_decimal_float(magnitude)?
    };

    Some(Number::Float(if negative { -float } else { float }))
}

pub fn parse_lua_integer_with_base(mut source: &[u8], base: u32) -> Option<i64> {
    if !(2..=36).contains(&base) {
        return None;
    }

    source = trim_lua_whitespace(source);

    let (negative, digits) = match source {
        [b'+', rest @ ..] if !rest.is_empty() => (false, rest),
        [b'-', rest @ ..] if !rest.is_empty() => (true, rest),
        [] | [b'+' | b'-'] => return None,
        _ => (false, source),
    };

    let mut value = 0_u64;
    for &digit in digits {
        let digit = digit_value(digit)?;
        if digit >= base {
            return None;
        }

        value = value
            .wrapping_mul(u64::from(base))
            .wrapping_add(u64::from(digit));
    }

    Some(if negative {
        value.wrapping_neg() as i64
    } else {
        value as i64
    })
}

fn parse_integer(source: &[u8], negative: bool) -> Option<i64> {
    if let Some(digits) = source
        .strip_prefix(b"0x")
        .or_else(|| source.strip_prefix(b"0X"))
    {
        return parse_hex_integer(digits, negative);
    }

    parse_decimal_integer(source, negative)
}

fn parse_decimal_integer(digits: &[u8], negative: bool) -> Option<i64> {
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }

    let limit = if negative {
        1_u64 << 63
    } else {
        i64::MAX as u64
    };

    let mut value = 0_u64;

    for &digit in digits {
        value = value.checked_mul(10)?;
        value = value.checked_add(u64::from(digit - b'0'))?;

        if value > limit {
            return None;
        }
    }

    if negative {
        if value == 1_u64 << 63 {
            Some(i64::MIN)
        } else {
            Some(-(value as i64))
        }
    } else {
        Some(value as i64)
    }
}

fn parse_hex_integer(digits: &[u8], negative: bool) -> Option<i64> {
    if digits.is_empty() {
        return None;
    }

    let mut value = 0_u64;

    for &digit in digits {
        let digit = hex_value(digit)?;
        value = value.wrapping_mul(16).wrapping_add(u64::from(digit));
    }

    let value = value as i64;

    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

fn parse_decimal_float(source: &[u8]) -> Option<f64> {
    let mut position = 0;
    let integer_digits = consume_digits(source, &mut position);

    let fractional_digits = if source.get(position) == Some(&b'.') {
        position += 1;
        consume_digits(source, &mut position)
    } else {
        0
    };

    if integer_digits == 0 && fractional_digits == 0 {
        return None;
    }

    if matches!(source.get(position), Some(b'e' | b'E')) {
        position += 1;

        if matches!(source.get(position), Some(b'+' | b'-')) {
            position += 1;
        }

        if consume_digits(source, &mut position) == 0 {
            return None;
        }
    }

    if position != source.len() {
        return None;
    }

    std::str::from_utf8(source).ok()?.parse().ok()
}

fn parse_hex_float(source: &[u8]) -> Option<f64> {
    const MAX_SIGNIFICANT_DIGITS: usize = 30;

    let source = source
        .strip_prefix(b"0x")
        .or_else(|| source.strip_prefix(b"0X"))?;

    let exponent_position = source.iter().position(|byte| matches!(byte, b'p' | b'P'));

    let (significand, exponent) = match exponent_position {
        Some(position) => {
            let significand = &source[..position];
            let exponent = parse_exponent(&source[position + 1..])?;
            (significand, exponent)
        }
        None => (source, 0),
    };

    let mut value = 0.0;
    let mut fractional = false;
    let mut significant_digits = 0_usize;
    let mut has_digit = false;
    let mut hexadecimal_exponent = 0_i64;

    for &byte in significand {
        if byte == b'.' {
            if fractional {
                return None;
            }

            fractional = true;
            continue;
        }

        let digit = f64::from(hex_value(byte)?);
        has_digit = true;

        if significant_digits != 0 || digit != 0.0 {
            significant_digits = significant_digits.saturating_add(1);

            if significant_digits <= MAX_SIGNIFICANT_DIGITS {
                value = value * 16.0 + digit;
            } else {
                hexadecimal_exponent = hexadecimal_exponent.saturating_add(1);
            }
        }

        if fractional {
            hexadecimal_exponent = hexadecimal_exponent.saturating_sub(1);
        }
    }

    if !has_digit {
        return None;
    }

    if value == 0.0 {
        return Some(0.0);
    }

    let binary_exponent = hexadecimal_exponent
        .saturating_mul(4)
        .saturating_add(i64::from(exponent));

    Some(scale_by_power_of_two(value, binary_exponent))
}

fn scale_by_power_of_two(value: f64, exponent: i64) -> f64 {
    debug_assert!(value.is_finite() && value > 0.0);

    const FRACTION_BITS: u32 = f64::MANTISSA_DIGITS - 1;
    const EXPONENT_BIAS: i64 = 1023;
    const MAX_EXPONENT: i64 = 1023;
    const MIN_SUBNORMAL_EXPONENT: i64 = -1074;

    let bits = value.to_bits();
    let value_exponent = i64::from(((bits >> FRACTION_BITS) & 0x7ff) as u16) - EXPONENT_BIAS;
    let fraction = bits & ((1_u64 << FRACTION_BITS) - 1);
    let normalized = f64::from_bits(((EXPONENT_BIAS as u64) << FRACTION_BITS) | fraction);
    let exponent = value_exponent.saturating_add(exponent);

    if exponent > MAX_EXPONENT {
        return f64::INFINITY;
    }

    if exponent < MIN_SUBNORMAL_EXPONENT - 1 {
        return 0.0;
    }

    if exponent == MIN_SUBNORMAL_EXPONENT - 1 {
        return (normalized * 0.5) * f64::from_bits(1);
    }

    let scale = if exponent > -EXPONENT_BIAS {
        f64::from_bits(((exponent + EXPONENT_BIAS) as u64) << FRACTION_BITS)
    } else {
        f64::from_bits(1_u64 << (exponent - MIN_SUBNORMAL_EXPONENT))
    };

    normalized * scale
}

fn parse_exponent(source: &[u8]) -> Option<i32> {
    let (negative, digits) = match source {
        [b'+', rest @ ..] => (false, rest),
        [b'-', rest @ ..] => (true, rest),
        _ => (false, source),
    };

    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }

    let mut value = 0_i32;

    for &digit in digits {
        value = value
            .saturating_mul(10)
            .saturating_add(i32::from(digit - b'0'));
    }

    Some(if negative {
        value.saturating_neg()
    } else {
        value
    })
}

fn consume_digits(source: &[u8], position: &mut usize) -> usize {
    let start = *position;

    while source.get(*position).is_some_and(u8::is_ascii_digit) {
        *position += 1;
    }

    *position - start
}

fn trim_lua_whitespace(mut source: &[u8]) -> &[u8] {
    while source.first().is_some_and(|byte| is_lua_whitespace(*byte)) {
        source = &source[1..];
    }

    while source.last().is_some_and(|byte| is_lua_whitespace(*byte)) {
        source = &source[..source.len() - 1];
    }

    source
}

fn is_lua_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

fn float_to_integer(value: f64) -> Option<i64> {
    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if value.is_finite() && value.fract() == 0.0 && value >= minimum && value < exclusive_maximum {
        Some(value as i64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Number, parse_lua_integer_with_base, parse_lua_number};

    #[test]
    fn hexadecimal_float_exponents_scale_long_significands_without_intermediate_overflow() {
        let mut source = b"0xe03".to_vec();
        source.extend(std::iter::repeat_n(b'0', 1000));
        source.extend_from_slice(b"p-4000");

        assert_eq!(parse_lua_number(&source), Some(Number::Float(3587.0)));
    }

    #[test]
    fn hexadecimal_float_exponents_scale_long_fractional_prefixes_without_underflow() {
        let mut source = b"0x0.".to_vec();
        source.extend(std::iter::repeat_n(b'0', 1000));
        source.extend_from_slice(b"e03p4000");

        assert_eq!(
            parse_lua_number(&source),
            Some(Number::Float(3587.0 / 4096.0))
        );
    }

    #[test]
    fn hexadecimal_float_scaling_handles_finite_overflow_and_subnormal_boundaries() {
        for (source, expected) in [
            (b"0x1.fffffffffffffp1023".as_slice(), f64::MAX),
            (b"0x1p1024".as_slice(), f64::INFINITY),
            (b"0x1p-1074".as_slice(), f64::from_bits(1)),
            (b"0x1p-1075".as_slice(), 0.0),
            (b"0x1.8p-1075".as_slice(), f64::from_bits(1)),
        ] {
            assert_eq!(parse_lua_number(source), Some(Number::Float(expected)));
        }
    }

    #[test]
    fn parses_integer_numerals_in_explicit_bases() {
        assert_eq!(parse_lua_integer_with_base(b"101", 2), Some(5));
        assert_eq!(parse_lua_integer_with_base(b" -fF ", 16), Some(-255));
        assert_eq!(parse_lua_integer_with_base(b"z", 36), Some(35));
        assert_eq!(
            parse_lua_integer_with_base(b"ffffffffffffffff", 16),
            Some(-1)
        );
    }

    #[test]
    fn rejects_invalid_explicit_base_numerals() {
        assert_eq!(parse_lua_integer_with_base(b"", 10), None);
        assert_eq!(parse_lua_integer_with_base(b"+", 10), None);
        assert_eq!(parse_lua_integer_with_base(b"2", 2), None);
        assert_eq!(parse_lua_integer_with_base(b"10x", 10), None);
        assert_eq!(parse_lua_integer_with_base(b"10", 1), None);
        assert_eq!(parse_lua_integer_with_base(b"10", 37), None);
    }
}
