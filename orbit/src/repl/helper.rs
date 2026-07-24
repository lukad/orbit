use std::borrow::Cow;

use orbit_common::SourceId;
use orbit_parser::lexer::{LexErrorKind, Lexer, Token};
use rustyline::{
    Context, Helper,
    completion::{Completer, Pair},
    highlight::{CmdKind, Highlighter},
    hint::{Hinter, HistoryHinter},
    validate::Validator,
};

use super::indent::AutoDedent;

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

pub(super) struct ReplHelper {
    auto_dedent: AutoDedent,
    completions: Vec<String>,
    context: String,
    hinter: HistoryHinter,
}

impl ReplHelper {
    pub(super) fn new(auto_dedent: AutoDedent) -> Self {
        Self {
            auto_dedent,
            completions: KEYWORDS
                .iter()
                .map(|keyword| (*keyword).to_owned())
                .collect(),
            context: String::new(),
            hinter: HistoryHinter::new(),
        }
    }

    pub(super) fn set_context(&mut self, context: &str) {
        context.clone_into(&mut self.context);
    }

    pub(super) fn set_completions(&mut self, globals: impl IntoIterator<Item = String>) {
        self.completions.clear();
        self.completions
            .extend(KEYWORDS.iter().map(|keyword| (*keyword).to_owned()));
        self.completions.extend(globals);
        self.completions.sort_unstable();
        self.completions.dedup();
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _context: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        if let Some((start, replacement)) = self.auto_dedent.take_completion(line, pos) {
            return Ok((
                start,
                vec![Pair {
                    display: replacement.clone(),
                    replacement,
                }],
            ));
        }

        let start = completion_start(line, pos);
        let prefix = &line[start..pos];
        let candidates = self
            .completions
            .iter()
            .filter(|candidate| candidate.starts_with(prefix))
            .map(|candidate| Pair {
                display: candidate.clone(),
                replacement: candidate.clone(),
            })
            .collect();

        Ok((start, candidates))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, context: &Context<'_>) -> Option<Self::Hint> {
        self.hinter.hint(line, pos, context)
    }
}

impl Highlighter for ReplHelper {
    fn highlight<'line>(&self, line: &'line str, _pos: usize) -> Cow<'line, str> {
        if self.context.is_empty() {
            return highlight_lua(line);
        }

        let mut source = String::with_capacity(self.context.len() + line.len() + 1);
        source.push_str(&self.context);
        source.push('\n');
        let start = source.len();
        source.push_str(line);

        Cow::Owned(highlight_lua_range(&source, start))
    }

    fn highlight_prompt<'buffer, 'self_lifetime: 'buffer, 'prompt: 'buffer>(
        &'self_lifetime self,
        prompt: &'prompt str,
        _default: bool,
    ) -> Cow<'buffer, str> {
        Cow::Owned(format!("\x1b[1;34m{prompt}\x1b[0m"))
    }

    fn highlight_hint<'hint>(&self, hint: &'hint str) -> Cow<'hint, str> {
        Cow::Owned(format!("\x1b[2;90m{hint}\x1b[0m"))
    }

    fn highlight_candidate<'candidate>(
        &self,
        candidate: &'candidate str,
        _completion: rustyline::CompletionType,
    ) -> Cow<'candidate, str> {
        highlight_lua(candidate)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, kind: CmdKind) -> bool {
        kind != CmdKind::MoveCursor
    }
}

impl Validator for ReplHelper {}
impl Helper for ReplHelper {}

fn completion_start(line: &str, pos: usize) -> usize {
    line[..pos]
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !matches!(
                character,
                'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '.' | ':'
            )
        })
        .map_or(0, |(index, character)| index + character.len_utf8())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Keyword,
    Literal,
    Number,
    String,
    Comment,
    Operator,
    Error,
}

impl Style {
    fn ansi(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Keyword => "\x1b[1;35m",
            Self::Literal => "\x1b[1;33m",
            Self::Number => "\x1b[36m",
            Self::String => "\x1b[32m",
            Self::Comment => "\x1b[2;90m",
            Self::Operator => "\x1b[34m",
            Self::Error => "\x1b[31m",
        }
    }
}

fn highlight_lua(source: &str) -> Cow<'_, str> {
    if source.is_empty() {
        Cow::Borrowed(source)
    } else {
        Cow::Owned(highlight_lua_range(source, 0))
    }
}

fn highlight_lua_range(source: &str, start: usize) -> String {
    let mut styles = vec![Style::Plain; source.len()];
    if let Ok(lexer) = Lexer::new(SourceId::new(0), source) {
        for token in lexer {
            match token {
                Ok(token) => mark(
                    &mut styles,
                    token.span.start,
                    token.span.end,
                    token_style(&token.value),
                ),
                Err(error) => {
                    let style = match error.kind {
                        LexErrorKind::UnterminatedString | LexErrorKind::UnterminatedLongString => {
                            Style::String
                        }
                        LexErrorKind::UnterminatedLongComment => Style::Comment,
                        LexErrorKind::UnexpectedCharacter
                        | LexErrorKind::InvalidNumber
                        | LexErrorKind::InvalidEscapeSequence
                        | LexErrorKind::SourceTooLarge => Style::Error,
                    };
                    mark(&mut styles, error.span.start, error.span.end, style);
                }
            }
        }
    }
    mark_skipped_comments(source, &mut styles);

    render_styles(source, &styles, start)
}

fn token_style(token: &Token) -> Style {
    match token {
        Token::Name(_) | Token::Eof => Style::Plain,
        Token::Number(_) => Style::Number,
        Token::String(_) => Style::String,
        Token::False | Token::Nil | Token::True => Style::Literal,
        Token::And
        | Token::Break
        | Token::Do
        | Token::Else
        | Token::ElseIf
        | Token::End
        | Token::For
        | Token::Function
        | Token::Goto
        | Token::If
        | Token::In
        | Token::Local
        | Token::Not
        | Token::Or
        | Token::Repeat
        | Token::Return
        | Token::Then
        | Token::Until
        | Token::While => Style::Keyword,
        _ => Style::Operator,
    }
}

fn mark(styles: &mut [Style], start: u32, end: u32, style: Style) {
    let start = usize::try_from(start)
        .unwrap_or(usize::MAX)
        .min(styles.len());
    let end = usize::try_from(end).unwrap_or(usize::MAX).min(styles.len());
    styles[start.min(end)..start.max(end)].fill(style);
}

fn mark_skipped_comments(source: &str, styles: &mut [Style]) {
    let bytes = source.as_bytes();
    let mut index = 0;

    while index + 1 < bytes.len() {
        if bytes[index] != b'-'
            || bytes[index + 1] != b'-'
            || styles[index] != Style::Plain
            || styles[index + 1] != Style::Plain
        {
            index += 1;
            continue;
        }

        let end = long_comment_end(bytes, index + 2).unwrap_or_else(|| {
            bytes[index + 2..]
                .iter()
                .position(|byte| matches!(*byte, b'\r' | b'\n'))
                .map_or(bytes.len(), |offset| index + 2 + offset)
        });
        styles[index..end].fill(Style::Comment);
        index = end;
    }
}

fn long_comment_end(source: &[u8], open: usize) -> Option<usize> {
    if source.get(open) != Some(&b'[') {
        return None;
    }
    let level = source[open + 1..]
        .iter()
        .take_while(|byte| **byte == b'=')
        .count();
    let body = open + level + 2;
    if source.get(body - 1) != Some(&b'[') {
        return None;
    }

    let mut index = body;
    while index < source.len() {
        let close = index + level + 2;
        if source[index] == b']'
            && source
                .get(index + 1..index + level + 1)
                .is_some_and(|equals| equals.iter().all(|byte| *byte == b'='))
            && source.get(index + level + 1) == Some(&b']')
        {
            return Some(close);
        }
        index += 1;
    }

    Some(source.len())
}

fn render_styles(source: &str, styles: &[Style], start: usize) -> String {
    let mut rendered = String::with_capacity(source.len() - start);
    let mut active = Style::Plain;

    for (offset, character) in source[start..].char_indices() {
        let style = styles[start + offset];
        if style != active {
            if active != Style::Plain {
                rendered.push_str("\x1b[0m");
            }
            rendered.push_str(style.ansi());
            active = style;
        }
        rendered.push(character);
    }

    if active != Style::Plain {
        rendered.push_str("\x1b[0m");
    }

    rendered
}

#[cfg(test)]
mod tests {
    use rustyline::{
        Context, completion::Completer, highlight::Highlighter, history::DefaultHistory,
    };

    use super::{AutoDedent, ReplHelper, highlight_lua};

    #[test]
    fn highlights_lua_tokens_and_preserves_the_text() {
        let source = "local answer = 42 -- comment\nreturn \"value\", true";
        let highlighted = highlight_lua(source).into_owned();

        assert!(highlighted.contains("\x1b[1;35mlocal\x1b[0m"));
        assert!(highlighted.contains("\x1b[36m42\x1b[0m"));
        assert!(highlighted.contains("\x1b[2;90m-- comment\x1b[0m"));
        assert!(highlighted.contains("\x1b[32m\"value\"\x1b[0m"));
        assert_eq!(strip_ansi(&highlighted), source);
    }

    #[test]
    fn carries_long_string_highlighting_into_continuation_lines() {
        let mut helper = ReplHelper::new(AutoDedent::default());
        helper.set_context("message = [[first");

        let highlighted = helper.highlight("second]]", 0);

        assert_eq!(highlighted, "\x1b[32msecond]]\x1b[0m");
    }

    #[test]
    fn completes_keywords_globals_and_table_fields() {
        let mut helper = ReplHelper::new(AutoDedent::default());
        helper.set_completions(["print".to_owned(), "math.sqrt".to_owned()]);
        let history = DefaultHistory::new();
        let context = Context::new(&history);

        let (start, candidates) = helper.complete("value = math.sq", 15, &context).unwrap();
        assert_eq!(start, 8);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].replacement, "math.sqrt");

        let (_, candidates) = helper.complete("ret", 3, &context).unwrap();
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.replacement == "return")
        );
    }

    fn strip_ansi(value: &str) -> String {
        let mut stripped = String::new();
        let mut characters = value.chars().peekable();

        while let Some(character) = characters.next() {
            if character != '\x1b' {
                stripped.push(character);
                continue;
            }
            if characters.next() != Some('[') {
                continue;
            }
            for character in characters.by_ref() {
                if character == 'm' {
                    break;
                }
            }
        }

        stripped
    }
}
