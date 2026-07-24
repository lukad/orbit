use orbit_common::Span;
use orbit_parser::lexer::Symbol;

pub use crate::arena::Arena;

macro_rules! arena_ids {
    ($($id:ident),+ $(,)?) => {
        $(
            impl From<u32> for $id {
                fn from(index: u32) -> Self {
                    Self(index)
                }
            }

            impl From<$id> for u32 {
                fn from(id: $id) -> Self {
                    id.0
                }
            }
        )+
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StmtId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UpvalueId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoopId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LabelId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChildFunctionId(pub u32);

arena_ids!(
    BlockId, StmtId, ExprId, ScopeId, LocalId, UpvalueId, LoopId, LabelId,
);

#[derive(Debug)]
pub struct HirChunk {
    pub(crate) strings: Vec<Box<[u8]>>,
    pub(crate) entry: HirFunction,
}

impl HirChunk {
    pub fn strings(&self) -> &[Box<[u8]>] {
        &self.strings
    }

    pub fn entry(&self) -> &HirFunction {
        &self.entry
    }

    pub fn into_parts(self) -> (Vec<Box<[u8]>>, HirFunction) {
        (self.strings, self.entry)
    }
}

impl StringId {
    pub(crate) fn new(index: u32) -> Self {
        Self(index)
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug)]
pub struct HirFunction {
    pub name: Option<Symbol>,
    pub span: Span,
    pub parameters: Vec<LocalId>,
    pub is_vararg: bool,
    pub locals: Arena<LocalId, HirLocal>,
    pub upvalues: Arena<UpvalueId, HirUpvalue>,
    pub scopes: Arena<ScopeId, HirScope>,
    pub blocks: Arena<BlockId, HirBlock>,
    pub statements: Arena<StmtId, HirStmt>,
    pub expressions: Arena<ExprId, HirExpr>,
    /// Number of loop identities allocated by the resolver. Loop structure
    /// itself lives in `HirStmtKind`, so it cannot disagree with a second table.
    pub loop_count: usize,
    pub labels: Arena<LabelId, HirLabel>,
    pub children: Vec<HirFunction>,
    pub body: BlockId,
}

#[derive(Debug)]
pub struct HirLocal {
    pub name: Symbol,
    pub span: Span,
    pub attribute: Option<LocalAttribute>,
    /// A nested function captures this local
    pub captured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAttribute {
    Const,
    Close,
}

#[derive(Debug)]
pub struct HirUpvalue {
    pub name: Symbol,
    pub span: Span,
    pub source: UpvalueSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpvalueSource {
    /// Used for the loaded chunk's `_ENV`
    ExternalEnvironment,
    /// Capture a local from the immediately enclosing function.
    ParentLocal(LocalId),
    /// Capture one of the immediately enclosing function's upvalues.
    ParentUpvalue(UpvalueId),
}

#[derive(Debug)]
pub struct HirScope {
    pub parent: Option<ScopeId>,
    pub has_captured_locals: bool,
    pub has_to_be_closed_locals: bool,
}

#[derive(Debug)]
pub struct HirBlock {
    pub span: Span,
    pub scope: ScopeId,
    pub statements: Vec<StmtId>,
}
#[derive(Debug)]
pub struct HirExpr {
    pub span: Span,
    pub kind: HirExprKind,
}

#[derive(Debug)]
pub enum HirExprKind {
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(StringId),
    Vararg,
    Read(Binding),
    Unary {
        operator: UnaryOperator,
        operand: ExprId,
    },
    Binary {
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
    },
    Index {
        table: ExprId,
        key: ExprId,
    },
    Call {
        callee: ExprId,
        arguments: Vec<ExprId>,
    },
    MethodCall {
        receiver: ExprId,
        method: StringId,
        arguments: Vec<ExprId>,
    },
    Closure(ChildFunctionId),
    Table {
        fields: Vec<HirTableField>,
    },
    /// Force a potentially multi-valued expression to exactly one result.
    /// This represents parentheses around Call and Vararg:
    ///
    /// ```text
    /// return f()    -- potentially many
    /// return (f())  -- exactly one
    /// ```
    AdjustToOne {
        expression: ExprId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Local(LocalId),
    Upvalue(UpvalueId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Negate,
    Not,
    Length,
    BitwiseNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDivide,
    Modulo,
    Power,

    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    ShiftLeft,
    ShiftRight,

    Concat,

    Equal,
    NotEqual,
    LessThan,
    LessEqual,
    GreaterThan,
    GreaterEqual,

    And,
    Or,
}

#[derive(Debug)]
pub enum HirTableField {
    /// `{ value }`
    List { span: Span, value: ExprId },
    /// `{ name = value }`
    Record {
        span: Span,
        name: StringId,
        value: ExprId,
    },
    /// `{ [key] = value }`
    Computed {
        span: Span,
        key: ExprId,
        value: ExprId,
    },
}

#[derive(Debug)]
pub struct HirPlace {
    pub span: Span,
    pub kind: HirPlaceKind,
}

#[derive(Debug)]
pub enum HirPlaceKind {
    Local(LocalId),
    Upvalue(UpvalueId),
    Index { table: ExprId, key: ExprId },
}

#[derive(Debug)]
pub struct HirStmt {
    pub span: Span,
    pub kind: HirStmtKind,
}

#[derive(Debug)]
pub enum HirStmtKind {
    /// Represents `do ... end`
    Block(BlockId),
    /// Local declaration and optional initialization.
    ///
    /// Initializer expressions are evaluated before the new locals receive
    /// their values.
    Local {
        locals: Vec<LocalId>,
        values: Vec<ExprId>,
    },

    /// Parallel assignment.
    ///
    /// Code generation must:
    /// 1. prepare all indexed targets,
    /// 2. evaluate all values,
    /// 3. perform the writes.
    Assign {
        targets: Vec<HirPlace>,
        values: Vec<ExprId>,
    },
    /// Lua only allows function calls as free-standing expression statements
    Call {
        call: ExprId,
    },
    If {
        branches: Vec<HirConditionalBranch>,
        else_block: Option<BlockId>,
    },
    While {
        loop_id: LoopId,
        condition: ExprId,
        body: BlockId,
    },
    Repeat {
        loop_id: LoopId,
        body: BlockId,
        condition: ExprId,
    },
    NumericFor {
        loop_id: LoopId,
        variable: LocalId,
        initial: ExprId,
        limit: ExprId,
        step: Option<ExprId>,
        body: BlockId,
    },
    GenericFor {
        loop_id: LoopId,
        variables: Vec<LocalId>,
        expressions: Vec<ExprId>,
        body: BlockId,
    },
    Return {
        values: Vec<ExprId>,
        exit: ExitPlan,
    },
    Break {
        target: LoopId,
        exit: ExitPlan,
    },
    Goto {
        target: LabelId,
        exit: ExitPlan,
    },
    Label {
        label: LabelId,
    },
}

#[derive(Debug)]
pub struct HirConditionalBranch {
    pub span: Span,
    pub condition: ExprId,
    pub body: BlockId,
}

#[derive(Debug)]
pub struct HirLabel {
    pub name: Symbol,
    pub span: Span,
    pub scope: ScopeId,
    pub active_locals: Vec<LocalId>,
}

#[derive(Debug, Clone)]
pub struct ExitPlan {
    pub scopes: Vec<ScopeId>,
}
