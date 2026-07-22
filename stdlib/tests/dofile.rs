use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use orbit_common::SourceId;
use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{
    CallOutcome, LoadError, LoadService, LuaString, State, Value, VmErrorKind, VmResult,
};

static NEXT_SCRIPT_ID: AtomicU64 = AtomicU64::new(0);

struct Script {
    path: PathBuf,
}

impl Script {
    fn new(source: &str) -> Self {
        let id = NEXT_SCRIPT_ID.fetch_add(1, Ordering::Relaxed);

        let path = std::env::temp_dir()
            .join(format!("orbit-dofile-test-{}-{id}.lua", std::process::id(),));

        fs::write(&path, source).unwrap();

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn call_dofile_with(state: &mut State, arguments: &[Value]) -> VmResult<Vec<Value>> {
    let Value::Function(dofile) = state.get_global(b"dofile")? else {
        panic!("dofile was not installed as a function");
    };

    match state.call(&dofile, arguments)? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => {
            panic!("dofile unexpectedly yielded");
        }
    }
}

fn call_dofile(state: &mut State, path: &Path) -> VmResult<Vec<Value>> {
    let filename = Value::String(LuaString::new(path.to_string_lossy().as_bytes()));
    call_dofile_with(state, &[filename])
}

#[derive(Debug, Default)]
struct LoadCalls {
    filenames: Vec<Vec<u8>>,
    stdin: usize,
}

struct StubLoadService {
    loader: Loader,
    file_source: Box<[u8]>,
    stdin_source: Box<[u8]>,
    calls: Arc<Mutex<LoadCalls>>,
}

impl StubLoadService {
    fn new(
        file_source: impl Into<Box<[u8]>>,
        stdin_source: impl Into<Box<[u8]>>,
    ) -> (Self, Arc<Mutex<LoadCalls>>) {
        let calls = Arc::new(Mutex::new(LoadCalls::default()));

        (
            Self {
                loader: Loader::new(),
                file_source: file_source.into(),
                stdin_source: stdin_source.into(),
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl LoadService for StubLoadService {
    fn compile_buffer(
        &mut self,
        source_id: SourceId,
        name: &[u8],
        source: &[u8],
    ) -> Result<orbit_compiler::bytecode::Chunk, LoadError> {
        self.loader.compile_buffer(source_id, name, source)
    }

    fn compile_file(
        &mut self,
        source_id: SourceId,
        filename: &[u8],
    ) -> Result<orbit_compiler::bytecode::Chunk, LoadError> {
        self.calls.lock().unwrap().filenames.push(filename.to_vec());
        self.loader
            .compile_buffer(source_id, filename, &self.file_source)
    }

    fn compile_stdin(
        &mut self,
        source_id: SourceId,
    ) -> Result<orbit_compiler::bytecode::Chunk, LoadError> {
        self.calls.lock().unwrap().stdin += 1;
        self.loader
            .compile_buffer(source_id, b"<stdin>", &self.stdin_source)
    }

    fn file_exists(&self, _filename: &[u8]) -> bool {
        true
    }
}

#[test]
fn dofile_executes_the_file_and_forwards_all_results() {
    let script = Script::new(
        r#"
loaded_value = 41
return loaded_value, "finished", nil
"#,
    );

    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let results = call_dofile(&mut state, script.path()).unwrap();

    assert_eq!(
        results,
        vec![
            Value::Integer(41),
            Value::String(LuaString::from("finished")),
            Value::Nil,
        ]
    );

    assert_eq!(
        state.get_global(b"loaded_value").unwrap(),
        Value::Integer(41)
    );
}

#[test]
fn dofile_reloads_the_file_on_every_call() {
    let script = Script::new(
        r#"
counter = (counter or 0) + 1
return counter
"#,
    );

    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    assert_eq!(
        call_dofile(&mut state, script.path()).unwrap(),
        vec![Value::Integer(1)]
    );

    assert_eq!(
        call_dofile(&mut state, script.path()).unwrap(),
        vec![Value::Integer(2)]
    );
}

#[test]
fn dofile_preserves_structured_lexer_and_parser_errors() {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let lex_script = Script::new("return @");
    let error = call_dofile(&mut state, lex_script.path()).unwrap_err();
    let lex_span = match &error.kind {
        VmErrorKind::LoadFailure(LoadError::Lex(error)) => error.span,
        other => panic!("expected a structured lexer error, got {other:?}"),
    };

    assert_eq!((lex_span.start, lex_span.end), (7, 8));
    let parse_script = Script::new("return )");
    let error = call_dofile(&mut state, parse_script.path()).unwrap_err();
    let parse_span = match &error.kind {
        VmErrorKind::LoadFailure(LoadError::Parse(error)) => error.span,
        other => panic!("expected a structured parser error, got {other:?}"),
    };

    assert_eq!((parse_span.start, parse_span.end), (7, 8));
    assert_ne!(parse_span.source, lex_span.source);
}

#[test]
fn dofile_preserves_structured_file_io_errors() {
    let id = NEXT_SCRIPT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "orbit-dofile-missing-{}-{id}.lua",
        std::process::id()
    ));

    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let error = call_dofile(&mut state, &path).unwrap_err();

    assert!(matches!(
        error.kind,
        VmErrorKind::LoadFailure(LoadError::FileIo {
            source_id,
            kind: std::io::ErrorKind::NotFound,
        }) if source_id == SourceId::new(0)
    ));
}

#[test]
fn dofile_coerces_numeric_filenames_but_rejects_other_types() {
    let (service, calls) = StubLoadService::new(&b"return 73"[..], &b"return 0"[..]);
    let mut state = State::new(service).unwrap();
    install(&mut state).unwrap();

    assert_eq!(
        call_dofile_with(&mut state, &[Value::Integer(42)]).unwrap(),
        vec![Value::Integer(73)]
    );
    assert_eq!(calls.lock().unwrap().filenames, [b"42".to_vec()]);

    let error = call_dofile_with(&mut state, &[Value::Boolean(true)]).unwrap_err();
    let VmErrorKind::NativeFunctionFailure { message } = error.kind else {
        panic!("expected a type error");
    };
    assert_eq!(
        message.as_ref(),
        "bad argument #1 to 'dofile' (string expected, got boolean)"
    );
}

#[test]
fn dofile_uses_stdin_for_no_argument_and_explicit_nil() {
    let (service, calls) = StubLoadService::new(&b"return 0"[..], &b"return 'stdin'"[..]);
    let mut state = State::new(service).unwrap();
    install(&mut state).unwrap();

    let expected = vec![Value::String(LuaString::from("stdin"))];
    assert_eq!(call_dofile_with(&mut state, &[]).unwrap(), expected);
    assert_eq!(
        call_dofile_with(&mut state, &[Value::Nil]).unwrap(),
        expected
    );

    assert_eq!(calls.lock().unwrap().stdin, 2);
}
