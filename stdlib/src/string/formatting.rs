use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Conversion {
    /// %%
    Percent,
    /// %c
    Character {
        left_align: bool,
        width: Option<u8>,
    },
    /// %d %i
    SignedDecimal {
        left_align: bool,
        force_sign: bool,
        leading_space: bool,
        zero_pad: bool,
        width: Option<u8>,
        precision: Option<u8>,
    },
    /// %u
    UnsignedDecimal {
        left_align: bool,
        zero_pad: bool,
        width: Option<u8>,
        precision: Option<u8>,
    },
    /// %o
    Octal {
        left_align: bool,
        alternate: bool,
        zero_pad: bool,
        width: Option<u8>,
        precision: Option<u8>,
    },
    /// %x %X
    Hexadecimal {
        uppercase: bool,
        left_align: bool,
        alternate: bool,
        zero_pad: bool,
        width: Option<u8>,
        precision: Option<u8>,
    },
    /// %a %A   FloatStyle::Hexadecimal
    /// %e %E   FloatStyle::Scientific
    /// %f      FloatStyle::Fixed
    /// %g %G   FloatStyle::General
    Float {
        style: FloatStyle,
        left_align: bool,
        force_sign: bool,
        leading_space: bool,
        alternate: bool,
        zero_pad: bool,
        width: Option<u8>,
        precision: Option<u8>,
    },
    String {
        left_align: bool,
        width: Option<u8>,
        precision: Option<u8>,
    },
    Pointer {
        left_align: bool,
        width: Option<u8>,
    },
    Quote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatStyle {
    /// %a %A
    Hexadecimal { uppercase: bool },
    /// %e %E
    Scientific { uppercase: bool },
    /// %f
    Fixed,
    /// %g %G
    General { uppercase: bool },
}

const LEFT: u8 = 1 << 0;
const PLUS: u8 = 1 << 1;
const SPACE: u8 = 1 << 2;
const ALTERNATE: u8 = 1 << 3;
const ZERO: u8 = 1 << 4;
const ALL_FLAGS: u8 = LEFT | PLUS | SPACE | ALTERNATE | ZERO;

const MAX_SPECIFIER_LENGTH: usize = 21;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("format specification must start with '%'")]
    MissingPercent,
    #[error("incomplete format specification")]
    MissingConversion,
    #[error("invalid conversion '{0}' to 'format'")]
    InvalidConversion(char),
    #[error("invalid conversion specification for '{0}'")]
    InvalidFlags(char),
    #[error("invalid conversion specification for '{0}'")]
    PrecisionNotAllowed(char),
    #[error("specifier '%q' cannot have modifiers")]
    QuoteHasModifiers,
    #[error("invalid format (too long)")]
    TooLong,
    #[error("trailing characters after format specification")]
    TrailingCharacters,
}

#[derive(Clone, Copy, Debug, Default)]
struct Modifiers {
    flags: u8,
    width: Option<u8>,
    precision: Option<u8>,
}

impl Modifiers {
    fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    fn require_flags(self, allowed: u8, conversion: u8) -> Result<Self, FormatError> {
        if self.flags & !allowed != 0 {
            Err(FormatError::InvalidFlags(char::from(conversion)))
        } else {
            Ok(self)
        }
    }

    fn reject_precision(self, conversion: u8) -> Result<Self, FormatError> {
        if self.precision.is_some() {
            Err(FormatError::PrecisionNotAllowed(char::from(conversion)))
        } else {
            Ok(self)
        }
    }

    fn has_any_modifier(self) -> bool {
        self.flags != 0 || self.width.is_some() || self.precision.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FormatSpec {
    conversion: u8,
    modifier_bytes: [u8; MAX_SPECIFIER_LENGTH - 1],
    modifier_length: u8,
}

impl FormatSpec {
    pub(crate) fn parse(input: &[u8]) -> Result<(Self, usize), FormatError> {
        if input.first() != Some(&b'%') {
            return Err(FormatError::MissingPercent);
        }

        if input.get(1) == Some(&b'%') {
            return Ok((
                Self {
                    conversion: b'%',
                    modifier_bytes: [0; MAX_SPECIFIER_LENGTH - 1],
                    modifier_length: 0,
                },
                2,
            ));
        }

        let modifier_length = input[1..]
            .iter()
            .take_while(|byte| b"-+ #0123456789.".contains(byte))
            .count();
        if modifier_length + 1 > MAX_SPECIFIER_LENGTH {
            return Err(FormatError::TooLong);
        }

        let conversion_index = 1 + modifier_length;
        let conversion = *input
            .get(conversion_index)
            .ok_or(FormatError::MissingConversion)?;

        let mut modifier_bytes = [0; MAX_SPECIFIER_LENGTH - 1];
        modifier_bytes[..modifier_length].copy_from_slice(&input[1..conversion_index]);

        Ok((
            Self {
                conversion,
                modifier_bytes,
                modifier_length: modifier_length
                    .try_into()
                    .expect("format modifier length is bounded"),
            },
            conversion_index + 1,
        ))
    }

    pub(crate) fn is_percent(self) -> bool {
        self.conversion == b'%' && self.modifier_length == 0
    }

    pub(crate) fn conversion_byte(self) -> u8 {
        self.conversion
    }

    pub(crate) fn has_modifiers(self) -> bool {
        self.modifier_length != 0
    }

    pub(crate) fn validate(self) -> Result<Conversion, FormatError> {
        if self.is_percent() {
            return Ok(Conversion::Percent);
        }

        let conversion = self.conversion;
        if !matches!(
            conversion,
            b'c' | b'd'
                | b'i'
                | b'u'
                | b'o'
                | b'x'
                | b'X'
                | b'a'
                | b'A'
                | b'e'
                | b'E'
                | b'f'
                | b'g'
                | b'G'
                | b's'
                | b'p'
                | b'q'
        ) {
            return Err(FormatError::InvalidConversion(char::from(conversion)));
        }

        let modifiers = parse_modifiers(self.modifier_bytes(), conversion)?;

        let parsed = match conversion {
            b'c' => {
                let modifiers = modifiers
                    .require_flags(LEFT, conversion)?
                    .reject_precision(conversion)?;

                Conversion::Character {
                    left_align: modifiers.has(LEFT),
                    width: modifiers.width,
                }
            }
            b'd' | b'i' => {
                let modifiers = modifiers.require_flags(LEFT | PLUS | SPACE | ZERO, conversion)?;

                Conversion::SignedDecimal {
                    left_align: modifiers.has(LEFT),
                    force_sign: modifiers.has(PLUS),
                    leading_space: modifiers.has(SPACE),
                    zero_pad: modifiers.has(ZERO),
                    width: modifiers.width,
                    precision: modifiers.precision,
                }
            }
            b'u' => {
                let modifiers = modifiers.require_flags(LEFT | ZERO, conversion)?;

                Conversion::UnsignedDecimal {
                    left_align: modifiers.has(LEFT),
                    zero_pad: modifiers.has(ZERO),
                    width: modifiers.width,
                    precision: modifiers.precision,
                }
            }
            b'o' => {
                let modifiers = modifiers.require_flags(LEFT | ALTERNATE | ZERO, conversion)?;

                Conversion::Octal {
                    left_align: modifiers.has(LEFT),
                    alternate: modifiers.has(ALTERNATE),
                    zero_pad: modifiers.has(ZERO),
                    width: modifiers.width,
                    precision: modifiers.precision,
                }
            }
            b'x' | b'X' => {
                let modifiers = modifiers.require_flags(LEFT | ALTERNATE | ZERO, conversion)?;

                Conversion::Hexadecimal {
                    uppercase: conversion == b'X',
                    left_align: modifiers.has(LEFT),
                    alternate: modifiers.has(ALTERNATE),
                    zero_pad: modifiers.has(ZERO),
                    width: modifiers.width,
                    precision: modifiers.precision,
                }
            }
            b'a' | b'A' | b'e' | b'E' | b'f' | b'g' | b'G' => {
                let modifiers = modifiers.require_flags(ALL_FLAGS, conversion)?;

                let style = match conversion {
                    b'a' => FloatStyle::Hexadecimal { uppercase: false },
                    b'A' => FloatStyle::Hexadecimal { uppercase: true },
                    b'e' => FloatStyle::Scientific { uppercase: false },
                    b'E' => FloatStyle::Scientific { uppercase: true },
                    b'f' => FloatStyle::Fixed,
                    b'g' => FloatStyle::General { uppercase: false },
                    b'G' => FloatStyle::General { uppercase: true },
                    _ => unreachable!(),
                };

                Conversion::Float {
                    style,
                    left_align: modifiers.has(LEFT),
                    force_sign: modifiers.has(PLUS),
                    leading_space: modifiers.has(SPACE),
                    alternate: modifiers.has(ALTERNATE),
                    zero_pad: modifiers.has(ZERO),
                    width: modifiers.width,
                    precision: modifiers.precision,
                }
            }
            b's' => {
                let modifiers = modifiers.require_flags(LEFT, conversion)?;

                Conversion::String {
                    left_align: modifiers.has(LEFT),
                    width: modifiers.width,
                    precision: modifiers.precision,
                }
            }
            b'p' => {
                let modifiers = modifiers
                    .require_flags(LEFT, conversion)?
                    .reject_precision(conversion)?;

                Conversion::Pointer {
                    left_align: modifiers.has(LEFT),
                    width: modifiers.width,
                }
            }
            b'q' => {
                if modifiers.has_any_modifier() {
                    return Err(FormatError::QuoteHasModifiers);
                }

                Conversion::Quote
            }
            _ => unreachable!("known conversion byte"),
        };

        Ok(parsed)
    }

    fn modifier_bytes(&self) -> &[u8] {
        &self.modifier_bytes[..usize::from(self.modifier_length)]
    }
}

impl Conversion {
    pub(crate) fn parse(input: &[u8]) -> Result<(Self, usize), FormatError> {
        let (spec, consumed) = FormatSpec::parse(input)?;
        Ok((spec.validate()?, consumed))
    }
}

fn parse_modifiers(input: &[u8], conversion: u8) -> Result<Modifiers, FormatError> {
    let mut cursor = 0;
    let flags = parse_flags(input, &mut cursor);
    let width = parse_digits(input, &mut cursor);
    let precision = if input.get(cursor) == Some(&b'.') {
        cursor += 1;
        Some(parse_digits(input, &mut cursor).unwrap_or(0))
    } else {
        None
    };

    if cursor != input.len() {
        return Err(FormatError::InvalidFlags(char::from(conversion)));
    }

    Ok(Modifiers {
        flags,
        width,
        precision,
    })
}

fn parse_flags(input: &[u8], cursor: &mut usize) -> u8 {
    let mut flags = 0;

    loop {
        let flag = match input.get(*cursor) {
            Some(b'-') => LEFT,
            Some(b'+') => PLUS,
            Some(b' ') => SPACE,
            Some(b'#') => ALTERNATE,
            Some(b'0') => ZERO,
            _ => break,
        };

        flags |= flag;
        *cursor += 1;
    }

    flags
}

fn parse_digits(input: &[u8], cursor: &mut usize) -> Option<u8> {
    let first = input.get(*cursor).copied()?;
    if !first.is_ascii_digit() {
        return None;
    }

    let mut value = first - b'0';
    *cursor += 1;

    if let Some(second) = input.get(*cursor).copied().filter(u8::is_ascii_digit) {
        value = value * 10 + (second - b'0');
        *cursor += 1;
    }

    Some(value)
}

impl FromStr for Conversion {
    type Err = FormatError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        let (conversion, consumed) = Self::parse(source.as_bytes())?;

        if consumed != source.len() {
            return Err(FormatError::TrailingCharacters);
        }

        Ok(conversion)
    }
}

impl Conversion {
    pub(crate) fn append_integer(self, output: &mut Vec<u8>, value: i64) {
        match self {
            Self::Character { left_align, width } => {
                append_character(output, value as u8, left_align, width);
            }
            Self::SignedDecimal {
                left_align,
                force_sign,
                leading_space,
                zero_pad,
                width,
                precision,
            } => {
                let sign = if value < 0 {
                    Some(b'-')
                } else if force_sign {
                    Some(b'+')
                } else if leading_space {
                    Some(b' ')
                } else {
                    None
                };
                let mut digits = unsigned_digits(value.unsigned_abs(), 10, false);
                apply_integer_precision(&mut digits, value == 0, precision);
                append_integer_field(
                    output,
                    sign,
                    &[],
                    &digits,
                    FieldOptions {
                        left_align,
                        zero_pad,
                        width,
                    },
                    precision,
                );
            }
            Self::UnsignedDecimal {
                left_align,
                zero_pad,
                width,
                precision,
            } => {
                let value = value as u64;
                let mut digits = unsigned_digits(value, 10, false);
                apply_integer_precision(&mut digits, value == 0, precision);
                append_integer_field(
                    output,
                    None,
                    &[],
                    &digits,
                    FieldOptions {
                        left_align,
                        zero_pad,
                        width,
                    },
                    precision,
                );
            }
            Self::Octal {
                left_align,
                alternate,
                zero_pad,
                width,
                precision,
            } => {
                let value = value as u64;
                let mut digits = unsigned_digits(value, 8, false);
                apply_integer_precision(&mut digits, value == 0, precision);
                if alternate && digits.first() != Some(&b'0') {
                    digits.insert(0, b'0');
                }
                append_integer_field(
                    output,
                    None,
                    &[],
                    &digits,
                    FieldOptions {
                        left_align,
                        zero_pad,
                        width,
                    },
                    precision,
                );
            }
            Self::Hexadecimal {
                uppercase,
                left_align,
                alternate,
                zero_pad,
                width,
                precision,
            } => {
                let value = value as u64;
                let mut digits = unsigned_digits(value, 16, uppercase);
                apply_integer_precision(&mut digits, value == 0, precision);
                let prefix = if alternate && value != 0 {
                    if uppercase { &b"0X"[..] } else { &b"0x"[..] }
                } else {
                    &[]
                };
                append_integer_field(
                    output,
                    None,
                    prefix,
                    &digits,
                    FieldOptions {
                        left_align,
                        zero_pad,
                        width,
                    },
                    precision,
                );
            }
            _ => unreachable!("non-integer conversion"),
        }
    }

    pub(crate) fn append_float(self, output: &mut Vec<u8>, value: f64) {
        let Self::Float {
            style,
            left_align,
            force_sign,
            leading_space,
            alternate,
            zero_pad,
            width,
            precision,
        } = self
        else {
            unreachable!("non-float conversion")
        };

        let uppercase = matches!(
            style,
            FloatStyle::Hexadecimal { uppercase: true }
                | FloatStyle::Scientific { uppercase: true }
                | FloatStyle::General { uppercase: true }
        );

        let sign = if value.is_nan() {
            None
        } else if value.is_sign_negative() {
            Some(b'-')
        } else if force_sign {
            Some(b'+')
        } else if leading_space {
            Some(b' ')
        } else {
            None
        };

        let (magnitude, prefix_length, zero_padding_allowed) = if value.is_nan() {
            (
                if uppercase {
                    b"NAN".to_vec()
                } else {
                    b"nan".to_vec()
                },
                0,
                false,
            )
        } else if value.is_infinite() {
            (
                if uppercase {
                    b"INF".to_vec()
                } else {
                    b"inf".to_vec()
                },
                0,
                false,
            )
        } else {
            let magnitude = match style {
                FloatStyle::Hexadecimal { uppercase } => {
                    format_hex_float(value.abs(), uppercase, alternate, precision)
                }
                FloatStyle::Scientific { uppercase } => format_scientific(
                    value.abs(),
                    usize::from(precision.unwrap_or(6)),
                    uppercase,
                    alternate,
                ),
                FloatStyle::Fixed => {
                    format_fixed(value.abs(), usize::from(precision.unwrap_or(6)), alternate)
                }
                FloatStyle::General { uppercase } => format_general(
                    value.abs(),
                    usize::from(precision.unwrap_or(6)),
                    uppercase,
                    alternate,
                ),
            };
            let prefix_length = usize::from(matches!(style, FloatStyle::Hexadecimal { .. })) * 2;
            (magnitude, prefix_length, true)
        };

        append_float_field(
            output,
            sign,
            &magnitude,
            prefix_length,
            FieldOptions {
                left_align,
                zero_pad,
                width,
            },
            zero_padding_allowed,
        );
    }

    pub(crate) fn append_pointer(self, output: &mut Vec<u8>, representation: &[u8]) {
        let Self::Pointer { left_align, width } = self else {
            unreachable!("non-pointer conversion")
        };

        let padding = width
            .map(usize::from)
            .unwrap_or_default()
            .saturating_sub(representation.len());

        if !left_align {
            output.resize(output.len() + padding, b' ');
        }

        output.extend_from_slice(representation);

        if left_align {
            output.resize(output.len() + padding, b' ');
        }
    }

    pub(crate) fn append_quoted_string(output: &mut Vec<u8>, bytes: &[u8]) {
        output.push(b'"');

        for (index, &byte) in bytes.iter().enumerate() {
            match byte {
                b'"' | b'\\' | b'\n' => {
                    output.push(b'\\');
                    output.push(byte);
                }
                0x00..=0x1f | 0x7f => {
                    let next_is_digit = bytes.get(index + 1).is_some_and(u8::is_ascii_digit);

                    if next_is_digit {
                        output.extend_from_slice(format!("\\{byte:03}").as_bytes());
                    } else {
                        output.extend_from_slice(format!("\\{byte}").as_bytes());
                    }
                }
                _ => output.push(byte),
            }
        }

        output.push(b'"');
    }
}

pub(crate) fn quoted_float(value: f64) -> Vec<u8> {
    if value == f64::INFINITY {
        b"1e9999".to_vec()
    } else if value == f64::NEG_INFINITY {
        b"-1e9999".to_vec()
    } else if value.is_nan() {
        b"(0/0)".to_vec()
    } else {
        let mut result = Vec::new();
        if value.is_sign_negative() {
            result.push(b'-');
        }
        result.extend(format_hex_float(value.abs(), false, false, None));
        result
    }
}

fn append_character(output: &mut Vec<u8>, value: u8, left_align: bool, width: Option<u8>) {
    let padding = width.map(usize::from).unwrap_or_default().saturating_sub(1);

    if !left_align {
        output.resize(output.len() + padding, b' ');
    }
    output.push(value);
    if left_align {
        output.resize(output.len() + padding, b' ');
    }
}

fn unsigned_digits(mut value: u64, radix: u64, uppercase: bool) -> Vec<u8> {
    if value == 0 {
        return vec![b'0'];
    }

    let alphabet = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut reversed = Vec::new();
    while value != 0 {
        reversed.push(alphabet[(value % radix) as usize]);
        value /= radix;
    }
    reversed.reverse();
    reversed
}

fn apply_integer_precision(digits: &mut Vec<u8>, is_zero: bool, precision: Option<u8>) {
    if is_zero && precision == Some(0) {
        digits.clear();
        return;
    }

    let zero_count = precision
        .map(usize::from)
        .unwrap_or_default()
        .saturating_sub(digits.len());
    if zero_count != 0 {
        let mut padded = Vec::with_capacity(zero_count + digits.len());
        padded.resize(zero_count, b'0');
        padded.extend_from_slice(digits);
        *digits = padded;
    }
}

#[derive(Clone, Copy)]
struct FieldOptions {
    left_align: bool,
    zero_pad: bool,
    width: Option<u8>,
}

fn append_integer_field(
    output: &mut Vec<u8>,
    sign: Option<u8>,
    prefix: &[u8],
    digits: &[u8],
    options: FieldOptions,
    precision: Option<u8>,
) {
    let content_length = usize::from(sign.is_some()) + prefix.len() + digits.len();
    let padding = options
        .width
        .map(usize::from)
        .unwrap_or_default()
        .saturating_sub(content_length);
    let pad_with_zeros = options.zero_pad && !options.left_align && precision.is_none();

    if !options.left_align && !pad_with_zeros {
        output.resize(output.len() + padding, b' ');
    }
    if let Some(sign) = sign {
        output.push(sign);
    }
    output.extend_from_slice(prefix);
    if pad_with_zeros {
        output.resize(output.len() + padding, b'0');
    }
    output.extend_from_slice(digits);
    if options.left_align {
        output.resize(output.len() + padding, b' ');
    }
}

fn append_float_field(
    output: &mut Vec<u8>,
    sign: Option<u8>,
    magnitude: &[u8],
    prefix_length: usize,
    options: FieldOptions,
    zero_padding_allowed: bool,
) {
    let content_length = usize::from(sign.is_some()) + magnitude.len();
    let padding = options
        .width
        .map(usize::from)
        .unwrap_or_default()
        .saturating_sub(content_length);
    let pad_with_zeros = options.zero_pad && zero_padding_allowed && !options.left_align;

    if !options.left_align && !pad_with_zeros {
        output.resize(output.len() + padding, b' ');
    }
    if let Some(sign) = sign {
        output.push(sign);
    }
    output.extend_from_slice(&magnitude[..prefix_length]);
    if pad_with_zeros {
        output.resize(output.len() + padding, b'0');
    }
    output.extend_from_slice(&magnitude[prefix_length..]);
    if options.left_align {
        output.resize(output.len() + padding, b' ');
    }
}

fn format_fixed(value: f64, precision: usize, alternate: bool) -> Vec<u8> {
    let mut result = format!("{value:.precision$}").into_bytes();
    if alternate && precision == 0 {
        result.push(b'.');
    }
    result
}

fn format_scientific(value: f64, precision: usize, uppercase: bool, alternate: bool) -> Vec<u8> {
    let raw = format!("{value:.precision$e}");
    let (coefficient, exponent) = raw
        .split_once('e')
        .expect("Rust scientific formatting includes an exponent");
    let exponent: i32 = exponent
        .parse()
        .expect("Rust scientific formatting produces a decimal exponent");

    let mut result = coefficient.as_bytes().to_vec();
    if alternate && precision == 0 {
        result.push(b'.');
    }
    append_exponent(&mut result, exponent, uppercase, true);
    result
}

fn format_general(value: f64, precision: usize, uppercase: bool, alternate: bool) -> Vec<u8> {
    let precision = precision.max(1);
    let scientific = format!(
        "{value:.fraction_digits$e}",
        fraction_digits = precision - 1
    );
    let (_, exponent) = scientific
        .split_once('e')
        .expect("Rust scientific formatting includes an exponent");
    let exponent: i32 = exponent
        .parse()
        .expect("Rust scientific formatting produces a decimal exponent");

    let mut result = if exponent < -4 || exponent >= precision as i32 {
        format_scientific(value, precision - 1, uppercase, alternate)
    } else {
        let fraction_digits = usize::try_from(precision as i32 - exponent - 1)
            .expect("fixed general format has a nonnegative precision");
        format_fixed(value, fraction_digits, alternate)
    };

    if !alternate {
        trim_fraction_zeros(&mut result);
    }
    result
}

fn trim_fraction_zeros(value: &mut Vec<u8>) {
    let exponent_start = value
        .iter()
        .position(|byte| matches!(byte, b'e' | b'E'))
        .unwrap_or(value.len());
    let Some(decimal_point) = value[..exponent_start]
        .iter()
        .position(|byte| *byte == b'.')
    else {
        return;
    };

    let mut fraction_end = exponent_start;
    while fraction_end > decimal_point + 1 && value[fraction_end - 1] == b'0' {
        fraction_end -= 1;
    }
    if fraction_end == decimal_point + 1 {
        fraction_end = decimal_point;
    }
    if fraction_end != exponent_start {
        value.drain(fraction_end..exponent_start);
    }
}

fn append_exponent(output: &mut Vec<u8>, exponent: i32, uppercase: bool, two_digits: bool) {
    output.push(if uppercase { b'E' } else { b'e' });
    output.push(if exponent < 0 { b'-' } else { b'+' });

    let digits = exponent.unsigned_abs().to_string();
    if two_digits && digits.len() < 2 {
        output.push(b'0');
    }
    output.extend_from_slice(digits.as_bytes());
}

fn format_hex_float(
    value: f64,
    uppercase: bool,
    alternate: bool,
    precision: Option<u8>,
) -> Vec<u8> {
    debug_assert!(value.is_finite() && !value.is_sign_negative());

    let (significand, exponent) = normalized_significand(value);
    let mut leading_digit = (significand >> 52) as u8;
    let exact_fraction = significand & ((1_u64 << 52) - 1);
    let mut fraction = Vec::new();

    match precision.map(usize::from) {
        None => {
            for index in 0..13 {
                let shift = 48 - index * 4;
                fraction.push(hex_digit(
                    ((exact_fraction >> shift) & 0xf) as u8,
                    uppercase,
                ));
            }
            while fraction.last() == Some(&b'0') {
                fraction.pop();
            }
        }
        Some(requested) if requested < 13 => {
            let dropped_bits = 52 - requested * 4;
            let mut retained = significand >> dropped_bits;
            let remainder_mask = (1_u64 << dropped_bits) - 1;
            let remainder = significand & remainder_mask;
            let halfway = 1_u64 << (dropped_bits - 1);
            if remainder > halfway || (remainder == halfway && retained & 1 != 0) {
                retained += 1;
            }

            leading_digit = (retained >> (requested * 4)) as u8;
            for index in 0..requested {
                let shift = (requested - index - 1) * 4;
                fraction.push(hex_digit(((retained >> shift) & 0xf) as u8, uppercase));
            }
        }
        Some(requested) => {
            for index in 0..13 {
                let shift = 48 - index * 4;
                fraction.push(hex_digit(
                    ((exact_fraction >> shift) & 0xf) as u8,
                    uppercase,
                ));
            }
            fraction.resize(requested, b'0');
        }
    }

    let mut result = if uppercase {
        b"0X".to_vec()
    } else {
        b"0x".to_vec()
    };
    result.push(hex_digit(leading_digit, uppercase));
    if alternate || !fraction.is_empty() {
        result.push(b'.');
        result.extend_from_slice(&fraction);
    }
    append_binary_exponent(&mut result, exponent, uppercase);
    result
}

fn append_binary_exponent(output: &mut Vec<u8>, exponent: i32, uppercase: bool) {
    output.push(if uppercase { b'P' } else { b'p' });
    output.push(if exponent < 0 { b'-' } else { b'+' });
    output.extend_from_slice(exponent.unsigned_abs().to_string().as_bytes());
}

fn normalized_significand(value: f64) -> (u64, i32) {
    let bits = value.to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1_u64 << 52) - 1);

    if exponent_bits != 0 {
        ((1_u64 << 52) | fraction, exponent_bits - 1023)
    } else if fraction == 0 {
        (0, 0)
    } else {
        let highest_bit = 63 - fraction.leading_zeros();
        let shift = 52 - highest_bit;
        (fraction << shift, -1022 - shift as i32)
    }
}

fn hex_digit(value: u8, uppercase: bool) -> u8 {
    match value {
        0..=9 => b'0' + value,
        10..=15 if uppercase => b'A' + value - 10,
        10..=15 => b'a' + value - 10,
        _ => unreachable!("hexadecimal digit is in range"),
    }
}
