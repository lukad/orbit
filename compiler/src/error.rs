use orbit_common::Span;

#[derive(Debug, thiserror::Error)]
#[error("{kind}")]
pub struct CompileError {
    pub span: Span,
    pub kind: CompileErrorKind,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileErrorKind {
    #[error("function requires {required} registers, but at most 256 are supported")]
    TooManyRegisters { required: u32 },
    #[error("function has too many parameters")]
    TooManyParameters,
    #[error("argument count does not fit in bytecode")]
    TooManyArguments,
    #[error("result count does not fit in bytecode")]
    TooManyResults,
    #[error("function has too many constants")]
    TooManyConstants,
    #[error("chunk has too many strings")]
    TooManyStrings,
    #[error("function has too many child prototypes")]
    TooManyChildren,
    #[error("function has too many upvalues")]
    TooManyUpvalues,
    #[error("function has too many instructions")]
    TooManyInstructions,
    #[error("jump offset does not fit in i32")]
    JumpTooFar,
    #[error("table constructor has too many fields")]
    TooManyTableFields,
}
