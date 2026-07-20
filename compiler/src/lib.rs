pub mod bytecode;
mod constants;
mod emitter;
mod error;
mod function;
mod registers;

pub use error::{CompileError, CompileErrorKind};

use crate::bytecode::Chunk;
use orbit_resolver::hir::HirChunk;

pub fn compile(hir: HirChunk) -> Result<Chunk, CompileError> {
    let (strings, entry) = hir.into_parts();

    u32::try_from(strings.len()).map_err(|_| CompileError {
        span: entry.span,
        kind: CompileErrorKind::TooManyStrings,
    })?;

    let entry = function::compile_entry(&entry)?;

    Ok(Chunk {
        strings: strings.into_boxed_slice(),
        entry,
    })
}
