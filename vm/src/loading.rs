use std::io;

use orbit_common::{SourceId, Span};
use orbit_compiler::{CompileError, bytecode::Chunk};
use orbit_parser::{lexer::LexError, parser::ParseError};
use orbit_resolver::Diagnostic;

/// A structured failure produced while locating or compiling dynamic source.
///
/// Source-language failures retain their original parser/compiler error types.
/// They deliberately do not retain filenames, rendered diagnostics, or source
/// text; callers relate their spans to source records using `SourceId`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("dynamic source loading is not configured")]
    DynamicLoadingDisabled { source_id: SourceId },
    #[error("source identifier space exhausted")]
    SourceIdExhausted,
    #[error("source identifier {} is already reserved", source_id.get())]
    SourceIdCollision { source_id: SourceId },
    #[error(
        "compiled source used identifier {}, expected {}",
        actual.get(),
        expected.get()
    )]
    UnexpectedSourceId {
        expected: SourceId,
        actual: SourceId,
    },
    #[error("filename cannot be represented on this platform")]
    InvalidFilenameEncoding { source_id: SourceId },
    #[error("source file I/O failed with {kind:?}")]
    FileIo {
        source_id: SourceId,
        kind: io::ErrorKind,
    },
    #[error("standard input I/O failed with {kind:?}")]
    StdinIo {
        source_id: SourceId,
        kind: io::ErrorKind,
    },
    #[error("source is not valid UTF-8")]
    InvalidUtf8 { span: Span },
    #[error(transparent)]
    Lex(#[from] LexError),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error("source resolution failed")]
    Resolve { diagnostics: Box<[Diagnostic]> },
    #[error(transparent)]
    Compile(#[from] CompileError),
}

impl LoadError {
    /// Returns the source identifier assigned to this load attempt, when one
    /// exists. Errors raised before an identifier can be allocated return
    /// `None`.
    pub fn source_id(&self) -> Option<SourceId> {
        match self {
            Self::DynamicLoadingDisabled { source_id }
            | Self::SourceIdCollision { source_id }
            | Self::InvalidFilenameEncoding { source_id }
            | Self::FileIo { source_id, .. }
            | Self::StdinIo { source_id, .. } => Some(*source_id),
            Self::UnexpectedSourceId { expected, .. } => Some(*expected),
            Self::InvalidUtf8 { span } => Some(span.source),
            Self::Lex(error) => Some(error.span.source),
            Self::Parse(error) => Some(error.span.source),
            Self::Resolve { diagnostics } => {
                diagnostics.first().map(|diagnostic| diagnostic.span.source)
            }
            Self::Compile(error) => Some(error.span.source),
            Self::SourceIdExhausted => None,
        }
    }

    pub fn primary_span(&self) -> Option<Span> {
        match self {
            Self::InvalidUtf8 { span } => Some(*span),
            Self::Lex(error) => Some(error.span),
            Self::Parse(error) => Some(error.span),
            Self::Resolve { diagnostics } => diagnostics.first().map(|error| error.span),
            Self::Compile(error) => Some(error.span),
            Self::DynamicLoadingDisabled { .. }
            | Self::SourceIdExhausted
            | Self::SourceIdCollision { .. }
            | Self::UnexpectedSourceId { .. }
            | Self::InvalidFilenameEncoding { .. }
            | Self::FileIo { .. }
            | Self::StdinIo { .. } => None,
        }
    }
}

/// Source input accepted by a dynamic [`LoadService`].
#[derive(Debug, Clone, Copy)]
pub enum LoadSource<'source> {
    Buffer {
        name: &'source [u8],
        source: &'source [u8],
    },
    File {
        filename: &'source [u8],
    },
    Stdin,
}

/// Host-provided dynamic source compiler and filesystem capability.
///
/// The VM assigns `source_id` before asking the service to compile. The
/// service must use that identifier for every span in the returned chunk and
/// in any structured source-language error. The VM deliberately stores no
/// filename or source text; embedders that need rendered diagnostics should
/// maintain their own source-ID mapping.
pub trait LoadService {
    fn compile(&mut self, source_id: SourceId, source: LoadSource<'_>) -> Result<Chunk, LoadError>;

    fn file_exists(&self, filename: &[u8]) -> bool;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoLoadService;

impl LoadService for NoLoadService {
    fn compile(
        &mut self,
        source_id: SourceId,
        _source: LoadSource<'_>,
    ) -> Result<Chunk, LoadError> {
        Err(LoadError::DynamicLoadingDisabled { source_id })
    }

    fn file_exists(&self, _filename: &[u8]) -> bool {
        false
    }
}
