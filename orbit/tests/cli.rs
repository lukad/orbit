use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

struct Script(PathBuf);

impl Script {
    fn new(source: &str) -> Self {
        Self::new_bytes(source.as_bytes())
    }

    fn new_bytes(source: &[u8]) -> Self {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("orbit-cli-test-{}-{id}.lua", std::process::id()));
        fs::write(&path, source).unwrap();
        Self(path)
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("orbit-cli-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_repl(input: &str) -> Output {
    let state_home = TestDirectory::new();
    run_repl_with_state_home(input, state_home.path())
}

fn run_repl_with_state_home(input: &str, state_home: &Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_orbit"))
        .env("XDG_STATE_HOME", state_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();

    child.wait_with_output().unwrap()
}

#[test]
fn repl_saves_history_under_xdg_state_home() {
    let state_home = TestDirectory::new();
    let output = run_repl_with_state_home("answer = 42\n", state_home.path());

    assert!(output.status.success());
    let history = fs::read_to_string(state_home.path().join("orbit").join("history")).unwrap();
    assert!(history.contains("answer = 42"));
}

#[test]
fn no_filename_starts_a_repl_that_evaluates_expressions() {
    let output = run_repl("answer = 40\nanswer + 2\n");

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("42\n"));
    assert!(output.stderr.is_empty());
}

#[test]
fn repl_collects_multiline_blocks_and_long_strings() {
    let output =
        run_repl("message = [[hello\nfrom a long string]]\nif true then\nprint(message)\nend\n");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("hello\nfrom a long string\n"));
    assert!(output.stderr.is_empty());
}

#[test]
fn repl_tracebacks_name_declared_functions_and_main_chunks() {
    let output =
        run_repl("function divide(left, right)\nreturn left / right\nend\ndivide(1, \"two\")\n");

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("in function 'divide'"), "{stderr}");
    assert!(stderr.contains("in main chunk"), "{stderr}");
}

#[test]
fn repl_continues_an_unterminated_string_with_an_escaped_newline() {
    let output = run_repl("print(\"hello\\\nfrom a short string\")\n");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("hello\nfrom a short string\n"));
    assert!(output.stderr.is_empty());
}

#[test]
fn repl_reports_an_incomplete_chunk_when_input_ends() {
    let output = run_repl("if true then\n");

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("expected End, but found Some(Eof)"));
    assert!(stderr.contains("if true then"));
}

#[test]
fn runtime_errors_fail_and_include_a_source_diagnostic() {
    let script = Script::new("return 1 + true\n");
    let output = Command::new(env!("CARGO_BIN_EXE_orbit"))
        .arg(&script.0)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("attempt to add a number value and a boolean value"));
    assert!(stderr.contains(&script.0.display().to_string()));
    assert!(stderr.contains("return 1 + true"));
    assert!(stderr.contains(":1:8 (pc 2)"));
}

#[test]
fn syntax_errors_fail_and_include_a_source_diagnostic() {
    let script = Script::new("return )\n");
    let output = Command::new(env!("CARGO_BIN_EXE_orbit"))
        .arg(&script.0)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("a return statement must be the final statement in its block"));
    assert!(stderr.contains(&script.0.display().to_string()));
    assert!(stderr.contains("return )"));
}

#[test]
fn invalid_utf8_errors_escape_and_highlight_the_invalid_bytes() {
    let script = Script::new_bytes(b"return 'before \xff after'\n");
    let output = Command::new(env!("CARGO_BIN_EXE_orbit"))
        .arg(&script.0)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("source is not valid UTF-8"));
    assert!(stderr.contains(&script.0.display().to_string()));
    assert!(stderr.contains(r"return 'before \xff after'"));
    assert!(!stderr.contains("[source 0 bytes"));
}
