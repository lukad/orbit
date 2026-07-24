#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParsedNumber {
    Integer(i64),
    Float(f64),
}

impl ParsedNumber {
    pub fn to_integer(self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(value),
            Self::Float(value) => float_to_integer(value),
        }
    }
}

pub fn parse_lua_number(mut source: &[u8]) -> Option<ParsedNumber> {
    source = trim_lua_whitespace(source);

    let (negative, magnitude) = match source {
        [b'+', rest @ ..] if !rest.is_empty() => (false, rest),
        [b'-', rest @ ..] if !rest.is_empty() => (true, rest),
        [] | [b'+' | b'-'] => return None,
        _ => (false, source),
    };

    if let Some(integer) = parse_integer(magnitude, negative) {
        return Some(ParsedNumber::Integer(integer));
    }

    let float = if magnitude.starts_with(b"0x") || magnitude.starts_with(b"0X") {
        parse_hex_float(magnitude)?
    } else {
        parse_decimal_float(magnitude)?
    };

    Some(ParsedNumber::Float(if negative { -float } else { float }))
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
    let mut fractional_place = 1.0 / 16.0;
    let mut digits = 0;

    for &byte in significand {
        if byte == b'.' {
            if fractional {
                return None;
            }

            fractional = true;
            continue;
        }

        let digit = f64::from(hex_value(byte)?);
        digits += 1;

        if fractional {
            value += digit * fractional_place;
            fractional_place /= 16.0;
        } else {
            value = value * 16.0 + digit;
        }
    }

    if digits == 0 {
        return None;
    }

    // Avoid 0 * infinity becoming NaN for huge positive exponents.
    if value == 0.0 {
        return Some(0.0);
    }

    Some(value * 2.0_f64.powi(exponent))
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
    use super::parse_lua_integer_with_base;

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
