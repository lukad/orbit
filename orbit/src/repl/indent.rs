use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use orbit_common::SourceId;
use orbit_parser::lexer::{LexErrorKind, Lexer, Token};
use rustyline::{
    Cmd, ConditionalEventHandler, Event, EventContext, EventHandler, InputMode, KeyCode, KeyEvent,
    RepeatCount,
};

use super::ReplEditor;

const INDENT: &str = "  ";
const CLOSING_CHARACTERS: [char; 3] = [')', ']', '}'];
const CLOSING_KEYWORDS: [&str; 4] = ["else", "elseif", "end", "until"];

#[derive(Clone, Default)]
pub(super) struct AutoDedent {
    state: Arc<Mutex<AutoDedentState>>,
}

#[derive(Default)]
struct AutoDedentState {
    suggested: usize,
    pending: Option<PendingEdit>,
}

struct PendingEdit {
    line: Box<str>,
    position: usize,
    replacement: String,
}

impl AutoDedent {
    pub(super) fn prepare(&self, suggested: usize) {
        let mut state = self.lock();
        state.suggested = suggested;
        state.pending = None;
    }

    pub(super) fn take_completion(&self, line: &str, position: usize) -> Option<(usize, String)> {
        let pending = self.lock().pending.take()?;
        if pending.line.as_ref() != line || pending.position != position {
            return None;
        }

        Some((0, pending.replacement))
    }

    fn lock(&self) -> MutexGuard<'_, AutoDedentState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) fn configure_auto_dedent(editor: &mut ReplEditor, auto_dedent: &AutoDedent) {
    editor.bind_sequence(
        Event::Any,
        EventHandler::Conditional(Box::new(AutoDedentHandler {
            auto_dedent: auto_dedent.clone(),
        })),
    );
}

struct AutoDedentHandler {
    auto_dedent: AutoDedent,
}

impl ConditionalEventHandler for AutoDedentHandler {
    fn handle(
        &self,
        event: &Event,
        repeat: RepeatCount,
        _positive: bool,
        context: &EventContext,
    ) -> Option<Cmd> {
        if repeat != 1 || context.input_mode() != InputMode::Insert {
            return None;
        }
        let KeyEvent(KeyCode::Char(character), modifiers) = *event.get(0)? else {
            return None;
        };
        if !modifiers.is_empty() {
            return None;
        }

        let mut state = self.auto_dedent.lock();
        let replacement =
            dedented_replacement(context.line(), context.pos(), state.suggested, character)
                .or_else(|| {
                    reindented_identifier(context.line(), context.pos(), state.suggested, character)
                })?;
        state.pending = Some(PendingEdit {
            line: context.line().into(),
            position: context.pos(),
            replacement,
        });

        // Completion is the only stable Rustyline extension point that can
        // replace a range and leave the cursor after the replacement.
        Some(Cmd::Complete)
    }
}

pub(super) fn suggested_indentation(source: &str) -> String {
    let Ok(lexer) = Lexer::new(SourceId::new(0), source) else {
        return String::new();
    };
    let mut depth = 0_usize;

    for token in lexer {
        match token {
            Ok(token) => match token.value {
                Token::Function
                | Token::Do
                | Token::Then
                | Token::Repeat
                | Token::LeftParen
                | Token::LeftBrace
                | Token::LeftBracket => depth += 1,
                Token::ElseIf
                | Token::End
                | Token::Until
                | Token::RightParen
                | Token::RightBrace
                | Token::RightBracket => depth = depth.saturating_sub(1),
                _ => {}
            },
            Err(error)
                if usize::try_from(error.span.end) == Ok(source.len())
                    && matches!(
                        error.kind,
                        LexErrorKind::UnterminatedString
                            | LexErrorKind::UnterminatedLongString
                            | LexErrorKind::UnterminatedLongComment
                    ) =>
            {
                return String::new();
            }
            Err(_) => {}
        }
    }

    INDENT.repeat(depth)
}

pub(super) fn normalize_closing_line(mut line: String, suggested: usize) -> String {
    let leading_spaces = line.bytes().take_while(|byte| *byte == b' ').count();
    if leading_spaces != suggested || suggested < INDENT.len() {
        return line;
    }

    let trimmed = &line[leading_spaces..];
    if starts_closing_token(trimmed) {
        line.replace_range(..INDENT.len(), "");
    }

    line
}

fn starts_closing_token(line: &str) -> bool {
    CLOSING_CHARACTERS
        .iter()
        .any(|character| line.starts_with(*character))
        || CLOSING_KEYWORDS
            .iter()
            .any(|keyword| starts_keyword(line, keyword))
}

fn dedented_replacement(
    line: &str,
    position: usize,
    suggested: usize,
    character: char,
) -> Option<String> {
    let (before, after) = line.split_at_checked(position)?;
    let leading_spaces = before.bytes().take_while(|byte| *byte == b' ').count();
    if suggested < INDENT.len() || leading_spaces != suggested {
        return None;
    }

    let prefix = &before[leading_spaces..];
    let completes_closer = match character {
        'd' => prefix == "en",
        'e' => prefix == "els",
        'f' => prefix == "elsei",
        'l' => prefix == "unti",
        character if CLOSING_CHARACTERS.contains(&character) => prefix
            .chars()
            .all(|character| CLOSING_CHARACTERS.contains(&character)),
        _ => false,
    };
    if !completes_closer || !starts_at_token_boundary(after) {
        return None;
    }

    let mut replacement = before[INDENT.len()..].to_owned();
    replacement.push(character);
    Some(replacement)
}

fn reindented_identifier(
    line: &str,
    position: usize,
    suggested: usize,
    character: char,
) -> Option<String> {
    if character != '_' && !character.is_ascii_alphanumeric() {
        return None;
    }

    let (before, _) = line.split_at_checked(position)?;
    let leading_spaces = before.bytes().take_while(|byte| *byte == b' ').count();
    if leading_spaces + INDENT.len() != suggested {
        return None;
    }

    let prefix = &before[leading_spaces..];
    if prefix.is_empty()
        || !CLOSING_KEYWORDS
            .iter()
            .any(|keyword| keyword.starts_with(prefix))
    {
        return None;
    }

    let mut candidate = prefix.to_owned();
    candidate.push(character);
    if CLOSING_KEYWORDS
        .iter()
        .any(|keyword| keyword.starts_with(&candidate))
    {
        return None;
    }

    let mut replacement = String::with_capacity(INDENT.len() + before.len() + character.len_utf8());
    replacement.push_str(INDENT);
    replacement.push_str(before);
    replacement.push(character);
    Some(replacement)
}

fn starts_at_token_boundary(source: &str) -> bool {
    source
        .bytes()
        .next()
        .is_none_or(|byte| byte != b'_' && !byte.is_ascii_alphanumeric())
}

fn starts_keyword(line: &str, keyword: &str) -> bool {
    let Some(remainder) = line.strip_prefix(keyword) else {
        return false;
    };

    starts_at_token_boundary(remainder)
}

#[cfg(test)]
mod tests {
    use super::{
        dedented_replacement, normalize_closing_line, reindented_identifier, suggested_indentation,
    };

    #[test]
    fn indents_block_bodies_and_nested_blocks() {
        assert_eq!(suggested_indentation("function greet(name)"), "  ");
        assert_eq!(
            suggested_indentation("function greet(name)\n  if name then"),
            "    "
        );
        assert_eq!(
            suggested_indentation("function greet(name)\n  if name then\n  end"),
            "  "
        );
        assert_eq!(
            suggested_indentation("if ready then\nelse\n  print('waiting')"),
            "  "
        );
    }

    #[test]
    fn indents_unclosed_delimiters() {
        assert_eq!(suggested_indentation("values = {\n  one,"), "  ");
        assert_eq!(
            suggested_indentation("call(\n  function()\n    return true\n  end"),
            "  "
        );
    }

    #[test]
    fn does_not_indent_inside_strings_or_long_comments() {
        assert_eq!(suggested_indentation("message = [["), "");
        assert_eq!(suggested_indentation("message = \"continued\\"), "");
        assert_eq!(suggested_indentation("--[=["), "");
    }

    #[test]
    fn dedents_closing_lines_from_the_suggested_level() {
        assert_eq!(normalize_closing_line("    end".to_owned(), 4), "  end");
        assert_eq!(normalize_closing_line("  else".to_owned(), 2), "else");
        assert_eq!(
            normalize_closing_line("  until done".to_owned(), 2),
            "until done"
        );
        assert_eq!(
            normalize_closing_line("  endless".to_owned(), 2),
            "  endless"
        );
        assert_eq!(normalize_closing_line("end".to_owned(), 2), "end");
    }

    #[test]
    fn prepares_cursor_safe_completions_for_closing_tokens() {
        assert_eq!(
            dedented_replacement("    en", 6, 4, 'd'),
            Some("  end".to_owned())
        );
        assert_eq!(
            dedented_replacement("    els", 7, 4, 'e'),
            Some("  else".to_owned())
        );
        assert_eq!(
            dedented_replacement("    elsei", 9, 4, 'f'),
            Some("  elseif".to_owned())
        );
        assert_eq!(
            dedented_replacement("  unti condition", 6, 2, 'l'),
            Some("until".to_owned())
        );
        assert_eq!(
            dedented_replacement("    ", 4, 4, '}'),
            Some("  }".to_owned())
        );
    }

    #[test]
    fn only_dedents_from_the_prepared_indentation() {
        assert_eq!(dedented_replacement("  elsei", 7, 4, 'f'), None);
        assert_eq!(dedented_replacement("    ename", 6, 4, 'd'), None);
        assert_eq!(dedented_replacement("en", 2, 0, 'd'), None);
    }

    #[test]
    fn restores_indentation_when_a_keyword_becomes_an_identifier() {
        assert_eq!(
            reindented_identifier("  end", 5, 4, 'p'),
            Some("    endp".to_owned())
        );
        assert_eq!(
            reindented_identifier("  else", 6, 4, 'i'),
            None,
            "`elsei` can still become `elseif`"
        );
        assert_eq!(
            reindented_identifier("  elsei", 7, 4, 'n'),
            Some("    elsein".to_owned())
        );
        assert_eq!(
            reindented_identifier("  elseif", 8, 4, 'x'),
            Some("    elseifx".to_owned())
        );
    }
}
