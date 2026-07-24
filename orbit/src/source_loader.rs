use std::{
    ffi::OsStr,
    io::{self, Read},
    path::PathBuf,
};

use orbit_common::SourceId;
use orbit_compiler::bytecode::Chunk;
use orbit_loader::Loader;
use orbit_vm::{LoadError, LoadService, LoadSource};

use crate::diagnostics::SharedSources;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

pub(crate) struct DiagnosticLoader {
    inner: Loader,
    sources: SharedSources,
}

impl DiagnosticLoader {
    pub(crate) fn new(sources: SharedSources) -> Self {
        Self {
            inner: Loader::new(),
            sources,
        }
    }
}

impl LoadService for DiagnosticLoader {
    fn compile(&mut self, source_id: SourceId, source: LoadSource<'_>) -> Result<Chunk, LoadError> {
        match source {
            LoadSource::Buffer { name, source } => {
                self.sources
                    .borrow_mut()
                    .insert(source_id, name, source.to_vec());
                self.inner
                    .compile(source_id, LoadSource::Buffer { name, source })
            }
            LoadSource::File { filename } => {
                let path = path_from_bytes(filename)
                    .ok_or(LoadError::InvalidFilenameEncoding { source_id })?;
                let mut source = std::fs::read(&path).map_err(|error| LoadError::FileIo {
                    source_id,
                    kind: error.kind(),
                })?;

                preprocess_file_source(&mut source);
                let name = path.to_string_lossy();
                let result = self.inner.compile(
                    source_id,
                    LoadSource::Buffer {
                        name: filename,
                        source: &source,
                    },
                );
                self.sources
                    .borrow_mut()
                    .insert(source_id, name.as_bytes(), source);
                result
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
                let result = self.inner.compile(
                    source_id,
                    LoadSource::Buffer {
                        name: b"<stdin>",
                        source: &source,
                    },
                );
                self.sources
                    .borrow_mut()
                    .insert(source_id, b"<stdin>", source);
                result
            }
        }
    }

    fn file_exists(&self, filename: &[u8]) -> bool {
        self.inner.file_exists(filename)
    }
}

#[cfg(unix)]
fn path_from_bytes(filename: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    Some(PathBuf::from(OsStr::from_bytes(filename)))
}

#[cfg(not(unix))]
fn path_from_bytes(filename: &[u8]) -> Option<PathBuf> {
    std::str::from_utf8(filename).ok().map(PathBuf::from)
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
