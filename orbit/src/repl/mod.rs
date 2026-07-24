mod helper;
mod indent;

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use orbit_parser::{
    lexer::{LexErrorKind, TokenKind},
    parser::ParseErrorKind,
};
use orbit_vm::{CallOutcome, Function, LoadError, State, Table, Value, VmError, VmErrorKind};
use rustyline::{CompletionType, Config, Editor, error::ReadlineError, history::DefaultHistory};

use self::helper::ReplHelper;
use self::indent::{
    AutoDedent, configure_auto_dedent, normalize_closing_line, suggested_indentation,
};
use crate::diagnostics::{SharedSources, print_runtime_error};

type ReplEditor = Editor<ReplHelper, DefaultHistory>;

const REPL_NAME: &[u8] = b"<stdin>";
const PRIMARY_PROMPT: &str = "$ ";
const CONTINUATION_PROMPT: &str = "> ";

pub(crate) fn run(state: &mut State, sources: &SharedSources) -> ExitCode {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .history_ignore_space(true)
        .build();
    let mut editor = match ReplEditor::with_config(config) {
        Ok(editor) => editor,
        Err(error) => {
            eprintln!("failed to initialize line editor: {error}");
            return ExitCode::FAILURE;
        }
    };
    let auto_dedent = AutoDedent::default();
    configure_auto_dedent(&mut editor, &auto_dedent);
    editor.set_helper(Some(ReplHelper::new(auto_dedent.clone())));
    let history_path = initialize_history(&mut editor);

    let print = match state.get_global(b"print") {
        Ok(Value::Function(print)) => print,
        Ok(_) => {
            eprintln!("standard library did not install a print function");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            print_runtime_error(&error, &sources.borrow());
            return ExitCode::FAILURE;
        }
    };

    let mut source = String::new();
    let mut incomplete_error = None;

    loop {
        refresh_helper(&mut editor, state, &source);
        let prompt = if source.is_empty() {
            PRIMARY_PROMPT
        } else {
            CONTINUATION_PROMPT
        };
        let indentation = suggested_indentation(&source);
        auto_dedent.prepare(indentation.len());
        let line = if source.is_empty() {
            editor.readline(prompt)
        } else {
            editor.readline_with_initial(prompt, (&indentation, ""))
        };
        let line = match line {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => {
                source.clear();
                incomplete_error = None;
                continue;
            }
            Err(ReadlineError::Eof) => {
                if !source.is_empty() {
                    remember_history(&mut editor, history_path.as_deref(), &source);
                }
                if let Some(error) = incomplete_error {
                    print_runtime_error(&error, &sources.borrow());
                }
                break;
            }
            Err(error) => {
                eprintln!("failed to read input: {error}");
                return ExitCode::FAILURE;
            }
        };
        let line = normalize_closing_line(line, indentation.len());

        let first_line = source.is_empty();
        if !first_line {
            source.push('\n');
        }
        source.push_str(&line);

        let compilation = if first_line {
            compile_line(state, &source)
        } else {
            compile_statement(state, &source)
        };

        let function = match compilation {
            Compilation::Ready(function) => {
                incomplete_error = None;
                function
            }
            Compilation::Incomplete(error) => {
                incomplete_error = Some(error);
                continue;
            }
            Compilation::Invalid(error) => {
                incomplete_error = None;
                remember_history(&mut editor, history_path.as_deref(), &source);
                print_runtime_error(&error, &sources.borrow());
                source.clear();
                continue;
            }
        };

        remember_history(&mut editor, history_path.as_deref(), &source);
        source.clear();

        let values = match state.call(&function, &[]) {
            Ok(CallOutcome::Returned(values)) => values,
            Ok(CallOutcome::Yielded { .. }) => {
                eprintln!("interactive chunk unexpectedly yielded");
                continue;
            }
            Err(error) => {
                print_runtime_error(&error, &sources.borrow());
                continue;
            }
        };

        if values.is_empty() {
            continue;
        }

        match state.call(&print, &values) {
            Ok(CallOutcome::Returned(_)) => {}
            Ok(CallOutcome::Yielded { .. }) => {
                eprintln!("print unexpectedly yielded");
            }
            Err(error) => {
                print_runtime_error(&error, &sources.borrow());
            }
        }
    }

    ExitCode::SUCCESS
}

enum Compilation {
    Ready(Function),
    Incomplete(VmError),
    Invalid(VmError),
}

fn compile_line(state: &mut State, source: &str) -> Compilation {
    let expression = format!("return {source};");

    match state.load_buffer(REPL_NAME, expression.as_bytes()) {
        Ok(function) => Compilation::Ready(function),
        Err(_) => compile_statement(state, source),
    }
}

fn compile_statement(state: &mut State, source: &str) -> Compilation {
    match state.load_buffer(REPL_NAME, source.as_bytes()) {
        Ok(function) => Compilation::Ready(function),
        Err(error) if is_incomplete_input(&error, source.len()) => Compilation::Incomplete(error),
        Err(error) => Compilation::Invalid(error),
    }
}

fn is_incomplete_input(error: &VmError, source_len: usize) -> bool {
    let VmErrorKind::LoadFailure(error) = &error.kind else {
        return false;
    };

    match error {
        LoadError::Lex(error) => {
            let ends_at_eof = usize::try_from(error.span.end) == Ok(source_len);
            ends_at_eof
                && matches!(
                    error.kind,
                    LexErrorKind::UnterminatedString
                        | LexErrorKind::UnterminatedLongString
                        | LexErrorKind::UnterminatedLongComment
                )
        }
        LoadError::Parse(error) => {
            let found_eof = match error.kind {
                ParseErrorKind::ExpectedToken { actual, .. }
                | ParseErrorKind::ExpectedExpression { actual }
                | ParseErrorKind::ExpectedStatement { actual } => actual == Some(TokenKind::Eof),
                _ => false,
            };
            let error_at_eof = usize::try_from(error.span.start) == Ok(source_len);

            found_eof || (error_at_eof && matches!(error.kind, ParseErrorKind::ExpectedArguments))
        }
        _ => false,
    }
}

fn refresh_helper(editor: &mut ReplEditor, state: &mut State, source: &str) {
    let Some(helper) = editor.helper_mut() else {
        return;
    };

    helper.set_context(source);
    if let Ok(completions) = global_completions(state) {
        helper.set_completions(completions);
    }
}

fn global_completions(state: &mut State) -> Result<Vec<String>, VmError> {
    let globals = state.globals()?;
    let mut names = BTreeSet::new();
    let mut tables = Vec::new();
    let mut previous = Value::Nil;

    while let Some((key, value)) = state.next(&globals, &previous)? {
        if let Some(name) = identifier_key(&key) {
            names.insert(name.clone());
            if let Value::Table(table) = value {
                tables.push((name, table));
            }
        }
        previous = key;
    }

    for (prefix, table) in tables {
        collect_table_completions(state, &table, &prefix, &mut names)?;
    }

    Ok(names.into_iter().collect())
}

fn collect_table_completions(
    state: &mut State,
    table: &Table,
    prefix: &str,
    names: &mut BTreeSet<String>,
) -> Result<(), VmError> {
    let mut previous = Value::Nil;

    while let Some((key, _)) = state.next(table, &previous)? {
        if let Some(name) = identifier_key(&key) {
            names.insert(format!("{prefix}.{name}"));
        }
        previous = key;
    }

    Ok(())
}

fn identifier_key(value: &Value) -> Option<String> {
    let Value::String(value) = value else {
        return None;
    };
    let value = std::str::from_utf8(value.as_bytes()).ok()?;

    is_identifier(value).then(|| value.to_owned())
}

fn is_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn initialize_history(editor: &mut ReplEditor) -> Option<PathBuf> {
    let path = history_path()?;
    let parent = path.parent()?;

    if let Err(error) = fs::create_dir_all(parent) {
        eprintln!(
            "failed to create REPL history directory {}: {error}",
            parent.display()
        );
        return None;
    }

    match editor.load_history(&path) {
        Ok(()) => {}
        Err(ReadlineError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("failed to load REPL history {}: {error}", path.display());
        }
    }

    Some(path)
}

fn history_path() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(fallback_state_home)?;

    Some(state_home.join("orbit").join("history"))
}

#[cfg(unix)]
fn fallback_state_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".local").join("state"))
}

#[cfg(windows)]
fn fallback_state_home() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(any(unix, windows)))]
fn fallback_state_home() -> Option<PathBuf> {
    None
}

fn remember_history(editor: &mut ReplEditor, history_path: Option<&Path>, source: &str) {
    if source.trim().is_empty() {
        return;
    }

    match editor.add_history_entry(source) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("failed to add REPL history entry: {error}");
            return;
        }
    }

    let Some(path) = history_path else {
        return;
    };
    if let Err(error) = editor.append_history(path) {
        eprintln!("failed to save REPL history {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use orbit_loader::Loader;
    use orbit_vm::State;

    use super::{global_completions, is_identifier};

    #[test]
    fn completion_keys_must_be_lua_identifiers() {
        assert!(is_identifier("_VERSION"));
        assert!(is_identifier("package2"));
        assert!(!is_identifier(""));
        assert!(!is_identifier("2package"));
        assert!(!is_identifier("not-a-name"));
        assert!(!is_identifier("mötley"));
    }

    #[test]
    fn completions_include_live_globals_and_library_fields() {
        let mut state = State::new(Loader::new()).unwrap();
        orbit_stdlib::install(&mut state).unwrap();

        let completions = global_completions(&mut state).unwrap();

        assert!(completions.binary_search(&"print".to_owned()).is_ok());
        assert!(completions.binary_search(&"math.max".to_owned()).is_ok());
        assert!(
            completions
                .binary_search(&"string.format".to_owned())
                .is_ok()
        );
    }
}
