use orbit_common::Span;

macro_rules! index_types {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub struct $name(u32);

            impl $name {
                pub const fn new(value: u32) -> Self {
                    Self(value)
                }

                pub const fn get(self) -> u32 {
                    self.0
                }
            }

            impl From<u32> for $name {
                fn from(value: u32) -> Self {
                    Self::new(value)
                }
            }

            impl From<$name> for u32 {
                fn from(index: $name) -> Self {
                    index.get()
                }
            }
        )+
    };
}

index_types!(ConstantIndex, PrototypeIndex, StringIndex, UpvalueIndex);

#[derive(Debug)]
pub struct Chunk {
    pub strings: Box<[Box<[u8]>]>,
    pub entry: Prototype,
}

#[derive(Debug)]
pub struct Prototype {
    pub span: Span,
    pub parameter_count: u8,
    pub is_vararg: bool,
    pub max_registers: u16,

    pub constants: Box<[Constant]>,
    pub upvalues: Box<[UpvalueDescriptor]>,
    pub children: Box<[Prototype]>,
    pub code: Box<[Instruction]>,

    pub source_map: Box<[SourceMapEntry]>,
}

#[derive(Debug)]
pub enum Constant {
    Integer(i64),
    FloatBits(u64),
    String(StringIndex),
}

#[derive(Debug)]
pub enum UpvalueDescriptor {
    /// Supplies the host-provided environment cell to the entry prototype.
    ExternalEnvironment,
    /// Captures the parent frame's register through its shared open-upvalue
    /// cell. Every closure that captures the same live register shares that
    /// cell until `CloseFrom` closes it.
    ParentRegister(Register),
    /// Reuses one of the parent closure's upvalue cells rather than copying
    /// the value stored in that cell.
    ParentUpvalue(UpvalueIndex),
}

#[derive(Debug)]
pub struct SourceMapEntry {
    pub pc: u32,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Register(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    Fixed(u8),
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
    Not,
    Length,
    BitwiseNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
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
}

#[derive(Debug, Clone)]
pub enum Instruction {
    LoadNil {
        dst: Register,
    },
    LoadBool {
        dst: Register,
        value: bool,
    },
    LoadSmallInt {
        dst: Register,
        value: i16,
    },
    LoadConst {
        dst: Register,
        constant: ConstantIndex,
    },
    Move {
        dst: Register,
        src: Register,
    },
    GetUpvalue {
        dst: Register,
        upvalue: UpvalueIndex,
    },
    SetUpvalue {
        upvalue: UpvalueIndex,
        src: Register,
    },
    /// Allocates an empty table. The hints affect initial capacity only.
    NewTable {
        dst: Register,
        array_hint: u32,
        hash_hint: u32,
    },
    GetTable {
        dst: Register,
        table: Register,
        key: Register,
    },
    SetTable {
        table: Register,
        key: Register,
        value: Register,
    },
    /// Writes consecutive integer keys beginning at the one-based
    /// `first_index`. `Fixed(n)` reads `src..src+n`. `Open` reads
    /// `src..runtime_top`, clears the open state, and resets the runtime top
    /// to `src` after consuming that extent.
    SetList {
        table: Register,
        src: Register,
        first_index: u32,
        count: Count,
    },
    Unary {
        op: UnaryOp,
        dst: Register,
        operand: Register,
    },
    Binary {
        op: BinaryOp,
        dst: Register,
        left: Register,
        right: Register,
    },
    /// Instantiates `children[child]`, binds its upvalues in descriptor order,
    /// and writes the resulting closure to `dst`.
    Closure {
        dst: Register,
        child: PrototypeIndex,
    },
    /// Marks the initialized value in `register` as a to-be-closed local.
    /// False values need no close entry. Every other value must provide
    /// `__close`. `CloseFrom` closes entries in descending register order.
    MarkToClose {
        register: Register,
    },
    /// Closes open upvalues and marked to-be-closed locals whose registers are
    /// greater than or equal to `base`, in descending register order. Open
    /// upvalues are closed before any `__close` methods run.
    CloseFrom {
        base: Register,
    },
    /// Adds the signed `offset` to the program counter after this instruction.
    Jump {
        offset: i32,
    },
    /// Adds `offset` to the program counter after this instruction when
    /// `condition` contains nil or false; every other value falls through.
    JumpIfFalsy {
        condition: Register,
        offset: i32,
    },
    Call {
        base: Register,
        arguments: Count,
        results: Count,
    },
    Vararg {
        base: Register,
        results: Count,
    },
    /// Snapshots the fixed or open result extent beginning at `base`, closes
    /// open upvalues and marked locals from `close_from` when present, and then
    /// returns the saved values. Closing therefore cannot disturb open results;
    /// if closing raises, the return is abandoned.
    Return {
        base: Register,
        values: Count,
        close_from: Option<Register>,
    },
    /// Prepares a numeric-for frame in four consecutive registers:
    /// `base` is the hidden internal index, `base + 1` is the limit,
    /// `base + 2` is the step, and `base + 3` is the user-visible variable.
    /// The VM validates the three numeric controls and rejects a zero step.
    /// When the initial value is within the inclusive limit in the step's
    /// direction, it writes that value to `base + 3` and falls through into
    /// the body. Otherwise it adds the signed `exit_offset` to the program
    /// counter after this instruction.
    ForPrep {
        base: Register,
        exit_offset: i32,
    },
    /// Advances the hidden numeric-for index at `base` by the step at
    /// `base + 2`. When the next value remains within the inclusive limit at
    /// `base + 1`, it writes that value to the visible variable at `base + 3`
    /// and adds the signed `body_offset` to the program counter after this
    /// instruction. Otherwise it falls through without changing the visible
    /// variable. Assigning to `base + 3` never changes iteration control.
    ForLoop {
        base: Register,
        body_offset: i32,
    },
    /// Calls the generic-for iterator stored at `base` with the state at
    /// `base + 1` and control value at `base + 2`. `base + 3` is the hidden
    /// closing value and is left unchanged. Exactly `variables` results are
    /// written beginning at `base + 4`, with normal truncation and nil-fill.
    /// This is an atomic bytecode operation: the VM must keep any call scratch
    /// internally and must not require registers beyond that result window.
    TForCall {
        base: Register,
        variables: u8,
    },
    /// Tests the first generic-for result at `base + 4`. When it is non-nil,
    /// copies it to the hidden control register at `base + 2` and adds the
    /// signed `body_offset` to the program counter after this instruction.
    /// Otherwise falls through to loop-exit cleanup. At least one visible
    /// variable must have been requested by the preceding `TForCall`.
    TForLoop {
        base: Register,
        body_offset: i32,
    },
}
