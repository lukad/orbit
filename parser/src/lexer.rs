use std::iter::FusedIterator;

use logos::{Lexer as LogosLexer, Logos};
use orbit_common::{SourceId, Span, Spanned};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol(Box<str>);

impl Symbol {
    pub fn new(value: impl Into<Box<str>>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0.into()
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for Symbol {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Symbol {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ByteString(Box<[u8]>);

impl ByteString {
    pub fn new(value: impl Into<Box<[u8]>>) -> Self {
        Self(value.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.0.into()
    }
}

impl AsRef<[u8]> for ByteString {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<&[u8]> for ByteString {
    fn from(value: &[u8]) -> Self {
        Self::new(value)
    }
}

impl From<Vec<u8>> for ByteString {
    fn from(value: Vec<u8>) -> Self {
        Self::new(value)
    }
}

#[derive(Logos, Debug, Clone, PartialEq, strum::EnumDiscriminants)]
#[strum_discriminants(name(TokenKind))]
#[logos(error = LexErrorKind)]
#[logos(skip r"[ \t\r\n\x0B\x0C]+")]
#[logos(skip(r"--", skip_comment))]
pub enum Token {
    #[regex(r"[A-Za-z_][A-Za-z0-9_]*", |lexer| Symbol::from(lexer.slice()))]
    Name(Symbol),

    #[regex(r"[0-9]+", lex_integer)]
    #[regex(r"0[xX][0-9A-Fa-f]+", lex_integer)]
    Integer(i64),

    #[regex(r"[0-9]+\.[0-9]+(?:[eE][+-]?[0-9]+)?", lex_decimal_float)]
    #[regex(r"\.[0-9]+(?:[eE][+-]?[0-9]+)?", lex_decimal_float)]
    #[regex(r"[0-9]+[eE][+-]?[0-9]+", lex_decimal_float)]
    #[regex(
        r"0[xX](?:[0-9A-Fa-f]+\.[0-9A-Fa-f]*|\.[0-9A-Fa-f]+|[0-9A-Fa-f]+)[pP][+-]?[0-9]+",
        lex_hex_float
    )]
    Float(f64),

    #[token("\"", lex_short_string)]
    #[token("'", lex_short_string)]
    #[regex(r"\[=*\[", lex_long_string)]
    String(ByteString),

    #[token("and")]
    And,
    #[token("break")]
    Break,
    #[token("do")]
    Do,
    #[token("else")]
    Else,
    #[token("elseif")]
    ElseIf,
    #[token("false")]
    False,
    #[token("for")]
    For,
    #[token("local")]
    Local,
    #[token("function")]
    Function,
    #[token("goto")]
    Goto,
    #[token("if")]
    If,
    #[token("in")]
    In,
    #[token("nil")]
    Nil,
    #[token("not")]
    Not,
    #[token("or")]
    Or,
    #[token("repeat")]
    Repeat,
    #[token("return")]
    Return,
    #[token("then")]
    Then,
    #[token("true")]
    True,
    #[token("until")]
    Until,
    #[token("while")]
    While,
    #[token("end")]
    End,

    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("//")]
    SlashSlash,
    #[token("%")]
    Percent,
    #[token("^")]
    Caret,
    #[token("#")]
    Hash,
    #[token("&")]
    Ampersand,
    #[token("~")]
    Tilde,
    #[token("|")]
    Pipe,
    #[token("<<")]
    ShiftLeft,
    #[token(">>")]
    ShiftRight,
    #[token("..")]
    DotDot,
    #[token("<")]
    Less,
    #[token("<=")]
    LessEqual,
    #[token(">")]
    Greater,
    #[token(">=")]
    GreaterEqual,
    #[token("==")]
    EqualEqual,
    #[token("~=")]
    TildeEqual,
    #[token("=")]
    Equal,

    #[token("(")]
    LeftParen,
    #[token(")")]
    RightParen,
    #[token("{")]
    LeftBrace,
    #[token("}")]
    RightBrace,
    #[token("[")]
    LeftBracket,
    #[token("]")]
    RightBracket,
    #[token("::")]
    DoubleColon,
    #[token(";")]
    Semicolon,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token("...")]
    Ellipsis,
    #[token(".")]
    Dot,

    Eof,
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
#[error("{kind}")]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
}

#[derive(Debug, thiserror::Error, Default, Clone, Copy, PartialEq, Eq)]
pub enum LexErrorKind {
    #[default]
    #[error("unexpected character")]
    UnexpectedCharacter,
    #[error("invalid number")]
    InvalidNumber,
    #[error("invalid escape sequence")]
    InvalidEscapeSequence,
    #[error("unterminated string")]
    UnterminatedString,
    #[error("unterminated long string")]
    UnterminatedLongString,
    #[error("unterminated long comment")]
    UnterminatedLongComment,
    #[error("source exceeds 4 GiB")]
    SourceTooLarge,
}

pub type LexResult<T> = Result<T, LexError>;

pub struct Lexer<'source> {
    source_id: SourceId,
    source_len: u32,
    inner: LogosLexer<'source, Token>,
    emitted_eof: bool,
}

impl<'source> Lexer<'source> {
    pub fn new(source_id: SourceId, source: &'source str) -> LexResult<Self> {
        let source_len = u32::try_from(source.len()).map_err(|_| LexError {
            kind: LexErrorKind::SourceTooLarge,
            span: Span::new(source_id, 0, 0),
        })?;

        Ok(Self {
            source_id,
            source_len,
            inner: Token::lexer(source),
            emitted_eof: false,
        })
    }

    pub fn source(&self) -> &'source str {
        self.inner.source()
    }

    pub const fn source_id(&self) -> SourceId {
        self.source_id
    }

    fn span(&self) -> Span {
        let inner_span = self.inner.span();

        Span {
            source: self.source_id,
            start: inner_span.start as u32,
            end: inner_span.end as u32,
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = LexResult<Spanned<Token>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            Some(Ok(token)) => Some(Ok(Spanned {
                value: token,
                span: self.span(),
            })),
            Some(Err(kind)) => Some(Err(LexError {
                kind,
                span: self.span(),
            })),
            None if !self.emitted_eof => {
                self.emitted_eof = true;
                Some(Ok(Spanned {
                    value: Token::Eof,
                    span: Span {
                        source: self.source_id,
                        start: self.source_len,
                        end: self.source_len,
                    },
                }))
            }
            None => None,
        }
    }
}

impl FusedIterator for Lexer<'_> {}

pub fn lex(source_id: SourceId, source: &str) -> LexResult<Vec<Spanned<Token>>> {
    Lexer::new(source_id, source)?.collect()
}

fn lex_integer(lexer: &mut LogosLexer<'_, Token>) -> Result<i64, LexErrorKind> {
    let literal = lexer.slice();

    if let Some(digits) = literal
        .strip_prefix("0x")
        .or_else(|| literal.strip_prefix("0X"))
    {
        u64::from_str_radix(digits, 16)
            .map(|value| value as i64)
            .map_err(|_| LexErrorKind::InvalidNumber)
    } else {
        literal.parse().map_err(|_| LexErrorKind::InvalidNumber)
    }
}

fn lex_decimal_float(lexer: &mut LogosLexer<'_, Token>) -> Result<f64, LexErrorKind> {
    lexer
        .slice()
        .parse()
        .map_err(|_| LexErrorKind::InvalidNumber)
}

fn lex_hex_float(lexer: &mut LogosLexer<'_, Token>) -> Result<f64, LexErrorKind> {
    parse_hex_float(lexer.slice())
}

fn parse_hex_float(literal: &str) -> Result<f64, LexErrorKind> {
    let literal = &literal[2..];
    let (significand, exponent) = literal
        .split_once(['p', 'P'])
        .map_or((literal, "0"), |parts| parts);
    let exponent = exponent
        .parse::<i32>()
        .map_err(|_| LexErrorKind::InvalidNumber)?;
    let mut value = 0.0;
    let mut fractional = false;
    let mut place = 1.0 / 16.0;

    for byte in significand.bytes() {
        if byte == b'.' {
            fractional = true;
            continue;
        }

        let digit = hex_value(byte).ok_or(LexErrorKind::InvalidNumber)? as f64;
        if fractional {
            value += digit * place;
            place /= 16.0;
        } else {
            value = value * 16.0 + digit;
        }
    }

    Ok(value * 2.0_f64.powi(exponent))
}

fn lex_short_string(lexer: &mut LogosLexer<'_, Token>) -> Result<ByteString, LexErrorKind> {
    let quote = lexer.slice().as_bytes()[0];
    let remainder = lexer.remainder().as_bytes();
    let mut value = Vec::new();
    let mut index = 0;
    let mut error = None;

    while index < remainder.len() {
        match remainder[index] {
            byte if byte == quote => {
                lexer.bump(index + 1);
                return match error {
                    Some(error) => Err(error),
                    None => Ok(ByteString::from(value)),
                };
            }
            b'\r' | b'\n' => {
                lexer.bump(index);
                return Err(LexErrorKind::UnterminatedString);
            }
            b'\\' => {
                let (consumed, decoded, escape_error) =
                    decode_escape(&remainder[index + 1..], quote);
                index += consumed + 1;
                if let Some(decoded) = decoded {
                    value.extend_from_slice(&decoded);
                }
                if error.is_none() {
                    error = escape_error;
                }
            }
            byte => {
                value.push(byte);
                index += 1;
            }
        }
    }

    lexer.bump(remainder.len());
    Err(LexErrorKind::UnterminatedString)
}

fn decode_escape(source: &[u8], quote: u8) -> (usize, Option<Vec<u8>>, Option<LexErrorKind>) {
    let Some(&escape) = source.first() else {
        return (0, None, Some(LexErrorKind::UnterminatedString));
    };

    let decoded = match escape {
        b'a' => b'\x07',
        b'b' => b'\x08',
        b'f' => b'\x0C',
        b'n' => b'\n',
        b'r' => b'\r',
        b't' => b'\t',
        b'v' => b'\x0B',
        b'\\' | b'\'' | b'"' => escape,
        b'\n' => return (1, Some(vec![b'\n']), None),
        b'\r' => {
            let consumed = if source.get(1) == Some(&b'\n') { 2 } else { 1 };
            return (consumed, Some(vec![b'\n']), None);
        }
        b'z' => {
            let whitespace = take_while(&source[1..], u8::is_ascii_whitespace);
            return (whitespace + 1, Some(Vec::new()), None);
        }
        b'x' => {
            if source.len() >= 3 && source[1].is_ascii_hexdigit() && source[2].is_ascii_hexdigit() {
                let value = hex_value(source[1]).unwrap() * 16 + hex_value(source[2]).unwrap();
                return (3, Some(vec![value]), None);
            }

            let digits = source[1..]
                .iter()
                .take(2)
                .take_while(|byte| byte.is_ascii_hexdigit())
                .count();
            return (digits + 1, None, Some(LexErrorKind::InvalidEscapeSequence));
        }
        b'u' => return decode_unicode_escape(source, quote),
        digit if digit.is_ascii_digit() => {
            let digits = take_while(&source[..source.len().min(3)], u8::is_ascii_digit);
            let value = source[..digits]
                .iter()
                .fold(0_u16, |value, digit| value * 10 + u16::from(digit - b'0'));

            if value <= u8::MAX.into() {
                return (digits, Some(vec![value as u8]), None);
            }

            return (digits, None, Some(LexErrorKind::InvalidEscapeSequence));
        }
        _ => return (1, None, Some(LexErrorKind::InvalidEscapeSequence)),
    };

    (1, Some(vec![decoded]), None)
}

fn decode_unicode_escape(
    source: &[u8],
    quote: u8,
) -> (usize, Option<Vec<u8>>, Option<LexErrorKind>) {
    if source.get(1) != Some(&b'{') {
        return (1, None, Some(LexErrorKind::InvalidEscapeSequence));
    }

    let Some(terminator) = source[2..]
        .iter()
        .position(|byte| matches!(*byte, b'}' | b'\r' | b'\n') || *byte == quote)
    else {
        return (
            source.len(),
            None,
            Some(LexErrorKind::InvalidEscapeSequence),
        );
    };
    let close = terminator + 2;
    if source[close] != b'}' {
        return (close, None, Some(LexErrorKind::InvalidEscapeSequence));
    }
    let digits = &source[2..close];

    if digits.is_empty() || !digits.iter().all(u8::is_ascii_hexdigit) {
        return (close + 1, None, Some(LexErrorKind::InvalidEscapeSequence));
    }

    let Ok(value) = u32::from_str_radix(std::str::from_utf8(digits).unwrap(), 16) else {
        return (close + 1, None, Some(LexErrorKind::InvalidEscapeSequence));
    };
    let Some(character) = char::from_u32(value) else {
        return (close + 1, None, Some(LexErrorKind::InvalidEscapeSequence));
    };
    let mut utf8 = [0; 4];

    (
        close + 1,
        Some(character.encode_utf8(&mut utf8).as_bytes().to_vec()),
        None,
    )
}

fn lex_long_string(lexer: &mut LogosLexer<'_, Token>) -> Result<ByteString, LexErrorKind> {
    let level = lexer.slice().len() - 2;
    let remainder = lexer.remainder().as_bytes();
    let Some((content_end, consumed)) = find_long_bracket_close(remainder, level) else {
        lexer.bump(remainder.len());
        return Err(LexErrorKind::UnterminatedLongString);
    };
    let content = strip_initial_newline(&remainder[..content_end]);
    let value = normalize_newlines(content);
    lexer.bump(consumed);

    Ok(ByteString::from(value))
}

fn skip_comment(lexer: &mut LogosLexer<'_, Token>) -> Result<(), LexErrorKind> {
    let remainder = lexer.remainder().as_bytes();

    if let Some((level, opening_length)) = long_bracket_open(remainder) {
        let body = &remainder[opening_length..];
        let Some((_, consumed)) = find_long_bracket_close(body, level) else {
            lexer.bump(remainder.len());
            return Err(LexErrorKind::UnterminatedLongComment);
        };

        lexer.bump(opening_length + consumed);
    } else {
        let line_length = remainder
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .unwrap_or(remainder.len());
        lexer.bump(line_length);
    }

    Ok(())
}

fn long_bracket_open(source: &[u8]) -> Option<(usize, usize)> {
    if source.first() != Some(&b'[') {
        return None;
    }

    let level = source[1..].iter().take_while(|byte| **byte == b'=').count();
    let closing_open = level + 1;

    (source.get(closing_open) == Some(&b'[')).then_some((level, closing_open + 1))
}

fn find_long_bracket_close(source: &[u8], level: usize) -> Option<(usize, usize)> {
    let mut index = 0;

    while index < source.len() {
        let equals_end = index + level + 1;
        if source[index] == b']'
            && source
                .get(index + 1..equals_end)
                .is_some_and(|equals| equals.iter().all(|byte| *byte == b'='))
            && source.get(equals_end) == Some(&b']')
        {
            return Some((index, equals_end + 1));
        }

        index += 1;
    }

    None
}

fn strip_initial_newline(source: &[u8]) -> &[u8] {
    match source {
        [b'\r', b'\n', remainder @ ..] | [b'\n', b'\r', remainder @ ..] => remainder,
        [b'\r' | b'\n', remainder @ ..] => remainder,
        _ => source,
    }
}

fn normalize_newlines(source: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(source.len());
    let mut index = 0;

    while index < source.len() {
        if matches!(source[index], b'\r' | b'\n') {
            let newline = source[index];
            normalized.push(b'\n');
            index += 1;

            if matches!(source.get(index), Some(b'\r' | b'\n')) && source[index] != newline {
                index += 1;
            }
        } else {
            normalized.push(source[index]);
            index += 1;
        }
    }

    normalized
}

fn take_while(source: &[u8], predicate: fn(&u8) -> bool) -> usize {
    source.iter().take_while(|byte| predicate(byte)).count()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE_ID: SourceId = SourceId::new(7);

    fn tokens(source: &str) -> Vec<Token> {
        lex(SOURCE_ID, source)
            .expect("source should lex")
            .into_iter()
            .map(|token| token.value)
            .collect()
    }

    #[test]
    fn lexes_the_vertical_slice() {
        assert_eq!(
            tokens("local function add(a) return a + 2 * (3 - 1) / 4 end"),
            vec![
                Token::Local,
                Token::Function,
                Token::Name(Symbol::from("add")),
                Token::LeftParen,
                Token::Name(Symbol::from("a")),
                Token::RightParen,
                Token::Return,
                Token::Name(Symbol::from("a")),
                Token::Plus,
                Token::Integer(2),
                Token::Star,
                Token::LeftParen,
                Token::Integer(3),
                Token::Minus,
                Token::Integer(1),
                Token::RightParen,
                Token::Slash,
                Token::Integer(4),
                Token::End,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn keeps_keyword_prefixes_as_names() {
        assert_eq!(
            tokens("locality function2 returning ending"),
            vec![
                Token::Name(Symbol::from("locality")),
                Token::Name(Symbol::from("function2")),
                Token::Name(Symbol::from("returning")),
                Token::Name(Symbol::from("ending")),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lexes_lua_numbers_and_concatenation() {
        assert_eq!(
            tokens("0 42 3.5 .25 1e3 0x2a 0x1.8p1 1..2"),
            vec![
                Token::Integer(0),
                Token::Integer(42),
                Token::Float(3.5),
                Token::Float(0.25),
                Token::Float(1000.0),
                Token::Integer(42),
                Token::Float(3.0),
                Token::Integer(1),
                Token::DotDot,
                Token::Integer(2),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn decodes_short_and_long_strings() {
        assert_eq!(
            tokens("'a\\n\\x62\\099' [=[\r\nlong\r\nstring]=]"),
            vec![
                Token::String(ByteString::from(&b"a\nbc"[..])),
                Token::String(ByteString::from(&b"long\nstring"[..])),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn skips_line_and_long_comments() {
        assert_eq!(
            tokens("local -- line\n--[=[ long\ncomment ]=]\nreturn"),
            vec![Token::Local, Token::Return, Token::Eof]
        );
    }

    #[test]
    fn attaches_source_spans_and_includes_one_eof() {
        let tokens = lex(SOURCE_ID, "return x").unwrap();
        let return_token = &tokens[0];
        let name = &tokens[1];
        let eof = &tokens[2];

        assert_eq!(return_token.span, span(0, 6));
        assert_eq!(name.span, span(7, 8));
        assert_eq!(eof.value, Token::Eof);
        assert_eq!(eof.span, span(8, 8));
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn reports_lexing_errors_with_spans() {
        let invalid_number = "9999999999999999999999999";
        let invalid_number_error = lex(SOURCE_ID, invalid_number).unwrap_err();
        assert_eq!(invalid_number_error.kind, LexErrorKind::InvalidNumber);
        assert_eq!(
            invalid_number_error.span,
            span(0, invalid_number.len() as u32)
        );

        let unterminated = "'unterminated";
        let unterminated_error = lex(SOURCE_ID, unterminated).unwrap_err();
        assert_eq!(unterminated_error.kind, LexErrorKind::UnterminatedString);
        assert_eq!(unterminated_error.span, span(0, unterminated.len() as u32));
    }

    #[test]
    fn reports_a_bad_string_as_one_error() {
        let error = lex(SOURCE_ID, r#""bad\q" return"#).unwrap_err();
        assert_eq!(
            error,
            LexError {
                kind: LexErrorKind::InvalidEscapeSequence,
                span: span(0, 7),
            }
        );
    }

    #[test]
    fn unicode_escapes_decode_and_report_the_whole_bad_string() {
        assert_eq!(
            tokens(r#""snowman: \u{2603}""#),
            vec![
                Token::String(ByteString::from("snowman: ☃".as_bytes())),
                Token::Eof,
            ]
        );

        let error = lex(SOURCE_ID, r#""bad \u{12" return"#).unwrap_err();
        assert_eq!(error.kind, LexErrorKind::InvalidEscapeSequence);
        assert_eq!(error.span, span(0, 11));
    }

    fn span(start: u32, end: u32) -> Span {
        Span {
            source: SOURCE_ID,
            start,
            end,
        }
    }
}
