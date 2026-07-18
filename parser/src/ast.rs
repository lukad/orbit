use orbit_common::{Span, Spanned};

use crate::lexer::{ByteString, Symbol};

pub type Chunk = Block;

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub const fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Empty,
    Local {
        names: Vec<LocalDecl>,
        values: Vec<Expr>,
    },
    Assign {
        targets: Vec<AssignmentTarget>,
        values: Vec<Expr>,
    },
    Call(Call),
    Label(Spanned<Symbol>),
    Break,
    Goto(Spanned<Symbol>),
    Do(Block),
    While {
        condition: Expr,
        body: Block,
    },
    Repeat {
        body: Block,
        condition: Expr,
    },
    If {
        branches: Vec<IfBranch>,
        else_block: Option<Block>,
    },
    NumericFor {
        name: Spanned<Symbol>,
        initial: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Block,
    },
    GenericFor {
        names: Vec<Spanned<Symbol>>,
        values: Vec<Expr>,
        body: Block,
    },
    Function {
        name: FunctionName,
        body: FunctionBody,
    },
    LocalFunction {
        name: Spanned<Symbol>,
        body: FunctionBody,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfBranch {
    pub condition: Expr,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalDecl {
    pub name: Spanned<Symbol>,
    pub attribute: Option<Spanned<LocalAttribute>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalAttribute {
    Const,
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentTarget {
    pub kind: AssignmentTargetKind,
    pub span: Span,
}

impl AssignmentTarget {
    pub const fn new(kind: AssignmentTargetKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssignmentTargetKind {
    Name(Symbol),
    Index {
        table: Box<Expr>,
        key: Box<Expr>,
    },
    Field {
        table: Box<Expr>,
        field: Spanned<Symbol>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub return_statement: Option<ReturnStmt>,
    pub span: Span,
}

impl Block {
    pub const fn new(
        statements: Vec<Stmt>,
        return_statement: Option<ReturnStmt>,
        span: Span,
    ) -> Self {
        Self {
            statements,
            return_statement,
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStmt {
    pub values: Vec<Expr>,
    pub span: Span,
}

impl ReturnStmt {
    pub const fn new(values: Vec<Expr>, span: Span) -> Self {
        Self { values, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionName {
    pub name: Spanned<Symbol>,
    pub fields: Vec<Spanned<Symbol>>,
    pub method: Option<Spanned<Symbol>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionBody {
    pub parameters: Vec<Spanned<Symbol>>,
    pub is_variadic: bool,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub const fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(ByteString),
    Vararg,
    Name(Symbol),
    Parenthesized(Box<Expr>),
    Unary {
        operator: UnaryOperator,
        expression: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        operator: BinaryOperator,
        right: Box<Expr>,
    },
    Index {
        table: Box<Expr>,
        key: Box<Expr>,
    },
    Field {
        table: Box<Expr>,
        field: Spanned<Symbol>,
    },
    Call(Call),
    Function(Box<FunctionBody>),
    Table(Vec<TableField>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Function {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },
    Method {
        receiver: Box<Expr>,
        method: Spanned<Symbol>,
        arguments: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    Or,
    And,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    BitwiseOr,
    BitwiseXor,
    BitwiseAnd,
    ShiftLeft,
    ShiftRight,
    Concat,
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDivide,
    Modulo,
    Power,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    Negate,
    Not,
    Length,
    BitwiseNot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableField {
    pub kind: TableFieldKind,
    pub span: Span,
}

impl TableField {
    pub const fn new(kind: TableFieldKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableFieldKind {
    Indexed { key: Expr, value: Expr },
    Named { name: Spanned<Symbol>, value: Expr },
    Value(Expr),
}
