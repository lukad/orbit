use core::ffi::{c_int, c_long, c_short};
use std::mem::{align_of, size_of};

const MAX_INTEGER_SIZE: usize = 16;
const MAX_TOTAL_SIZE: usize = i32::MAX as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Endian {
    Little,
    Big,
}

impl Endian {
    fn native() -> Self {
        if cfg!(target_endian = "little") {
            Self::Little
        } else {
            Self::Big
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemKind {
    SignedInteger,
    UnsignedInteger,
    Float,
    Double,
    LuaNumber,
    FixedString,
    LengthString,
    ZeroString,
    Padding,
    AlignmentPadding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub kind: ItemKind,
    pub size: usize,
    pub padding: usize,
    pub endian: Endian,
}

pub(crate) struct FormatParser<'a> {
    format: &'a [u8],
    cursor: usize,
    endian: Endian,
    max_alignment: usize,
}

impl<'a> FormatParser<'a> {
    pub(crate) fn new(format: &'a [u8]) -> Self {
        Self {
            format,
            cursor: 0,
            endian: Endian::native(),
            max_alignment: 1,
        }
    }

    pub(crate) fn next_item(&mut self, offset: usize) -> Result<Option<Item>, FormatError> {
        loop {
            let Some((kind, size)) = self.next_option()? else {
                return Ok(None);
            };

            if kind == ParsedKind::NoOp {
                continue;
            }

            if kind == ParsedKind::AlignmentPadding {
                let Some((next_kind, alignment)) = self.next_option()? else {
                    return Err(FormatError::InvalidNextOption);
                };

                if next_kind == ParsedKind::FixedString || alignment == 0 {
                    return Err(FormatError::InvalidNextOption);
                }

                return Ok(Some(Item {
                    kind: ItemKind::AlignmentPadding,
                    size: 0,
                    padding: padding_for(offset, alignment, self.max_alignment, false)?,
                    endian: self.endian,
                }));
            }

            return Ok(Some(Item {
                kind: kind
                    .into_item_kind()
                    .expect("non-control format option has an item kind"),
                size,
                padding: padding_for(
                    offset,
                    size,
                    self.max_alignment,
                    kind == ParsedKind::FixedString,
                )?,
                endian: self.endian,
            }));
        }
    }

    fn next_option(&mut self) -> Result<Option<(ParsedKind, usize)>, FormatError> {
        let Some(option) = self.next_byte() else {
            return Ok(None);
        };

        let parsed = match option {
            b'b' => (ParsedKind::SignedInteger, size_of::<i8>()),
            b'B' => (ParsedKind::UnsignedInteger, size_of::<u8>()),
            b'h' => (ParsedKind::SignedInteger, size_of::<c_short>()),
            b'H' => (ParsedKind::UnsignedInteger, size_of::<c_short>()),
            b'l' => (ParsedKind::SignedInteger, size_of::<c_long>()),
            b'L' => (ParsedKind::UnsignedInteger, size_of::<c_long>()),
            b'j' => (ParsedKind::SignedInteger, size_of::<i64>()),
            b'J' => (ParsedKind::UnsignedInteger, size_of::<i64>()),
            b'T' => (ParsedKind::UnsignedInteger, size_of::<usize>()),
            b'f' => (ParsedKind::Float, size_of::<f32>()),
            b'n' => (ParsedKind::LuaNumber, size_of::<f64>()),
            b'd' => (ParsedKind::Double, size_of::<f64>()),
            b'i' => (
                ParsedKind::SignedInteger,
                self.integer_size(size_of::<c_int>())?,
            ),
            b'I' => (
                ParsedKind::UnsignedInteger,
                self.integer_size(size_of::<c_int>())?,
            ),
            b's' => (
                ParsedKind::LengthString,
                self.integer_size(size_of::<usize>())?,
            ),
            b'c' => (
                ParsedKind::FixedString,
                self.number().ok_or(FormatError::MissingFixedStringSize)?,
            ),
            b'z' => (ParsedKind::ZeroString, 0),
            b'x' => (ParsedKind::Padding, 1),
            b'X' => (ParsedKind::AlignmentPadding, 0),
            b' ' => (ParsedKind::NoOp, 0),
            b'<' => {
                self.endian = Endian::Little;
                (ParsedKind::NoOp, 0)
            }
            b'>' => {
                self.endian = Endian::Big;
                (ParsedKind::NoOp, 0)
            }
            b'=' => {
                self.endian = Endian::native();
                (ParsedKind::NoOp, 0)
            }
            b'!' => {
                self.max_alignment = self.integer_size(native_max_alignment())?;
                (ParsedKind::NoOp, 0)
            }
            option => return Err(FormatError::InvalidOption(option)),
        };

        Ok(Some(parsed))
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = *self.format.get(self.cursor)?;
        if byte == 0 {
            self.cursor = self.format.len();
            return None;
        }

        self.cursor += 1;
        Some(byte)
    }

    fn integer_size(&mut self, default: usize) -> Result<usize, FormatError> {
        let size = self.number().unwrap_or(default);
        if !(1..=MAX_INTEGER_SIZE).contains(&size) {
            return Err(FormatError::IntegralSizeOutOfLimits(size));
        }

        Ok(size)
    }

    fn number(&mut self) -> Option<usize> {
        let first = self.format.get(self.cursor).copied()?;
        if !first.is_ascii_digit() {
            return None;
        }

        let mut number = 0usize;
        loop {
            let digit = usize::from(self.format[self.cursor] - b'0');
            number = number * 10 + digit;
            self.cursor += 1;

            let Some(next) = self.format.get(self.cursor).copied() else {
                break;
            };
            if !next.is_ascii_digit() || number > (MAX_TOTAL_SIZE - 9) / 10 {
                break;
            }
        }

        Some(number)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParsedKind {
    SignedInteger,
    UnsignedInteger,
    Float,
    Double,
    LuaNumber,
    FixedString,
    LengthString,
    ZeroString,
    Padding,
    AlignmentPadding,
    NoOp,
}

impl ParsedKind {
    fn into_item_kind(self) -> Option<ItemKind> {
        match self {
            Self::SignedInteger => Some(ItemKind::SignedInteger),
            Self::UnsignedInteger => Some(ItemKind::UnsignedInteger),
            Self::Float => Some(ItemKind::Float),
            Self::Double => Some(ItemKind::Double),
            Self::LuaNumber => Some(ItemKind::LuaNumber),
            Self::FixedString => Some(ItemKind::FixedString),
            Self::LengthString => Some(ItemKind::LengthString),
            Self::ZeroString => Some(ItemKind::ZeroString),
            Self::Padding => Some(ItemKind::Padding),
            Self::AlignmentPadding => Some(ItemKind::AlignmentPadding),
            Self::NoOp => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormatError {
    IntegralSizeOutOfLimits(usize),
    MissingFixedStringSize,
    InvalidOption(u8),
    InvalidNextOption,
    AlignmentNotPowerOfTwo,
    VariableLength,
    ResultTooLarge,
    IntegerDoesNotFit(usize),
}

impl FormatError {
    pub(crate) fn is_argument_error(self) -> bool {
        matches!(
            self,
            Self::InvalidNextOption
                | Self::AlignmentNotPowerOfTwo
                | Self::VariableLength
                | Self::ResultTooLarge
        )
    }

    pub(crate) fn message(self) -> String {
        match self {
            Self::IntegralSizeOutOfLimits(size) => {
                format!("integral size ({size}) out of limits [1,{MAX_INTEGER_SIZE}]")
            }
            Self::MissingFixedStringSize => "missing size for format option 'c'".into(),
            Self::InvalidOption(option) => {
                format!("invalid format option '{}'", char::from(option))
            }
            Self::InvalidNextOption => "invalid next option for option 'X'".into(),
            Self::AlignmentNotPowerOfTwo => "format asks for alignment not power of 2".into(),
            Self::VariableLength => "variable-length format".into(),
            Self::ResultTooLarge => "format result too large".into(),
            Self::IntegerDoesNotFit(size) => {
                format!("{size}-byte integer does not fit into Lua Integer")
            }
        }
    }
}

pub(crate) fn pack_size(format: &[u8]) -> Result<usize, FormatError> {
    let mut parser = FormatParser::new(format);
    let mut total = 0usize;

    while let Some(item) = parser.next_item(total)? {
        if matches!(item.kind, ItemKind::LengthString | ItemKind::ZeroString) {
            return Err(FormatError::VariableLength);
        }

        total = total
            .checked_add(item.padding)
            .and_then(|total| total.checked_add(item.size))
            .filter(|total| *total <= MAX_TOTAL_SIZE)
            .ok_or(FormatError::ResultTooLarge)?;
    }

    Ok(total)
}

fn padding_for(
    offset: usize,
    size: usize,
    max_alignment: usize,
    fixed_string: bool,
) -> Result<usize, FormatError> {
    if fixed_string {
        return Ok(0);
    }

    let alignment = size.min(max_alignment);
    if alignment <= 1 {
        return Ok(0);
    }
    if !alignment.is_power_of_two() {
        return Err(FormatError::AlignmentNotPowerOfTwo);
    }

    Ok((alignment - (offset & (alignment - 1))) & (alignment - 1))
}

fn native_max_alignment() -> usize {
    [
        align_of::<f64>(),
        align_of::<*const ()>(),
        align_of::<i64>(),
        align_of::<c_long>(),
    ]
    .into_iter()
    .max()
    .expect("native alignment list is non-empty")
}

pub(crate) fn integer_bytes(value: i64, size: usize, endian: Endian, signed: bool) -> Vec<u8> {
    let mut bytes = vec![0; size];
    let extension = if signed && value < 0 { 0xff } else { 0 };
    let bits = value as u64;

    for significance in 0..size {
        let byte = if significance < 8 {
            (bits >> (significance * 8)) as u8
        } else {
            extension
        };

        let destination = match endian {
            Endian::Little => significance,
            Endian::Big => size - 1 - significance,
        };
        bytes[destination] = byte;
    }

    bytes
}

pub(crate) fn float_bytes(value: f32, endian: Endian) -> [u8; size_of::<f32>()] {
    match endian {
        Endian::Little => value.to_le_bytes(),
        Endian::Big => value.to_be_bytes(),
    }
}

pub(crate) fn double_bytes(value: f64, endian: Endian) -> [u8; size_of::<f64>()] {
    match endian {
        Endian::Little => value.to_le_bytes(),
        Endian::Big => value.to_be_bytes(),
    }
}

pub(crate) fn read_integer(bytes: &[u8], endian: Endian, signed: bool) -> Result<i64, FormatError> {
    let size = bytes.len();
    let significant = size.min(8);
    let mut result = 0u64;

    for significance in 0..significant {
        let source = match endian {
            Endian::Little => significance,
            Endian::Big => size - 1 - significance,
        };

        result |= u64::from(bytes[source]) << (significance * 8);
    }

    if signed && size < 8 {
        let sign_bit = 1u64 << (size * 8 - 1);
        if result & sign_bit != 0 {
            result |= u64::MAX << (size * 8);
        }
    }

    if size > 8 {
        let extension = if signed && (result as i64) < 0 {
            0xff
        } else {
            0
        };

        for significance in 8..size {
            let source = match endian {
                Endian::Little => significance,
                Endian::Big => size - 1 - significance,
            };

            if bytes[source] != extension {
                return Err(FormatError::IntegerDoesNotFit(size));
            }
        }
    }

    Ok(result as i64)
}

pub(crate) fn check_integer_range(
    value: i64,
    size: usize,
    signed: bool,
) -> Result<(), &'static str> {
    if size >= 8 {
        return Ok(());
    }

    let bits = size * 8;

    if signed {
        let limit = 1i128 << (bits - 1);
        let value = i128::from(value);

        if !(-limit..limit).contains(&value) {
            return Err("integer overflow");
        }
    } else {
        let value = value as u64;
        let limit = 1u64 << bits;

        if value >= limit {
            return Err("unsigned overflow");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::ffi::{c_int, c_long, c_short};
    use std::mem::size_of;

    use super::{FormatError, pack_size};

    #[test]
    fn calculates_fixed_format_sizes() {
        let native_sizes = 2
            + 2 * size_of::<c_short>()
            + 2 * size_of::<c_long>()
            + 2 * size_of::<i64>()
            + size_of::<usize>()
            + size_of::<f32>()
            + size_of::<c_int>()
            + 2 * size_of::<f64>();
        assert_eq!(pack_size(b"bBhHlLjJTfind").unwrap(), native_sizes);
        assert_eq!(
            pack_size(b"iI i1 I2 i4 I8 i16").unwrap(),
            2 * size_of::<c_int>() + 31
        );
        assert_eq!(pack_size(b"c0c3xx").unwrap(), 5);
        assert_eq!(pack_size(b"<>==").unwrap(), 0);
    }

    #[test]
    fn applies_requested_alignment() {
        assert_eq!(pack_size(b"!8 b Xh i4").unwrap(), 8);
        assert_eq!(pack_size(b"!8 xXi8").unwrap(), 8);
        assert_eq!(pack_size(b"!2 xXi8").unwrap(), 2);
        assert_eq!(pack_size(b"!16 xXi16").unwrap(), 16);
    }

    #[test]
    fn rejects_variable_length_formats() {
        assert_eq!(pack_size(b"s"), Err(FormatError::VariableLength));
        assert_eq!(pack_size(b"z"), Err(FormatError::VariableLength));
    }

    #[test]
    fn validates_sizes_and_alignment_options() {
        assert_eq!(
            pack_size(b"i17"),
            Err(FormatError::IntegralSizeOutOfLimits(17))
        );
        assert_eq!(pack_size(b"c"), Err(FormatError::MissingFixedStringSize));
        assert_eq!(pack_size(b"X"), Err(FormatError::InvalidNextOption));
        assert_eq!(pack_size(b"Xc1"), Err(FormatError::InvalidNextOption));
        assert_eq!(
            pack_size(b"!3xi3"),
            Err(FormatError::AlignmentNotPowerOfTwo)
        );
    }
}
