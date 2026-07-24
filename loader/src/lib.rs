use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
};

use orbit_common::{SourceId, Span};
use orbit_compiler::bytecode::Chunk;
use orbit_vm::{LoadError, LoadService, LoadSource};

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// The default host-filesystem source loader.
///
/// `Loader` deliberately permits arbitrary paths supplied by Lua code. It is
/// appropriate for the trusted command-line interpreter, but it is not a
/// sandbox. Embedders running untrusted Lua should provide a rooted or
/// allowlisted [`LoadService`] instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct Loader;

impl Loader {
    pub fn new() -> Self {
        Self
    }

    fn compile_bytes(&mut self, source_id: SourceId, bytes: &[u8]) -> Result<Chunk, LoadError> {
        let text = std::str::from_utf8(bytes).map_err(|error| LoadError::InvalidUtf8 {
            span: utf8_error_span(source_id, error, bytes.len()),
        })?;

        compile_source(source_id, text)
    }
}

impl LoadService for Loader {
    fn compile(&mut self, source_id: SourceId, source: LoadSource<'_>) -> Result<Chunk, LoadError> {
        match source {
            LoadSource::Buffer { source, .. } => self.compile_bytes(source_id, source),
            LoadSource::File { filename } => {
                let path = path_from_bytes(filename)
                    .map_err(|()| LoadError::InvalidFilenameEncoding { source_id })?;

                let mut source = fs::read(&path).map_err(|error| LoadError::FileIo {
                    source_id,
                    kind: error.kind(),
                })?;

                preprocess_file_source(&mut source);
                self.compile_bytes(source_id, &source)
            }
            LoadSource::Stdin => {
                let mut source = Vec::new();

                io::stdin()
                    .lock()
                    .read_to_end(&mut source)
                    .map_err(|error| LoadError::StdinIo {
                        source_id,
                        kind: error.kind(),
                    })?;

                preprocess_file_source(&mut source);
                self.compile_bytes(source_id, &source)
            }
        }
    }

    fn file_exists(&self, filename: &[u8]) -> bool {
        path_from_bytes(filename)
            .ok()
            .is_some_and(|path| fs::File::open(path).is_ok())
    }
}

fn compile_source(source_id: SourceId, source: &str) -> Result<Chunk, LoadError> {
    let tokens = orbit_parser::lexer::lex(source_id, source).map_err(LoadError::Lex)?;
    let ast = orbit_parser::parser::parse_chunk(source_id, &tokens).map_err(LoadError::Parse)?;

    let hir = orbit_resolver::resolve(&ast).map_err(|diagnostics| LoadError::Resolve {
        diagnostics: diagnostics.into_boxed_slice(),
    })?;

    orbit_compiler::compile(hir).map_err(LoadError::Compile)
}

fn preprocess_file_source(source: &mut [u8]) {
    let mut first = 0;

    if source.starts_with(UTF8_BOM) {
        source[..UTF8_BOM.len()].fill(b' ');
        first = UTF8_BOM.len();
    }

    if source.get(first) == Some(&b'#') {
        let line_length = source[first..]
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(source.len() - first);

        source[first..first + line_length].fill(b' ');
    }
}

fn utf8_error_span(source: SourceId, error: std::str::Utf8Error, source_len: usize) -> Span {
    let start = u32::try_from(error.valid_up_to()).unwrap_or(u32::MAX);
    let end = match error.error_len() {
        Some(length) => start.saturating_add(u32::try_from(length).unwrap_or(u32::MAX)),
        None => u32::try_from(source_len).unwrap_or(u32::MAX),
    };

    Span::new(source, start, end)
}

#[cfg(unix)]
fn path_from_bytes(filename: &[u8]) -> Result<PathBuf, ()> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    Ok(PathBuf::from(OsStr::from_bytes(filename)))
}

#[cfg(not(unix))]
fn path_from_bytes(filename: &[u8]) -> Result<PathBuf, ()> {
    let filename = std::str::from_utf8(filename).map_err(|_| ())?;

    Ok(PathBuf::from(filename))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use orbit_common::{SourceId, Span};
    use orbit_compiler::CompileErrorKind;
    use orbit_vm::{
        CallOutcome, LoadError, LoadService, LoadSource, NoLoadService, State, VmError,
        VmErrorKind, VmTraceFrame,
    };

    use super::Loader;

    static NEXT_FILE_ID: AtomicU64 = AtomicU64::new(0);

    struct TestFile(PathBuf);

    impl TestFile {
        fn new(source: &[u8]) -> Self {
            let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("orbit-loader-test-{}-{id}.lua", std::process::id()));

            fs::write(&path, source).unwrap();
            Self(path)
        }
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn innermost_source(error: &VmError) -> SourceId {
        let Some(VmTraceFrame::Lua { function_span, .. }) = error.frames.first() else {
            panic!("runtime error should contain an innermost Lua frame");
        };

        function_span.source
    }

    fn call_error(state: &mut State, function: &orbit_vm::Function) -> VmError {
        match state.call(function, &[]) {
            Err(error) => error,
            Ok(CallOutcome::Returned(values)) => {
                panic!("source unexpectedly returned {values:?}")
            }
            Ok(CallOutcome::Yielded { .. }) => panic!("source unexpectedly yielded"),
        }
    }

    #[test]
    fn compiles_a_buffer_with_the_assigned_source_id() {
        let mut loader = Loader::new();
        let source_id = SourceId::new(7);

        let loaded = loader
            .compile(
                source_id,
                LoadSource::Buffer {
                    name: b"example.lua",
                    source: b"return 42",
                },
            )
            .unwrap();

        assert_eq!(loaded.entry.span.source, source_id);
    }

    #[test]
    fn exposes_structured_lex_and_parse_errors() {
        let mut loader = Loader::new();

        let lex = loader
            .compile(
                SourceId::new(11),
                LoadSource::Buffer {
                    name: b"lex.lua",
                    source: b"return @",
                },
            )
            .unwrap_err();
        let LoadError::Lex(lex) = lex else {
            panic!("invalid character should retain LexError");
        };
        assert_eq!(lex.span, Span::new(SourceId::new(11), 7, 8));

        let parse = loader
            .compile(
                SourceId::new(12),
                LoadSource::Buffer {
                    name: b"parse.lua",
                    source: b"return )",
                },
            )
            .unwrap_err();
        let LoadError::Parse(parse) = parse else {
            panic!("invalid syntax should retain ParseError");
        };
        assert_eq!(parse.span, Span::new(SourceId::new(12), 7, 8));
    }

    #[test]
    fn exposes_structured_resolve_and_compile_errors() {
        let mut loader = Loader::new();

        let resolve = loader
            .compile(
                SourceId::new(15),
                LoadSource::Buffer {
                    name: b"resolve.lua",
                    source: b"goto missing",
                },
            )
            .unwrap_err();
        let LoadError::Resolve { diagnostics } = resolve else {
            panic!("an unresolved label should retain resolver diagnostics");
        };
        assert!(!diagnostics.is_empty());
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.source == SourceId::new(15))
        );

        let parameters = (0..256)
            .map(|index| format!("p{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!("return function({parameters}) end");
        let compile = loader
            .compile(
                SourceId::new(16),
                LoadSource::Buffer {
                    name: b"compile.lua",
                    source: source.as_bytes(),
                },
            )
            .unwrap_err();
        let LoadError::Compile(compile) = compile else {
            panic!("a compiler limit should retain CompileError");
        };
        assert_eq!(compile.span.source, SourceId::new(16));
        assert_eq!(compile.kind, CompileErrorKind::TooManyParameters);
    }

    #[test]
    fn exposes_the_exact_span_of_invalid_utf8() {
        let mut loader = Loader::new();

        let error = loader
            .compile(
                SourceId::new(13),
                LoadSource::Buffer {
                    name: b"utf8.lua",
                    source: b"return \xff",
                },
            )
            .unwrap_err();

        assert_eq!(
            error,
            LoadError::InvalidUtf8 {
                span: Span::new(SourceId::new(13), 7, 8),
            }
        );
        assert_eq!(
            error.primary_span(),
            Some(Span::new(SourceId::new(13), 7, 8))
        );
        assert_eq!(error.source_id(), Some(SourceId::new(13)));

        let truncated = loader
            .compile(
                SourceId::new(14),
                LoadSource::Buffer {
                    name: b"utf8.lua",
                    source: b"return \xe2\x82",
                },
            )
            .unwrap_err();
        assert_eq!(
            truncated.primary_span(),
            Some(Span::new(SourceId::new(14), 7, 9))
        );
    }

    #[test]
    fn file_loading_accepts_a_bom_and_shebang_without_shifting_spans() {
        const SOURCE: &[u8] = b"\xef\xbb\xbf#!/usr/bin/env lua\nreturn 1 + true\n";
        const FAILING_EXPRESSION: &[u8] = b"1 + true";

        let file = TestFile::new(SOURCE);
        let mut loader = Loader::new();
        let source_id = SourceId::new(19);

        let chunk = loader
            .compile(
                source_id,
                LoadSource::File {
                    filename: file.0.as_os_str().as_encoded_bytes(),
                },
            )
            .unwrap();
        let mut state = State::new(NoLoadService).unwrap();
        let function = state.load_chunk(chunk).unwrap();
        let error = call_error(&mut state, &function);

        let start = SOURCE
            .windows(FAILING_EXPRESSION.len())
            .position(|window| window == FAILING_EXPRESSION)
            .unwrap();
        let expected = Span::new(
            source_id,
            u32::try_from(start).unwrap(),
            u32::try_from(start + FAILING_EXPRESSION.len()).unwrap(),
        );

        let Some(VmTraceFrame::Lua {
            instruction_span, ..
        }) = error.frames.first()
        else {
            panic!("runtime error should contain an innermost Lua frame");
        };

        assert_eq!(*instruction_span, Some(expected));
    }

    #[test]
    fn failed_compilations_expose_only_a_structured_error() {
        let mut state = State::new(Loader::new()).unwrap();

        let error = state.load_buffer(b"broken.lua", b"return )").unwrap_err();
        let orbit_vm::VmErrorKind::LoadFailure(LoadError::Parse(parse)) = error.kind else {
            panic!("syntax failure should retain its ParseError");
        };

        assert_eq!(parse.span.source, SourceId::new(0));
    }

    #[test]
    fn failed_compilations_consume_their_source_identifier() {
        let mut state = State::new(Loader::new()).unwrap();

        let error = state.load_buffer(b"broken.lua", b"return )").unwrap_err();
        let VmErrorKind::LoadFailure(LoadError::Parse(parse)) = error.kind else {
            panic!("syntax failure should retain its ParseError");
        };

        let function = state.load_buffer(b"next.lua", b"return 1 + true").unwrap();
        let runtime_error = call_error(&mut state, &function);

        assert_eq!(parse.span.source, SourceId::new(0));
        assert_eq!(innermost_source(&runtime_error), SourceId::new(1));
    }

    #[test]
    fn precompiled_chunks_advance_the_dynamic_source_identifier_allocator() {
        let mut compiler = Loader::new();
        let precompiled = compiler
            .compile(
                SourceId::new(41),
                LoadSource::Buffer {
                    name: b"precompiled.lua",
                    source: b"return 1",
                },
            )
            .unwrap();

        let mut state = State::new(Loader::new()).unwrap();
        let precompiled_function = state.load_chunk(precompiled).unwrap();
        let dynamic_function = state
            .load_buffer(b"dynamic.lua", b"return 1 + true")
            .unwrap();
        let runtime_error = call_error(&mut state, &dynamic_function);

        assert_eq!(innermost_source(&runtime_error), SourceId::new(42));

        drop(precompiled_function);
    }

    #[test]
    fn source_identifier_collisions_remain_reserved_after_collection() {
        let mut state = State::new(Loader::new()).unwrap();
        let dynamic_function = state.load_buffer(b"dynamic.lua", b"return 1").unwrap();

        drop(dynamic_function);
        state.collect_garbage().unwrap();

        let mut compiler = Loader::new();
        let colliding = compiler
            .compile(
                SourceId::new(0),
                LoadSource::Buffer {
                    name: b"precompiled.lua",
                    source: b"return 2",
                },
            )
            .unwrap();
        let error = state.load_chunk(colliding).unwrap_err();

        assert_eq!(
            error.kind,
            VmErrorKind::LoadFailure(LoadError::SourceIdCollision {
                source_id: SourceId::new(0),
            })
        );
    }
}
