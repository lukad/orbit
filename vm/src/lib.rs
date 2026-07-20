use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::Rc};

use orbit_common::Span;
use orbit_compiler::bytecode::{
    BinaryOp, Chunk, Constant, Count, Instruction, Prototype, Register, UnaryOp, UpvalueDescriptor,
    UpvalueIndex,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VmErrorKind {
    #[error("program counter out of bounds: {pc}")]
    ProgramCounterOutOfBounds { pc: usize },
    #[error("invalid register: {register}")]
    InvalidRegister { register: u8 },
    #[error("invalid jump offset: {offset}")]
    InvalidJump { offset: i32 },
    #[error("invalid constant index: {constant}")]
    InvalidConstant { constant: u32 },
    #[error("invalid string index: {string}")]
    InvalidString { string: u32 },
    #[error("attempt to get the length of a {kind} value")]
    InvalidLengthOperand { kind: &'static str },
    #[error("string length does not fit in a Lua integer: {length}")]
    StringTooLong { length: usize },
    #[error("attempt to negate a {kind} value")]
    InvalidNegateOperand { kind: &'static str },
    #[error("attempt to perform bitwise not on a {kind} value")]
    InvalidBitwiseOperand { kind: &'static str },
    #[error("attempt to add a {left} value and a {right} value")]
    InvalidAddOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to subtract a {right} value from a {left} value")]
    InvalidSubtractOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to multiply a {left} value by a {right} value")]
    InvalidMultiplyOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to divide a {left} value by a {right} value")]
    InvalidDivideOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to floor-divide a {left} value by a {right} value")]
    InvalidFloorDivideOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to divide an integer by zero")]
    IntegerDivisionByZero,
    #[error("attempt to calculate modulo of a {left} value by a {right} value")]
    InvalidModuloOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to calculate integer modulo by zero")]
    IntegerModuloByZero,
    #[error("attempt to raise a {left} value to a {right} value")]
    InvalidPowerOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to perform {operation} on a {left} value and a {right} value")]
    InvalidBitwiseOperands {
        operation: &'static str,
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to concatenate a {left} value and a {right} value")]
    InvalidConcatOperands {
        left: &'static str,
        right: &'static str,
    },
    #[error("attempt to compare a {left} value with a {right} value using {operation}")]
    InvalidComparisonOperands {
        operation: &'static str,
        left: &'static str,
        right: &'static str,
    },
    #[error("register R{base} cannot be offset by {offset}")]
    InvalidRegisterOffset { base: u8, offset: u8 },
    #[error("numeric for-loop step cannot be zero")]
    ZeroForStep,
    #[error("numeric for-loop control values must be numbers")]
    InvalidForControl,
    #[error("attempt to index a {kind} value")]
    InvalidTableOperand { kind: &'static str },
    #[error("table index is nil")]
    NilTableKey,
    #[error("table index is NaN")]
    NaNTableKey,
    #[error("table is already borrowed")]
    TableBorrowConflict,
    #[error("SetList first index must be at least one, got {first_index}")]
    InvalidListIndex { first_index: u32 },
    #[error("no open result extent is available")]
    MissingOpenResultExtent,
    #[error("a VM value is already borrowed")]
    ValueBorrowConflict,
    #[error("invalid upvalue index: {upvalue}")]
    InvalidUpvalue { upvalue: u32 },
    #[error("invalid child prototype index: {child}")]
    InvalidChildPrototype { child: u32 },
    #[error("entry upvalue {upvalue} tries to capture a parent frame")]
    InvalidEntryUpvalue { upvalue: usize },
    #[error("child prototype {child} upvalue {upvalue} directly captures the external environment")]
    InvalidChildExternalEnvironment { child: u32, upvalue: usize },
    #[error("to-be-closed locals require __close metamethod support")]
    UnsupportedToBeClosedLocal,
    #[error("attempt to call a {kind} value")]
    InvalidCallOperand { kind: &'static str },
    #[error("invalid register range: start {start}, count {count}")]
    InvalidRegisterRange { start: usize, count: usize },
    #[error("prototype declares {parameters} parameters but only provides {registers} registers")]
    InvalidPrototypeRegisters { parameters: u8, registers: u16 },
    #[error("attempt to read varargs from a non-vararg function")]
    InvalidVarargAccess,
    #[error(
        "open results begin at register {result_base}, after requested register {requested_start}"
    )]
    InvalidOpenResultStart {
        requested_start: usize,
        result_base: usize,
    },
    #[error("generic for requires at least one visible variable")]
    InvalidGenericForVariableCount,
}

type FaultResult<T> = Result<T, VmErrorKind>;

/// One frame in a runtime traceback, ordered from the innermost frame outward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmTraceFrame {
    Lua {
        function_span: Span,
        pc: usize,
        instruction_span: Option<Span>,
    },
    Native {
        name: Box<str>,
    },
}

/// A runtime failure together with the Lua/native frames active when it occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmError {
    pub kind: VmErrorKind,
    pub frames: Box<[VmTraceFrame]>,
}

impl VmError {
    pub fn new(kind: VmErrorKind) -> Self {
        Self {
            kind,
            frames: Box::new([]),
        }
    }

    fn with_frames(kind: VmErrorKind, frames: Box<[VmTraceFrame]>) -> Self {
        Self { kind, frames }
    }

    fn append_frames(&mut self, frames: impl IntoIterator<Item = VmTraceFrame>) {
        let mut combined = Vec::from(std::mem::take(&mut self.frames));
        combined.extend(frames);
        self.frames = combined.into_boxed_slice();
    }
}

impl From<VmErrorKind> for VmError {
    fn from(kind: VmErrorKind) -> Self {
        Self::new(kind)
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.kind)?;

        if self.frames.is_empty() {
            return Ok(());
        }

        write!(formatter, "\nstack traceback:")?;

        for frame in &self.frames {
            match frame {
                VmTraceFrame::Lua {
                    function_span,
                    pc,
                    instruction_span,
                } => {
                    let span = instruction_span.unwrap_or(*function_span);
                    write!(
                        formatter,
                        "\n\t[source {} bytes {}..{}, pc {}]",
                        span.source.get(),
                        span.start,
                        span.end,
                        pc
                    )?;
                }
                VmTraceFrame::Native { name } => {
                    write!(formatter, "\n\t[native: {name}]")?;
                }
            }
        }

        Ok(())
    }
}

impl std::error::Error for VmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

pub type VmResult<T> = Result<T, VmError>;

#[derive(Debug)]
struct RuntimePrototype {
    function_span: Span,
    parameter_count: u8,
    is_vararg: bool,
    max_registers: u16,
    constants: Box<[RuntimeConstant]>,
    upvalues: Box<[CaptureDescriptor]>,
    children: Box<[Rc<RuntimePrototype>]>,
    code: Box<[Instruction]>,
    source_map: Box<[(u32, Span)]>,
}

impl RuntimePrototype {
    fn load(prototype: &Prototype, strings: &[Rc<[u8]>]) -> FaultResult<Rc<Self>> {
        let constants = prototype
            .constants
            .iter()
            .map(|constant| match constant {
                Constant::Integer(value) => Ok(RuntimeConstant::Integer(*value)),
                Constant::FloatBits(bits) => Ok(RuntimeConstant::Float(f64::from_bits(*bits))),
                Constant::String(index) => {
                    let index = index.get();

                    let string = strings
                        .get(index as usize)
                        .cloned()
                        .ok_or(VmErrorKind::InvalidString { string: index })?;

                    Ok(RuntimeConstant::String(string))
                }
            })
            .collect::<FaultResult<Vec<_>>>()?
            .into_boxed_slice();

        let upvalues = prototype
            .upvalues
            .iter()
            .map(|descriptor| match descriptor {
                UpvalueDescriptor::ExternalEnvironment => CaptureDescriptor::ExternalEnvironment,
                UpvalueDescriptor::ParentRegister(register) => {
                    CaptureDescriptor::ParentRegister(*register)
                }
                UpvalueDescriptor::ParentUpvalue(upvalue) => {
                    CaptureDescriptor::ParentUpvalue(*upvalue)
                }
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let children = prototype
            .children
            .iter()
            .map(|child| Self::load(child, strings))
            .collect::<FaultResult<Vec<_>>>()?
            .into_boxed_slice();

        let code = prototype.code.to_vec().into_boxed_slice();
        let source_map = prototype
            .source_map
            .iter()
            .map(|entry| (entry.pc, entry.span))
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Ok(Rc::new(Self {
            function_span: prototype.span,
            parameter_count: prototype.parameter_count,
            is_vararg: prototype.is_vararg,
            max_registers: prototype.max_registers,
            constants,
            upvalues,
            children,
            code,
            source_map,
        }))
    }

    fn instruction_span(&self, pc: usize) -> Option<Span> {
        if pc >= self.code.len() {
            return None;
        }

        self.source_map
            .iter()
            .rev()
            .find_map(|(entry_pc, span)| ((*entry_pc as usize) <= pc).then_some(*span))
    }
}

#[derive(Debug, Clone)]
enum RuntimeConstant {
    Integer(i64),
    Float(f64),
    String(Rc<[u8]>),
}

#[derive(Debug, Clone, Copy)]
enum CaptureDescriptor {
    ExternalEnvironment,
    ParentRegister(Register),
    ParentUpvalue(UpvalueIndex),
}

#[derive(Debug, Clone)]
enum TableKey {
    Boolean(bool),
    Integer(i64),
    Float(u64),
    String(Rc<[u8]>),
    Table(TableRef),
    Closure(ClosureRef),
    NativeFunction(NativeFunctionRef),
}

impl Eq for TableKey {}

impl Hash for TableKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);

        match self {
            TableKey::Boolean(value) => value.hash(state),
            TableKey::Integer(value) => value.hash(state),
            TableKey::Float(value) => value.hash(state),
            TableKey::String(value) => value.as_ref().hash(state),
            TableKey::Table(value) => Rc::as_ptr(value).hash(state),
            TableKey::Closure(value) => {
                Rc::as_ptr(value).hash(state);
            }
            TableKey::NativeFunction(value) => {
                Rc::as_ptr(value).hash(state);
            }
        }
    }
}

impl PartialEq for TableKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TableKey::Boolean(a), TableKey::Boolean(b)) => a == b,
            (TableKey::Integer(a), TableKey::Integer(b)) => a == b,
            (TableKey::Float(a), TableKey::Float(b)) => a == b,
            (TableKey::String(a), TableKey::String(b)) => a == b,
            (TableKey::Table(a), TableKey::Table(b)) => Rc::ptr_eq(a, b),
            (TableKey::Closure(a), TableKey::Closure(b)) => Rc::ptr_eq(a, b),
            (TableKey::NativeFunction(a), TableKey::NativeFunction(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

pub struct Table {
    entries: HashMap<TableKey, Value>,
}

impl std::fmt::Debug for Table {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Table")
            .field("len", &self.entries.len())
            .finish_non_exhaustive()
    }
}

type TableRef = Rc<RefCell<Table>>;

#[derive(Clone)]
pub struct Environment {
    table: TableRef,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            table: Rc::new(RefCell::new(Table {
                entries: HashMap::new(),
            })),
        }
    }

    pub fn get(&self, name: impl AsRef<[u8]>) -> VmResult<Value> {
        let key = TableKey::String(Rc::from(name.as_ref()));

        let table = self
            .table
            .try_borrow()
            .map_err(|_| VmErrorKind::TableBorrowConflict)?;

        Ok(table.entries.get(&key).cloned().unwrap_or(Value::Nil))
    }

    pub fn set(&self, name: impl AsRef<[u8]>, value: Value) -> VmResult<()> {
        let key = TableKey::String(Rc::from(name.as_ref()));

        let mut table = self
            .table
            .try_borrow_mut()
            .map_err(|_| VmErrorKind::TableBorrowConflict)?;

        match value {
            Value::Nil => {
                table.entries.remove(&key);
            }

            value => {
                table.entries.insert(key, value);
            }
        }

        Ok(())
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

type ValueCell = Rc<RefCell<Value>>;
type ClosureRef = Rc<Closure>;

pub struct Closure {
    prototype: Rc<RuntimePrototype>,
    upvalues: Box<[ValueCell]>,
}

type NativeFunctionRef = Rc<NativeFunction>;

type NativeFunctionCallback = Box<dyn Fn(&[Value]) -> VmResult<Vec<Value>>>;

pub struct NativeFunction {
    name: Rc<str>,
    callback: NativeFunctionCallback,
}

impl std::fmt::Debug for NativeFunction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeFunction")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Closure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Closure")
            .field("prototype", &Rc::as_ptr(&self.prototype))
            .field("upvalue_count", &self.upvalues.len())
            .finish()
    }
}

#[derive(Clone)]
pub enum Value {
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(Rc<[u8]>),
    Table(TableRef),
    Closure(ClosureRef),
    NativeFunction(NativeFunctionRef),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nil => formatter.write_str("Nil"),
            Self::Boolean(value) => formatter.debug_tuple("Boolean").field(value).finish(),
            Self::Integer(value) => formatter.debug_tuple("Integer").field(value).finish(),
            Self::Float(value) => formatter.debug_tuple("Float").field(value).finish(),
            Self::String(value) => formatter.debug_tuple("String").field(value).finish(),
            Self::Table(value) => formatter
                .debug_struct("Table")
                .field("id", &Rc::as_ptr(value))
                .finish_non_exhaustive(),
            Self::Closure(value) => formatter.debug_tuple("Closure").field(value).finish(),
            Self::NativeFunction(value) => formatter
                .debug_tuple("NativeFunction")
                .field(value)
                .finish(),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        values_equal(self, other)
    }
}

impl Value {
    fn is_falsy(&self) -> bool {
        matches!(self, Value::Nil | Value::Boolean(false))
    }

    fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Boolean(_) => "boolean",
            Value::Integer(_) | Value::Float(_) => "number",
            Value::String(_) => "string",
            Value::Table(_) => "table",
            Value::Closure(_) | Value::NativeFunction(_) => "function",
        }
    }

    fn to_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(value) => Some(*value),
            Value::Float(value) => float_to_integer(*value),
            Value::Nil
            | Value::Boolean(_)
            | Value::String(_)
            | Value::Table(_)
            | Value::Closure(_)
            | Value::NativeFunction(_) => None,
        }
    }

    fn to_float(&self) -> Option<f64> {
        match self {
            Value::Integer(value) => Some(*value as f64),
            Value::Float(value) => Some(*value),
            Value::Nil
            | Value::Boolean(_)
            | Value::String(_)
            | Value::Table(_)
            | Value::Closure(_)
            | Value::NativeFunction(_) => None,
        }
    }

    pub fn native_function(
        name: impl Into<Rc<str>>,
        callback: impl Fn(&[Value]) -> VmResult<Vec<Value>> + 'static,
    ) -> Self {
        Self::NativeFunction(Rc::new(NativeFunction {
            name: name.into(),
            callback: Box::new(callback),
        }))
    }
}

#[derive(Debug, Clone, Copy)]
struct OpenExtent {
    base: usize,
    top: usize,
}

#[derive(Debug, Clone, Copy)]
enum ResultTarget {
    Call { base: usize, results: Count },
    GenericFor { start: usize, variables: usize },
}

enum FrameBoundary {
    Invoke {
        callee: Value,
        arguments: Vec<Value>,
        target: ResultTarget,
    },
    Return(Vec<Value>),
}

struct Activation {
    frame: CallFrame,
    return_to: Option<ResultTarget>,
}

struct Vm {
    stack: Vec<Activation>,
}

struct CallFrame {
    prototype: Rc<RuntimePrototype>,
    upvalues: Box<[ValueCell]>,
    varargs: Box<[Value]>,
    registers: Vec<ValueCell>,
    open_results: Option<OpenExtent>,
    pc: usize,
    current_pc: Option<usize>,
}

impl CallFrame {
    fn new(chunk: &Chunk, environment: &Environment) -> FaultResult<Self> {
        let strings = chunk
            .strings
            .iter()
            .map(|string| Rc::<[u8]>::from(string.as_ref()))
            .collect::<Vec<_>>();

        let prototype = RuntimePrototype::load(&chunk.entry, &strings)?;

        let environment_cell = Rc::new(RefCell::new(Value::Table(Rc::clone(&environment.table))));

        let upvalues = prototype
            .upvalues
            .iter()
            .copied()
            .enumerate()
            .map(|(index, descriptor)| match descriptor {
                CaptureDescriptor::ExternalEnvironment => Ok(Rc::clone(&environment_cell)),
                CaptureDescriptor::ParentRegister(_) | CaptureDescriptor::ParentUpvalue(_) => {
                    Err(VmErrorKind::InvalidEntryUpvalue { upvalue: index })
                }
            })
            .collect::<FaultResult<Vec<_>>>()?
            .into_boxed_slice();

        let registers = (0..usize::from(prototype.max_registers))
            .map(|_| Rc::new(RefCell::new(Value::Nil)))
            .collect();

        Ok(Self {
            prototype,
            upvalues,
            varargs: vec![].into_boxed_slice(),
            registers,
            open_results: None,
            pc: 0,
            current_pc: None,
        })
    }

    fn from_closure(closure: ClosureRef, arguments: Vec<Value>) -> FaultResult<Self> {
        let prototype = Rc::clone(&closure.prototype);
        let parameter_count = usize::from(prototype.parameter_count);
        let register_count = usize::from(prototype.max_registers);

        if parameter_count > register_count {
            return Err(VmErrorKind::InvalidPrototypeRegisters {
                parameters: prototype.parameter_count,
                registers: prototype.max_registers,
            });
        }

        let registers = (0..register_count)
            .map(|index| {
                let value = if index < parameter_count {
                    arguments.get(index).cloned().unwrap_or(Value::Nil)
                } else {
                    Value::Nil
                };

                Rc::new(RefCell::new(value))
            })
            .collect();

        let varargs = if prototype.is_vararg {
            arguments
                .iter()
                .skip(parameter_count)
                .cloned()
                .collect::<Vec<_>>()
                .into_boxed_slice()
        } else {
            Vec::new().into_boxed_slice()
        };

        let upvalues = closure.upvalues.clone();

        Ok(Self {
            prototype,
            upvalues,
            varargs,
            registers,
            open_results: None,
            pc: 0,
            current_pc: None,
        })
    }

    fn ensure_register_capacity(&mut self, required: usize) {
        if self.registers.len() < required {
            self.registers
                .resize_with(required, || Rc::new(RefCell::new(Value::Nil)));
        }
    }

    fn get_register(&self, register: Register) -> FaultResult<Value> {
        let cell =
            self.registers
                .get(usize::from(register.0))
                .ok_or(VmErrorKind::InvalidRegister {
                    register: register.0,
                })?;

        let value = cell
            .try_borrow()
            .map_err(|_| VmErrorKind::ValueBorrowConflict)?;

        Ok(value.clone())
    }

    fn set_register(&mut self, register: Register, value: Value) -> FaultResult<()> {
        let cell =
            self.registers
                .get(usize::from(register.0))
                .ok_or(VmErrorKind::InvalidRegister {
                    register: register.0,
                })?;

        let mut current_value = cell
            .try_borrow_mut()
            .map_err(|_| VmErrorKind::ValueBorrowConflict)?;

        *current_value = value;
        Ok(())
    }

    fn get_registers(&self, base: Register, count: usize) -> FaultResult<Vec<Value>> {
        self.get_register_range(base.0 as usize, count)
    }

    fn get_register_range(&self, start: usize, count: usize) -> FaultResult<Vec<Value>> {
        let end = start
            .checked_add(count)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        let cells = self
            .registers
            .get(start..end)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        cells
            .iter()
            .map(|cell| {
                let value = cell
                    .try_borrow()
                    .map_err(|_| VmErrorKind::ValueBorrowConflict)?;

                Ok(value.clone())
            })
            .collect()
    }

    fn set_register_range(
        &mut self,
        start: usize,
        count: usize,
        values: &[Value],
    ) -> FaultResult<()> {
        let end = start
            .checked_add(count)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        let cells = self
            .registers
            .get_mut(start..end)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        for (index, cell) in cells.iter_mut().enumerate() {
            let value = values.get(index).cloned().unwrap_or(Value::Nil);

            let mut destination = cell
                .try_borrow_mut()
                .map_err(|_| VmErrorKind::ValueBorrowConflict)?;

            *destination = value;
        }

        Ok(())
    }

    fn set_open_results(&mut self, base: usize, values: &[Value]) -> FaultResult<()> {
        let top = base
            .checked_add(values.len())
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: base,
                count: values.len(),
            })?;

        self.ensure_register_capacity(top);
        self.set_register_range(base, values.len(), values)?;
        self.registers
            .truncate(usize::from(self.prototype.max_registers).max(top));

        self.open_results = Some(OpenExtent { base, top });

        Ok(())
    }

    fn reset_open_results(&mut self) {
        self.open_results = None;
        self.registers
            .truncate(usize::from(self.prototype.max_registers));
    }

    fn take_open_results(&mut self, start: usize) -> FaultResult<Vec<Value>> {
        let extent = self
            .open_results
            .take()
            .ok_or(VmErrorKind::MissingOpenResultExtent)?;

        if start > extent.base {
            return Err(VmErrorKind::InvalidOpenResultStart {
                requested_start: start,
                result_base: extent.base,
            });
        }

        let values = self.get_register_range(start, extent.top - start)?;
        self.reset_open_results();
        Ok(values)
    }

    fn get_upvalue(&self, upvalue: UpvalueIndex) -> FaultResult<Value> {
        let index = upvalue.get();

        let cell = self
            .upvalues
            .get(index as usize)
            .ok_or(VmErrorKind::InvalidUpvalue { upvalue: index })?;

        let value = cell
            .try_borrow()
            .map_err(|_| VmErrorKind::ValueBorrowConflict)?;

        Ok(value.clone())
    }

    fn set_upvalue(&mut self, upvalue: UpvalueIndex, value: Value) -> FaultResult<()> {
        let index = upvalue.get();

        let cell = self
            .upvalues
            .get(index as usize)
            .ok_or(VmErrorKind::InvalidUpvalue { upvalue: index })?;

        let mut current_value = cell
            .try_borrow_mut()
            .map_err(|_| VmErrorKind::ValueBorrowConflict)?;

        *current_value = value;
        Ok(())
    }

    fn close_upvalues_from(&mut self, base: Register) -> FaultResult<()> {
        let start = usize::from(base.0);

        let registers = self
            .registers
            .get_mut(start..)
            .ok_or(VmErrorKind::InvalidRegister { register: base.0 })?;

        for cell in registers {
            let value = {
                let value = cell
                    .try_borrow()
                    .map_err(|_| VmErrorKind::ValueBorrowConflict)?;

                value.clone()
            };

            *cell = Rc::new(RefCell::new(value));
        }

        Ok(())
    }

    fn apply_jump(&mut self, offset: i32) -> FaultResult<()> {
        let target = self
            .pc
            .checked_add_signed(offset as isize)
            .filter(|target| *target < self.prototype.code.len())
            .ok_or(VmErrorKind::InvalidJump { offset })?;

        self.pc = target;
        Ok(())
    }

    fn accept_results(&mut self, target: ResultTarget, values: &[Value]) -> FaultResult<()> {
        match target {
            ResultTarget::Call { base, results } => match results {
                Count::Fixed(count) => {
                    self.reset_open_results();
                    self.set_register_range(base, usize::from(count), values)
                }
                Count::Open => self.set_open_results(base, values),
            },
            ResultTarget::GenericFor { start, variables } => {
                self.reset_open_results();
                self.set_register_range(start, variables, values)
            }
        }
    }

    fn trace_frame(&self) -> VmTraceFrame {
        let pc = self.current_pc.unwrap_or(self.pc);

        VmTraceFrame::Lua {
            function_span: self.prototype.function_span,
            pc,
            instruction_span: self.prototype.instruction_span(pc),
        }
    }

    fn run_until_boundary(&mut self) -> FaultResult<FrameBoundary> {
        loop {
            let prototype = Rc::clone(&self.prototype);

            self.current_pc = Some(self.pc);

            let instruction = prototype
                .code
                .get(self.pc)
                .ok_or(VmErrorKind::ProgramCounterOutOfBounds { pc: self.pc })?;

            self.pc += 1;

            match instruction {
                Instruction::LoadNil { dst } => self.set_register(*dst, Value::Nil)?,
                Instruction::LoadBool { dst, value } => {
                    self.set_register(*dst, Value::Boolean(*value))?
                }
                Instruction::LoadSmallInt { dst, value } => {
                    self.set_register(*dst, Value::Integer(*value as i64))?
                }
                Instruction::LoadConst { dst, constant } => {
                    let constant_index = constant.get();

                    let Some(constant) = prototype.constants.get(constant_index as usize) else {
                        return Err(VmErrorKind::InvalidConstant {
                            constant: constant_index,
                        });
                    };

                    let value = match constant {
                        RuntimeConstant::Integer(value) => Value::Integer(*value),
                        RuntimeConstant::Float(value) => Value::Float(*value),
                        RuntimeConstant::String(value) => Value::String(Rc::clone(value)),
                    };

                    self.set_register(*dst, value)?;
                }
                Instruction::Move { dst, src } => {
                    let value = self.get_register(*src)?;
                    self.set_register(*dst, value)?
                }
                Instruction::GetUpvalue { dst, upvalue } => {
                    let value = self.get_upvalue(*upvalue)?;
                    self.set_register(*dst, value)?;
                }
                Instruction::SetUpvalue { upvalue, src } => {
                    let value = self.get_register(*src)?;
                    self.set_upvalue(*upvalue, value)?;
                }
                Instruction::Vararg { base, results } => {
                    if !prototype.is_vararg {
                        return Err(VmErrorKind::InvalidVarargAccess);
                    }

                    let values = self.varargs.to_vec();
                    let base = usize::from(base.0);

                    match results {
                        Count::Fixed(count) => {
                            self.reset_open_results();
                            self.set_register_range(base, usize::from(*count), &values)?;
                        }
                        Count::Open => {
                            self.set_open_results(base, &values)?;
                        }
                    }
                }
                Instruction::Closure { dst, child } => {
                    let child_index = child.get();

                    let child_prototype = prototype
                        .children
                        .get(child_index as usize)
                        .cloned()
                        .ok_or(VmErrorKind::InvalidChildPrototype { child: child_index })?;

                    let captured_upvalues = child_prototype
                        .upvalues
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(upvalue_index, descriptor)| match descriptor {
                            CaptureDescriptor::ParentRegister(register) => {
                                self.registers.get(usize::from(register.0)).cloned().ok_or(
                                    VmErrorKind::InvalidRegister {
                                        register: register.0,
                                    },
                                )
                            }
                            CaptureDescriptor::ParentUpvalue(upvalue) => {
                                let index = upvalue.get();

                                self.upvalues
                                    .get(index as usize)
                                    .cloned()
                                    .ok_or(VmErrorKind::InvalidUpvalue { upvalue: index })
                            }
                            CaptureDescriptor::ExternalEnvironment => {
                                Err(VmErrorKind::InvalidChildExternalEnvironment {
                                    child: child_index,
                                    upvalue: upvalue_index,
                                })
                            }
                        })
                        .collect::<FaultResult<Vec<_>>>()?
                        .into_boxed_slice();

                    let closure = Rc::new(Closure {
                        prototype: child_prototype,
                        upvalues: captured_upvalues,
                    });

                    self.set_register(*dst, Value::Closure(closure))?;
                }
                Instruction::Call {
                    base,
                    arguments,
                    results,
                } => {
                    let callee = self.get_register(*base)?;
                    let base = usize::from(base.0);
                    let argument_start = base + 1;

                    let arguments = match arguments {
                        Count::Fixed(count) => {
                            self.get_register_range(argument_start, usize::from(*count))?
                        }
                        Count::Open => self.take_open_results(argument_start)?,
                    };

                    return Ok(FrameBoundary::Invoke {
                        callee,
                        arguments,
                        target: ResultTarget::Call {
                            base,
                            results: *results,
                        },
                    });
                }
                Instruction::Return {
                    base,
                    values,
                    close_from,
                } => {
                    let base = usize::from(base.0);

                    let values = match values {
                        Count::Fixed(count) => {
                            self.get_register_range(base, usize::from(*count))?
                        }
                        Count::Open => self.take_open_results(base)?,
                    };

                    if let Some(close_from) = close_from {
                        self.close_upvalues_from(*close_from)?;
                    }
                    return Ok(FrameBoundary::Return(values));
                }
                Instruction::CloseFrom { base } => {
                    self.close_upvalues_from(*base)?;
                }
                Instruction::MarkToClose { register } => {
                    let value = self.get_register(*register)?;

                    if !value.is_falsy() {
                        return Err(VmErrorKind::UnsupportedToBeClosedLocal);
                    }
                }
                Instruction::Jump { offset } => {
                    self.apply_jump(*offset)?;
                }
                Instruction::JumpIfFalsy { condition, offset } => {
                    if self.get_register(*condition)?.is_falsy() {
                        self.apply_jump(*offset)?;
                    }
                }
                Instruction::Unary {
                    op: UnaryOp::Not,
                    dst,
                    operand,
                } => {
                    let operand = self.get_register(*operand)?;
                    let result = Value::Boolean(operand.is_falsy());
                    self.set_register(*dst, result)?;
                }
                Instruction::Unary {
                    op: UnaryOp::Length,
                    dst,
                    operand,
                } => {
                    let operand = self.get_register(*operand)?;

                    let result = match operand {
                        Value::String(string) => {
                            let length = i64::try_from(string.len()).map_err(|_| {
                                VmErrorKind::StringTooLong {
                                    length: string.len(),
                                }
                            })?;
                            Value::Integer(length)
                        }
                        value => {
                            return Err(VmErrorKind::InvalidLengthOperand {
                                kind: value.type_name(),
                            });
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::Unary {
                    op: UnaryOp::Negate,
                    dst,
                    operand,
                } => {
                    let operand = self.get_register(*operand)?;

                    let result = match operand {
                        Value::Integer(value) => Value::Integer(value.wrapping_neg()),
                        Value::Float(value) => Value::Float(-value),
                        value => {
                            return Err(VmErrorKind::InvalidNegateOperand {
                                kind: value.type_name(),
                            });
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::Unary {
                    op: UnaryOp::BitwiseNot,
                    dst,
                    operand,
                } => {
                    let operand = self.get_register(*operand)?;
                    let integer =
                        operand
                            .to_integer()
                            .ok_or(VmErrorKind::InvalidBitwiseOperand {
                                kind: operand.type_name(),
                            })?;

                    self.set_register(*dst, Value::Integer(!integer))?;
                }
                Instruction::Binary {
                    op: BinaryOp::Add,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let result = match (left, right) {
                        (Value::Integer(left), Value::Integer(right)) => {
                            Value::Integer(left.wrapping_add(right))
                        }
                        (Value::Integer(left), Value::Float(right)) => {
                            Value::Float(left as f64 + right)
                        }
                        (Value::Float(left), Value::Integer(right)) => {
                            Value::Float(left + right as f64)
                        }
                        (Value::Float(left), Value::Float(right)) => Value::Float(left + right),
                        (left, right) => {
                            return Err(VmErrorKind::InvalidAddOperands {
                                left: left.type_name(),
                                right: right.type_name(),
                            });
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::Binary {
                    op: BinaryOp::Subtract,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let result = match (left, right) {
                        (Value::Integer(left), Value::Integer(right)) => {
                            Value::Integer(left.wrapping_sub(right))
                        }
                        (Value::Integer(left), Value::Float(right)) => {
                            Value::Float(left as f64 - right)
                        }
                        (Value::Float(left), Value::Integer(right)) => {
                            Value::Float(left - right as f64)
                        }
                        (Value::Float(left), Value::Float(right)) => Value::Float(left - right),
                        (left, right) => {
                            return Err(VmErrorKind::InvalidSubtractOperands {
                                left: left.type_name(),
                                right: right.type_name(),
                            });
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::Binary {
                    op: BinaryOp::Multiply,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let result = match (left, right) {
                        (Value::Integer(left), Value::Integer(right)) => {
                            Value::Integer(left.wrapping_mul(right))
                        }
                        (Value::Integer(left), Value::Float(right)) => {
                            Value::Float(left as f64 * right)
                        }
                        (Value::Float(left), Value::Integer(right)) => {
                            Value::Float(left * right as f64)
                        }
                        (Value::Float(left), Value::Float(right)) => Value::Float(left * right),
                        (left, right) => {
                            return Err(VmErrorKind::InvalidMultiplyOperands {
                                left: left.type_name(),
                                right: right.type_name(),
                            });
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::Binary {
                    op: BinaryOp::Divide,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(left_number), Some(right_number)) =
                        (left.to_float(), right.to_float())
                    else {
                        return Err(VmErrorKind::InvalidDivideOperands {
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    let result = Value::Float(left_number / right_number);
                    self.set_register(*dst, result)?;
                }
                Instruction::Binary {
                    op: BinaryOp::FloorDivide,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let result = match (left, right) {
                        (Value::Integer(left), Value::Integer(right)) => {
                            if right == 0 {
                                return Err(VmErrorKind::IntegerDivisionByZero);
                            }

                            let quotient = if right == -1 {
                                left.wrapping_neg()
                            } else {
                                let quotient = left / right;
                                let remainder = left % right;

                                if remainder != 0 && (left < 0) != (right < 0) {
                                    quotient - 1
                                } else {
                                    quotient
                                }
                            };

                            Value::Integer(quotient)
                        }
                        (Value::Integer(left), Value::Float(right)) => {
                            Value::Float((left as f64 / right).floor())
                        }
                        (Value::Float(left), Value::Integer(right)) => {
                            Value::Float((left / right as f64).floor())
                        }
                        (Value::Float(left), Value::Float(right)) => {
                            Value::Float((left / right).floor())
                        }
                        (left, right) => {
                            return Err(VmErrorKind::InvalidFloorDivideOperands {
                                left: left.type_name(),
                                right: right.type_name(),
                            });
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::Binary {
                    op: BinaryOp::Modulo,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let result = match (left, right) {
                        (Value::Integer(left), Value::Integer(right)) => {
                            if right == 0 {
                                return Err(VmErrorKind::IntegerModuloByZero);
                            }

                            let remainder = if right == -1 {
                                0
                            } else {
                                let remainder = left % right;

                                if remainder != 0 && (remainder < 0) != (right < 0) {
                                    remainder + right
                                } else {
                                    remainder
                                }
                            };

                            Value::Integer(remainder)
                        }
                        (Value::Integer(left), Value::Float(right)) => {
                            Value::Float(float_modulo(left as f64, right))
                        }
                        (Value::Float(left), Value::Integer(right)) => {
                            Value::Float(float_modulo(left, right as f64))
                        }
                        (Value::Float(left), Value::Float(right)) => {
                            Value::Float(float_modulo(left, right))
                        }
                        (left, right) => {
                            return Err(VmErrorKind::InvalidModuloOperands {
                                left: left.type_name(),
                                right: right.type_name(),
                            });
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::Binary {
                    op: BinaryOp::Power,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(left_number), Some(right_number)) =
                        (left.to_float(), right.to_float())
                    else {
                        return Err(VmErrorKind::InvalidPowerOperands {
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    self.set_register(*dst, Value::Float(left_number.powf(right_number)))?;
                }
                Instruction::Binary {
                    op: BinaryOp::BitwiseAnd,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(left_integer), Some(right_integer)) =
                        (left.to_integer(), right.to_integer())
                    else {
                        return Err(VmErrorKind::InvalidBitwiseOperands {
                            operation: "bitwise and",
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    self.set_register(*dst, Value::Integer(left_integer & right_integer))?;
                }
                Instruction::Binary {
                    op: BinaryOp::BitwiseOr,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(left_integer), Some(right_integer)) =
                        (left.to_integer(), right.to_integer())
                    else {
                        return Err(VmErrorKind::InvalidBitwiseOperands {
                            operation: "bitwise or",
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    self.set_register(*dst, Value::Integer(left_integer | right_integer))?;
                }
                Instruction::Binary {
                    op: BinaryOp::BitwiseXor,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(left_integer), Some(right_integer)) =
                        (left.to_integer(), right.to_integer())
                    else {
                        return Err(VmErrorKind::InvalidBitwiseOperands {
                            operation: "bitwise xor",
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    self.set_register(*dst, Value::Integer(left_integer ^ right_integer))?;
                }
                Instruction::Binary {
                    op: BinaryOp::ShiftLeft,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(value), Some(distance)) = (left.to_integer(), right.to_integer())
                    else {
                        return Err(VmErrorKind::InvalidBitwiseOperands {
                            operation: "left shift",
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    self.set_register(*dst, Value::Integer(shift_left(value, distance)))?;
                }
                Instruction::Binary {
                    op: BinaryOp::ShiftRight,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(value), Some(distance)) = (left.to_integer(), right.to_integer())
                    else {
                        return Err(VmErrorKind::InvalidBitwiseOperands {
                            operation: "right shift",
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    self.set_register(*dst, Value::Integer(shift_right(value, distance)))?;
                }
                Instruction::Binary {
                    op: BinaryOp::Concat,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    let (Some(mut bytes), Some(right_bytes)) =
                        (concat_bytes(&left), concat_bytes(&right))
                    else {
                        return Err(VmErrorKind::InvalidConcatOperands {
                            left: left.type_name(),
                            right: right.type_name(),
                        });
                    };

                    bytes.extend_from_slice(&right_bytes);
                    self.set_register(*dst, Value::String(Rc::from(bytes)))?;
                }
                Instruction::Binary {
                    op: BinaryOp::Equal,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    self.set_register(*dst, Value::Boolean(values_equal(&left, &right)))?;
                }
                Instruction::Binary {
                    op: BinaryOp::NotEqual,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;

                    self.set_register(*dst, Value::Boolean(!values_equal(&left, &right)))?;
                }
                Instruction::Binary {
                    op: BinaryOp::LessThan,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;
                    let result = values_less_than(&left, &right).ok_or(
                        VmErrorKind::InvalidComparisonOperands {
                            operation: "<",
                            left: left.type_name(),
                            right: right.type_name(),
                        },
                    )?;

                    self.set_register(*dst, Value::Boolean(result))?;
                }
                Instruction::Binary {
                    op: BinaryOp::LessEqual,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;
                    let result = values_less_equal(&left, &right).ok_or(
                        VmErrorKind::InvalidComparisonOperands {
                            operation: "<=",
                            left: left.type_name(),
                            right: right.type_name(),
                        },
                    )?;

                    self.set_register(*dst, Value::Boolean(result))?;
                }
                Instruction::Binary {
                    op: BinaryOp::GreaterThan,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;
                    let result = values_less_than(&right, &left).ok_or(
                        VmErrorKind::InvalidComparisonOperands {
                            operation: ">",
                            left: left.type_name(),
                            right: right.type_name(),
                        },
                    )?;

                    self.set_register(*dst, Value::Boolean(result))?;
                }
                Instruction::Binary {
                    op: BinaryOp::GreaterEqual,
                    dst,
                    left,
                    right,
                } => {
                    let left = self.get_register(*left)?;
                    let right = self.get_register(*right)?;
                    let result = values_less_equal(&right, &left).ok_or(
                        VmErrorKind::InvalidComparisonOperands {
                            operation: ">=",
                            left: left.type_name(),
                            right: right.type_name(),
                        },
                    )?;

                    self.set_register(*dst, Value::Boolean(result))?;
                }
                Instruction::ForPrep { base, exit_offset } => {
                    let index_register = *base;
                    let limit_register = offset_register(*base, 1)?;
                    let step_register = offset_register(*base, 2)?;
                    let visible_register = offset_register(*base, 3)?;

                    let initial = self.get_register(index_register)?;
                    let limit = self.get_register(limit_register)?;
                    let step = self.get_register(step_register)?;

                    match (&initial, &step) {
                        (Value::Integer(initial), Value::Integer(step)) => {
                            let initial = *initial;
                            let step = *step;

                            if step == 0 {
                                return Err(VmErrorKind::ZeroForStep);
                            }

                            let Some(limit) = integer_for_limit(&limit, step)? else {
                                self.apply_jump(*exit_offset)?;
                                continue;
                            };

                            self.set_register(limit_register, Value::Integer(limit))?;

                            let enters = if step > 0 {
                                initial <= limit
                            } else {
                                initial >= limit
                            };

                            if enters {
                                self.set_register(visible_register, Value::Integer(initial))?;
                            } else {
                                self.apply_jump(*exit_offset)?;
                            }
                        }
                        _ => {
                            let initial =
                                initial.to_float().ok_or(VmErrorKind::InvalidForControl)?;
                            let limit = limit.to_float().ok_or(VmErrorKind::InvalidForControl)?;
                            let step = step.to_float().ok_or(VmErrorKind::InvalidForControl)?;

                            if step == 0.0 {
                                return Err(VmErrorKind::ZeroForStep);
                            }

                            self.set_register(index_register, Value::Float(initial))?;
                            self.set_register(limit_register, Value::Float(limit))?;
                            self.set_register(step_register, Value::Float(step))?;

                            let enters = if step > 0.0 {
                                initial <= limit
                            } else {
                                initial >= limit
                            };

                            if enters {
                                self.set_register(visible_register, Value::Float(initial))?;
                            } else {
                                self.apply_jump(*exit_offset)?;
                            }
                        }
                    }
                }
                Instruction::ForLoop { base, body_offset } => {
                    let index_register = *base;
                    let limit_register = offset_register(*base, 1)?;
                    let step_register = offset_register(*base, 2)?;
                    let visible_register = offset_register(*base, 3)?;

                    let index = self.get_register(index_register)?;
                    let limit = self.get_register(limit_register)?;
                    let step = self.get_register(step_register)?;

                    match (index, limit, step) {
                        (Value::Integer(index), Value::Integer(limit), Value::Integer(step)) => {
                            if step == 0 {
                                return Err(VmErrorKind::ZeroForStep);
                            }

                            let Some(next) = index.checked_add(step) else {
                                continue;
                            };

                            self.set_register(index_register, Value::Integer(next))?;

                            let continues = if step > 0 {
                                next <= limit
                            } else {
                                next >= limit
                            };

                            if continues {
                                self.set_register(visible_register, Value::Integer(next))?;
                                self.apply_jump(*body_offset)?;
                            }
                        }

                        (index, limit, step) => {
                            let index = index.to_float().ok_or(VmErrorKind::InvalidForControl)?;
                            let limit = limit.to_float().ok_or(VmErrorKind::InvalidForControl)?;
                            let step = step.to_float().ok_or(VmErrorKind::InvalidForControl)?;

                            if step == 0.0 {
                                return Err(VmErrorKind::ZeroForStep);
                            }

                            let next = index + step;
                            self.set_register(index_register, Value::Float(next))?;

                            let continues = if step > 0.0 {
                                next <= limit
                            } else {
                                next >= limit
                            };

                            if continues {
                                self.set_register(visible_register, Value::Float(next))?;
                                self.apply_jump(*body_offset)?;
                            }
                        }
                    }
                }
                Instruction::TForCall { base, variables } => {
                    if *variables == 0 {
                        return Err(VmErrorKind::InvalidGenericForVariableCount);
                    }

                    let state_register = offset_register(*base, 1)?;
                    let control_register = offset_register(*base, 2)?;

                    let iterator = self.get_register(*base)?;
                    let state = self.get_register(state_register)?;
                    let control = self.get_register(control_register)?;

                    let result_start = usize::from(base.0) + 4;

                    return Ok(FrameBoundary::Invoke {
                        callee: iterator,
                        arguments: vec![state, control],
                        target: ResultTarget::GenericFor {
                            start: result_start,
                            variables: usize::from(*variables),
                        },
                    });
                }
                Instruction::TForLoop { base, body_offset } => {
                    let control_register = offset_register(*base, 2)?;
                    let first_result_register = offset_register(*base, 4)?;

                    let first_result = self.get_register(first_result_register)?;

                    if !matches!(first_result, Value::Nil) {
                        self.set_register(control_register, first_result)?;
                        self.apply_jump(*body_offset)?;
                    }
                }
                Instruction::NewTable {
                    dst,
                    array_hint,
                    hash_hint,
                } => {
                    let capacity = (*array_hint as usize).saturating_add(*hash_hint as usize);

                    let table = Table {
                        entries: HashMap::with_capacity(capacity),
                    };

                    self.set_register(*dst, Value::Table(Rc::new(RefCell::new(table))))?;
                }
                Instruction::SetTable { table, key, value } => {
                    let table = self.get_register(*table)?;

                    let table = match table {
                        Value::Table(table) => table,
                        value => {
                            return Err(VmErrorKind::InvalidTableOperand {
                                kind: value.type_name(),
                            });
                        }
                    };

                    let key = self.get_register(*key)?;
                    let key = table_key(key)?;

                    let value = self.get_register(*value)?;

                    let mut table = table
                        .try_borrow_mut()
                        .map_err(|_| VmErrorKind::TableBorrowConflict)?;

                    match value {
                        Value::Nil => {
                            table.entries.remove(&key);
                        }
                        value => {
                            table.entries.insert(key, value);
                        }
                    }
                }
                Instruction::GetTable { dst, table, key } => {
                    let table = self.get_register(*table)?;

                    let table = match table {
                        Value::Table(table) => table,
                        value => {
                            return Err(VmErrorKind::InvalidTableOperand {
                                kind: value.type_name(),
                            });
                        }
                    };

                    let key = self.get_register(*key)?;
                    let key = table_lookup_key(key);

                    let result = match key {
                        None => Value::Nil,
                        Some(key) => {
                            let table = table
                                .try_borrow()
                                .map_err(|_| VmErrorKind::TableBorrowConflict)?;
                            table.entries.get(&key).cloned().unwrap_or(Value::Nil)
                        }
                    };

                    self.set_register(*dst, result)?;
                }
                Instruction::SetList {
                    table,
                    src,
                    first_index,
                    count,
                } => {
                    if *first_index == 0 {
                        return Err(VmErrorKind::InvalidListIndex {
                            first_index: *first_index,
                        });
                    }

                    let table = self.get_register(*table)?;

                    let table = match table {
                        Value::Table(table) => table,
                        value => {
                            return Err(VmErrorKind::InvalidTableOperand {
                                kind: value.type_name(),
                            });
                        }
                    };

                    let values = match count {
                        Count::Fixed(count) => self.get_registers(*src, usize::from(*count))?,
                        Count::Open => self.take_open_results(usize::from(src.0))?,
                    };

                    let mut table = table
                        .try_borrow_mut()
                        .map_err(|_| VmErrorKind::TableBorrowConflict)?;

                    for (offset, value) in values.into_iter().enumerate() {
                        let index = i64::from(*first_index) + offset as i64;
                        let key = TableKey::Integer(index);

                        match value {
                            Value::Nil => {
                                table.entries.remove(&key);
                            }

                            value => {
                                table.entries.insert(key, value);
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Vm {
    fn new(frame: CallFrame) -> Self {
        Self {
            stack: vec![Activation {
                frame,
                return_to: None,
            }],
        }
    }

    fn trace_frames(&self, native: Option<Box<str>>) -> Vec<VmTraceFrame> {
        let mut frames = Vec::with_capacity(self.stack.len() + usize::from(native.is_some()));

        if let Some(name) = native {
            frames.push(VmTraceFrame::Native { name });
        }

        frames.extend(
            self.stack
                .iter()
                .rev()
                .map(|activation| activation.frame.trace_frame()),
        );

        frames
    }

    fn runtime_error(&self, kind: VmErrorKind, native: Option<Box<str>>) -> VmError {
        VmError::with_frames(kind, self.trace_frames(native).into_boxed_slice())
    }

    fn attach_trace(&self, mut error: VmError, native: Option<Box<str>>) -> VmError {
        error.append_frames(self.trace_frames(native));
        error
    }

    fn run(mut self) -> VmResult<Vec<Value>> {
        loop {
            let boundary = {
                let activation = self.stack.last_mut().expect("VM stack is never empty");
                activation.frame.run_until_boundary()
            };

            let boundary = match boundary {
                Ok(boundary) => boundary,
                Err(kind) => return Err(self.runtime_error(kind, None)),
            };

            match boundary {
                FrameBoundary::Invoke {
                    callee,
                    arguments,
                    target,
                } => match callee {
                    Value::Closure(closure) => {
                        let frame = match CallFrame::from_closure(closure, arguments) {
                            Ok(frame) => frame,
                            Err(kind) => return Err(self.runtime_error(kind, None)),
                        };

                        self.stack.push(Activation {
                            frame,
                            return_to: Some(target),
                        });
                    }
                    Value::NativeFunction(function) => {
                        let values = match (function.callback)(&arguments) {
                            Ok(values) => values,
                            Err(error) => {
                                let name = function.name.as_ref().into();
                                return Err(self.attach_trace(error, Some(name)));
                            }
                        };

                        let result = self
                            .stack
                            .last_mut()
                            .expect("caller frame remains active")
                            .frame
                            .accept_results(target, &values);

                        if let Err(kind) = result {
                            return Err(self.runtime_error(kind, None));
                        }
                    }
                    value => {
                        return Err(self.runtime_error(
                            VmErrorKind::InvalidCallOperand {
                                kind: value.type_name(),
                            },
                            None,
                        ));
                    }
                },
                FrameBoundary::Return(values) => {
                    let activation = self.stack.pop().expect("returning frame is active");

                    let Some(target) = activation.return_to else {
                        debug_assert!(self.stack.is_empty());
                        return Ok(values);
                    };

                    let result = self
                        .stack
                        .last_mut()
                        .expect("non-entry frame has a caller")
                        .frame
                        .accept_results(target, &values);

                    if let Err(kind) = result {
                        return Err(self.runtime_error(kind, None));
                    }
                }
            }
        }
    }
}

pub fn execute(chunk: &Chunk, environment: &Environment) -> VmResult<Vec<Value>> {
    let frame = CallFrame::new(chunk, environment).map_err(VmError::from)?;
    Vm::new(frame).run()
}

fn table_key(value: Value) -> FaultResult<TableKey> {
    match value {
        Value::Nil => Err(VmErrorKind::NilTableKey),
        Value::Boolean(value) => Ok(TableKey::Boolean(value)),
        Value::Integer(value) => Ok(TableKey::Integer(value)),
        Value::Float(value) => {
            if value.is_nan() {
                return Err(VmErrorKind::NaNTableKey);
            }

            if let Some(integer) = float_to_integer(value) {
                Ok(TableKey::Integer(integer))
            } else {
                Ok(TableKey::Float(value.to_bits()))
            }
        }
        Value::String(value) => Ok(TableKey::String(value)),
        Value::Table(value) => Ok(TableKey::Table(value)),
        Value::Closure(closure) => Ok(TableKey::Closure(closure)),
        Value::NativeFunction(function) => Ok(TableKey::NativeFunction(function)),
    }
}

fn table_lookup_key(value: Value) -> Option<TableKey> {
    match value {
        Value::Nil => None,
        Value::Boolean(value) => Some(TableKey::Boolean(value)),
        Value::Integer(value) => Some(TableKey::Integer(value)),
        Value::Float(value) => {
            if value.is_nan() {
                return None;
            }

            if let Some(integer) = float_to_integer(value) {
                Some(TableKey::Integer(integer))
            } else {
                Some(TableKey::Float(value.to_bits()))
            }
        }
        Value::String(value) => Some(TableKey::String(value)),
        Value::Table(value) => Some(TableKey::Table(value)),
        Value::Closure(closure) => Some(TableKey::Closure(closure)),
        Value::NativeFunction(function) => Some(TableKey::NativeFunction(function)),
    }
}

fn offset_register(base: Register, offset: u8) -> FaultResult<Register> {
    let register = base
        .0
        .checked_add(offset)
        .ok_or(VmErrorKind::InvalidRegisterOffset {
            base: base.0,
            offset,
        })?;

    Ok(Register(register))
}

fn integer_for_limit(limit: &Value, step: i64) -> FaultResult<Option<i64>> {
    match limit {
        Value::Integer(limit) => Ok(Some(*limit)),
        Value::Float(limit) => {
            if limit.is_nan() {
                return Ok(None);
            }

            let minimum = i64::MIN as f64;
            let exclusive_maximum = -(i64::MIN as f64);

            if step > 0 {
                if *limit < minimum {
                    Ok(None)
                } else if *limit >= exclusive_maximum {
                    Ok(Some(i64::MAX))
                } else {
                    Ok(Some(limit.floor() as i64))
                }
            } else if *limit >= exclusive_maximum {
                Ok(None)
            } else if *limit < minimum {
                Ok(Some(i64::MIN))
            } else {
                Ok(Some(limit.ceil() as i64))
            }
        }
        Value::Nil
        | Value::Boolean(_)
        | Value::String(_)
        | Value::Table(_)
        | Value::Closure(_)
        | Value::NativeFunction(_) => Err(VmErrorKind::InvalidForControl),
    }
}

fn float_modulo(left: f64, right: f64) -> f64 {
    let mut remainder = left % right;

    if (remainder > 0.0 && right < 0.0) || (remainder < 0.0 && right > 0.0) {
        remainder += right;
    }

    remainder
}

fn float_to_integer(value: f64) -> Option<i64> {
    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if value.is_finite() && value.fract() == 0.0 && value >= minimum && value < exclusive_maximum {
        Some(value as i64)
    } else {
        None
    }
}

fn shift_left(value: i64, distance: i64) -> i64 {
    if !(-63..=63).contains(&distance) {
        return 0;
    }

    if distance < 0 {
        ((value as u64) >> distance.unsigned_abs() as u32) as i64
    } else {
        ((value as u64) << distance as u32) as i64
    }
}

fn shift_right(value: i64, distance: i64) -> i64 {
    if !(-63..=63).contains(&distance) {
        return 0;
    }

    if distance < 0 {
        ((value as u64) << distance.unsigned_abs() as u32) as i64
    } else {
        ((value as u64) >> distance as u32) as i64
    }
}

fn concat_bytes(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::String(value) => Some(value.to_vec()),
        Value::Integer(value) => Some(value.to_string().into_bytes()),
        Value::Float(value) => Some(format_lua_float(*value).into_bytes()),
        Value::Nil
        | Value::Boolean(_)
        | Value::Table(_)
        | Value::Closure(_)
        | Value::NativeFunction(_) => None,
    }
}

fn format_general_float(value: f64, significant_digits: usize) -> String {
    debug_assert!(value.is_finite());
    debug_assert!(significant_digits > 0);

    let scientific = format!("{value:.precision$e}", precision = significant_digits - 1);
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("LowerExp output always has an exponent");
    let exponent = exponent
        .parse::<i32>()
        .expect("LowerExp output always has a decimal exponent");
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');

    if exponent < -4 || exponent >= significant_digits as i32 {
        return format!("{mantissa}e{exponent:+03}");
    }

    let (sign, coefficient) = mantissa
        .strip_prefix('-')
        .map_or(("", mantissa), |coefficient| ("-", coefficient));
    let digits = coefficient.replace('.', "");
    let decimal_position = exponent + 1;
    let mut formatted = String::from(sign);

    if decimal_position <= 0 {
        formatted.push_str("0.");
        formatted.extend(std::iter::repeat_n('0', (-decimal_position) as usize));
        formatted.push_str(&digits);
    } else if decimal_position as usize >= digits.len() {
        formatted.push_str(&digits);
        formatted.extend(std::iter::repeat_n(
            '0',
            decimal_position as usize - digits.len(),
        ));
    } else {
        let decimal_position = decimal_position as usize;
        formatted.push_str(&digits[..decimal_position]);
        formatted.push('.');
        formatted.push_str(&digits[decimal_position..]);
    }

    formatted
}

fn format_lua_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }

    if value == f64::INFINITY {
        return "inf".to_owned();
    }

    if value == f64::NEG_INFINITY {
        return "-inf".to_owned();
    }

    let mut formatted = format_general_float(value, 15);

    if formatted
        .parse::<f64>()
        .expect("generated float representation must parse")
        != value
    {
        formatted = format_general_float(value, 17);
    }

    if !formatted.contains('.') && !formatted.contains('e') {
        formatted.push_str(".0");
    }

    formatted
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Nil, Value::Nil) => true,
        (Value::Boolean(left), Value::Boolean(right)) => left == right,
        (Value::Integer(left), Value::Integer(right)) => left == right,
        (Value::Float(left), Value::Float(right)) => left == right,
        (Value::Integer(left), Value::Float(right)) => float_to_integer(*right) == Some(*left),
        (Value::Float(left), Value::Integer(right)) => float_to_integer(*left) == Some(*right),
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Table(left), Value::Table(right)) => Rc::ptr_eq(left, right),
        (Value::Closure(left), Value::Closure(right)) => Rc::ptr_eq(left, right),
        (Value::NativeFunction(left), Value::NativeFunction(right)) => Rc::ptr_eq(left, right),
        _ => false,
    }
}

fn values_less_than(left: &Value, right: &Value) -> Option<bool> {
    match (left, right) {
        (Value::Integer(left), Value::Integer(right)) => Some(left < right),
        (Value::Float(left), Value::Float(right)) => Some(left < right),
        (Value::Integer(left), Value::Float(right)) => Some(integer_less_float(*left, *right)),
        (Value::Float(left), Value::Integer(right)) => Some(float_less_integer(*left, *right)),
        (Value::String(left), Value::String(right)) => Some(left.as_ref() < right.as_ref()),
        _ => None,
    }
}

fn values_less_equal(left: &Value, right: &Value) -> Option<bool> {
    match (left, right) {
        (Value::Integer(left), Value::Integer(right)) => Some(left <= right),
        (Value::Float(left), Value::Float(right)) => Some(left <= right),
        (Value::Integer(left), Value::Float(right)) => {
            Some(integer_less_equal_float(*left, *right))
        }
        (Value::Float(left), Value::Integer(right)) => {
            Some(float_less_equal_integer(*left, *right))
        }
        (Value::String(left), Value::String(right)) => Some(left.as_ref() <= right.as_ref()),
        _ => None,
    }
}

fn integer_less_float(integer: i64, float: f64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float >= exclusive_maximum {
        true
    } else if float < minimum {
        false
    } else if float.fract() == 0.0 {
        integer < float as i64
    } else {
        integer <= float.floor() as i64
    }
}

fn integer_less_equal_float(integer: i64, float: f64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float >= exclusive_maximum {
        true
    } else if float < minimum {
        false
    } else {
        integer <= float.floor() as i64
    }
}

fn float_less_integer(float: f64, integer: i64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float < minimum {
        true
    } else if float >= exclusive_maximum {
        false
    } else {
        (float.floor() as i64) < integer
    }
}

fn float_less_equal_integer(float: f64, integer: i64) -> bool {
    if float.is_nan() {
        return false;
    }

    let minimum = i64::MIN as f64;
    let exclusive_maximum = -(i64::MIN as f64);

    if float < minimum {
        true
    } else if float >= exclusive_maximum {
        false
    } else {
        float.ceil() as i64 <= integer
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::SourceId;
    use orbit_compiler::bytecode::SourceMapEntry;
    use orbit_parser::{lexer::lex, parser::parse_chunk};

    use super::*;

    fn compile_source(source_id: SourceId, source: &str) -> Chunk {
        let tokens = lex(source_id, source).unwrap();
        let ast = parse_chunk(source_id, &tokens).unwrap();
        let hir = orbit_resolver::resolve(&ast).unwrap();
        orbit_compiler::compile(hir).unwrap()
    }

    fn execute_source(source: &str) -> VmResult<Vec<Value>> {
        let chunk = compile_source(SourceId::new(0), source);
        execute(&chunk, &Environment::new())
    }

    fn assert_execute(source: &str, expected: Vec<Value>) {
        let actual = execute_source(source).unwrap();

        assert_eq!(actual, expected, "source:\n{source}");
    }

    fn string_value(value: &str) -> Value {
        Value::String(Rc::from(value.as_bytes()))
    }

    fn source_span(source_id: SourceId, source: &str, needle: &str) -> Span {
        let start = source.find(needle).unwrap();
        Span::new(
            source_id,
            u32::try_from(start).unwrap(),
            u32::try_from(start + needle.len()).unwrap(),
        )
    }

    #[test]
    fn executes_unary_operators_with_lua_truthiness() {
        assert_execute(
            r#"
                return not nil, not false, not 0, #"orbit", -(-5), ~5
            "#,
            vec![
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Integer(5),
                Value::Integer(5),
                Value::Integer(-6),
            ],
        );
    }

    #[test]
    fn executes_integer_and_mixed_numeric_arithmetic() {
        assert_execute(
            r#"
                return 7 + 3, 7 - 10, 6 * 7, 7 / 2, -7 // 3, -7 % 3, 2 ^ 3
            "#,
            vec![
                Value::Integer(10),
                Value::Integer(-3),
                Value::Integer(42),
                Value::Float(3.5),
                Value::Integer(-3),
                Value::Integer(2),
                Value::Float(8.0),
            ],
        );
    }

    #[test]
    fn executes_bitwise_operators_and_reversed_shifts() {
        assert_execute(
            r#"
                return 6 & 3, 6 | 3, 6 ~ 3, 1 << 4, 16 >> 2, 8 << -1, 8 >> -1,
                    3.0 & 1
            "#,
            vec![
                Value::Integer(2),
                Value::Integer(7),
                Value::Integer(5),
                Value::Integer(16),
                Value::Integer(4),
                Value::Integer(4),
                Value::Integer(16),
                Value::Integer(1),
            ],
        );
    }

    #[test]
    fn concatenates_numbers_and_compares_mixed_numeric_values() {
        assert_execute(
            r#"
                return "orbit" .. 42 .. 3.5, 1 == 1.0, 2 ~= 3, 1 < 1.5,
                    2.5 <= 2, "a" < "b", 3 >= 3.0
            "#,
            vec![
                string_value("orbit423.5"),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Boolean(true),
            ],
        );
    }

    #[test]
    fn short_circuit_operators_preserve_values_and_skip_operands() {
        assert_execute(
            r#"
                local calls = 0

                local function mark(value)
                    calls = calls + 1
                    return value
                end

                local a = false and mark(1)
                local b = true or mark(2)
                local c = nil or mark(3)
                local d = 0 and mark(4)

                if false then
                    calls = 100
                elseif nil then
                    calls = 200
                else
                    calls = calls + 10
                end

                return a, b, c, d, calls
            "#,
            vec![
                Value::Boolean(false),
                Value::Boolean(true),
                Value::Integer(3),
                Value::Integer(4),
                Value::Integer(12),
            ],
        );
    }

    #[test]
    fn executes_while_repeat_and_break_control_flow() {
        assert_execute(
            r#"
                local index = 0
                local total = 0

                while index < 10 do
                    index = index + 1
                    if index == 4 then
                        break
                    end
                    total = total + index
                end

                repeat
                    total = total + 1
                until total >= 10

                return index, total
            "#,
            vec![Value::Integer(4), Value::Integer(10)],
        );
    }

    #[test]
    fn executes_ascending_descending_and_float_numeric_for_loops() {
        assert_execute(
            r#"
                local integer_total = 0
                for value = 1, 5 do
                    integer_total = integer_total + value
                end
                for value = 5, 1, -2 do
                    integer_total = integer_total + value
                end

                local float_total = 0.0
                for value = 0.5, 1.0, 0.25 do
                    float_total = float_total + value
                end

                return integer_total, float_total
            "#,
            vec![Value::Integer(24), Value::Float(2.25)],
        );
    }

    #[test]
    fn executes_generic_for_iterators_with_multiple_visible_values() {
        assert_execute(
            r#"
                local function iterator(limit, control)
                    local next_value = control + 1
                    if next_value > limit then
                        return nil
                    end
                    return next_value, next_value * next_value
                end

                local total = 0
                for index, square in iterator, 4, 0 do
                    total = total + index + square
                end

                return total
            "#,
            vec![Value::Integer(40)],
        );
    }

    #[test]
    fn table_reads_writes_deletes_and_canonicalizes_numeric_keys() {
        assert_execute(
            r#"
                local values = { 10, 20, name = "orbit", [true] = 30 }
                values[2] = nil
                values[1.0] = 11
                values[3] = values[1] + values[true]

                return values[1], values[2], values[3], values.name, values[0 / 0]
            "#,
            vec![
                Value::Integer(11),
                Value::Nil,
                Value::Integer(41),
                string_value("orbit"),
                Value::Nil,
            ],
        );
    }

    #[test]
    fn tables_use_identity_for_table_keys_and_equality() {
        assert_execute(
            r#"
                local keys = {}
                local first = {}
                local second = {}

                keys[first] = "first"
                keys[second] = "second"

                return keys[first], keys[second], first == first, first == second
            "#,
            vec![
                string_value("first"),
                string_value("second"),
                Value::Boolean(true),
                Value::Boolean(false),
            ],
        );
    }

    #[test]
    fn closes_captured_numeric_for_variables_between_iterations() {
        assert_execute(
            r#"
                local functions = {}

                for index = 1, 3 do
                    functions[index] = function()
                        return index
                    end
                end

                return functions[1](), functions[2](), functions[3]()
            "#,
            vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)],
        );
    }

    #[test]
    fn globals_are_shared_with_nested_closures() {
        assert_execute(
            r#"
                answer = 41

                local function read_answer()
                    return answer + 1
                end

                return read_answer(), missing
            "#,
            vec![Value::Integer(42), Value::Nil],
        );
    }

    #[test]
    fn false_to_be_closed_values_are_noops() {
        assert_execute(
            r#"
                local value <close> = false
                return 42
            "#,
            vec![Value::Integer(42)],
        );
    }

    #[test]
    fn fills_missing_parameters_with_nil() {
        assert_execute(
            r#"
                local function values(a, b, c)
                    return a, b, c
                end

                return values(10)
            "#,
            vec![Value::Integer(10), Value::Nil, Value::Nil],
        );
    }

    #[test]
    fn discards_extra_arguments_for_non_vararg_functions() {
        assert_execute(
            r#"
                local function first(value)
                    return value
                end

                return first(10, 20, 30)
            "#,
            vec![Value::Integer(10)],
        );
    }

    #[test]
    fn nil_fills_missing_fixed_results() {
        assert_execute(
            r#"
                local function one()
                    return 7
                end

                local a, b, c = one()
                return a, b, c
            "#,
            vec![Value::Integer(7), Value::Nil, Value::Nil],
        );
    }

    #[test]
    fn closures_share_mutated_upvalues() {
        assert_execute(
            r#"
                local value = 0

                local function increment()
                    value = value + 1
                    return value
                end

                return increment(), increment()
            "#,
            vec![Value::Integer(1), Value::Integer(2)],
        );
    }

    #[test]
    fn nested_closures_forward_parent_upvalue_cells() {
        assert_execute(
            r#"
                local value = 42

                local function make_middle()
                    return function()
                        return function()
                            return value
                        end
                    end
                end

                local middle = make_middle()
                local inner = middle()

                return inner()
            "#,
            vec![Value::Integer(42)],
        );
    }

    #[test]
    fn closing_a_scope_detaches_reused_registers() {
        assert_execute(
            r#"
                local first, second

                do
                    local value = 1
                    first = function()
                        return value
                    end
                end

                do
                    local value = 2
                    second = function()
                        return value
                    end
                end

                return first(), second()
            "#,
            vec![Value::Integer(1), Value::Integer(2)],
        );
    }

    #[test]
    fn fixed_vararg_reads_nil_fill_missing_values() {
        assert_execute(
            r#"
                local function collect(first, ...)
                    local a, b, c = ...
                    return first, a, b, c
                end

                return collect(1, 2, 3)
            "#,
            vec![
                Value::Integer(1),
                Value::Integer(2),
                Value::Integer(3),
                Value::Nil,
            ],
        );
    }

    #[test]
    fn open_vararg_returns_preserve_every_value() {
        assert_execute(
            r#"
                local function identity(...)
                    return ...
                end

                return identity(1, 2, 3, 4)
            "#,
            vec![
                Value::Integer(1),
                Value::Integer(2),
                Value::Integer(3),
                Value::Integer(4),
            ],
        );
    }

    #[test]
    fn open_results_become_outer_call_arguments() {
        assert_execute(
            r#"
                local function values()
                    return 20, 22
                end

                local function add(left, right)
                    return left + right
                end

                return add(values())
            "#,
            vec![Value::Integer(42)],
        );
    }

    #[test]
    fn open_results_expand_the_final_table_list_field() {
        assert_execute(
            r#"
                local function values()
                    return 2, 3
                end

                local result = { 1, values() }

                return result[1], result[2], result[3]
            "#,
            vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)],
        );
    }

    #[test]
    fn calling_a_non_function_returns_an_error() {
        let error = execute_source("return (42)()").unwrap_err();

        assert!(matches!(
            error.kind,
            VmErrorKind::InvalidCallOperand { kind: "number" }
        ));
    }

    #[test]
    fn invalid_unary_operands_return_typed_errors() {
        let error = execute_source("return #true").unwrap_err();
        assert!(matches!(
            error.kind,
            VmErrorKind::InvalidLengthOperand { kind: "boolean" }
        ));

        let error = execute_source("return -false").unwrap_err();
        assert!(matches!(
            error.kind,
            VmErrorKind::InvalidNegateOperand { kind: "boolean" }
        ));

        let error = execute_source("return ~1.5").unwrap_err();
        assert!(matches!(
            error.kind,
            VmErrorKind::InvalidBitwiseOperand { kind: "number" }
        ));
    }

    #[test]
    fn invalid_binary_operands_return_typed_errors() {
        let error = execute_source(r#"return 1 + "value""#).unwrap_err();
        assert!(matches!(
            error.kind,
            VmErrorKind::InvalidAddOperands {
                left: "number",
                right: "string"
            }
        ));

        let error = execute_source("return 1 & 1.5").unwrap_err();
        assert!(matches!(
            error.kind,
            VmErrorKind::InvalidBitwiseOperands {
                operation: "bitwise and",
                left: "number",
                right: "number"
            }
        ));

        let error = execute_source("return true < false").unwrap_err();
        assert!(matches!(
            error.kind,
            VmErrorKind::InvalidComparisonOperands {
                operation: "<",
                left: "boolean",
                right: "boolean"
            }
        ));
    }

    #[test]
    fn integer_floor_division_and_modulo_by_zero_return_errors() {
        let error = execute_source("return 1 // 0").unwrap_err();
        assert!(matches!(error.kind, VmErrorKind::IntegerDivisionByZero));

        let error = execute_source("return 1 % 0").unwrap_err();
        assert!(matches!(error.kind, VmErrorKind::IntegerModuloByZero));
    }

    #[test]
    fn invalid_table_keys_return_errors() {
        let error = execute_source(
            r#"
                local values = {}
                values[nil] = 1
            "#,
        )
        .unwrap_err();
        assert!(matches!(error.kind, VmErrorKind::NilTableKey));

        let error = execute_source(
            r#"
                local values = {}
                values[0 / 0] = 1
            "#,
        )
        .unwrap_err();
        assert!(matches!(error.kind, VmErrorKind::NaNTableKey));
    }

    #[test]
    fn zero_numeric_for_step_returns_an_error() {
        let error = execute_source(
            r#"
                for value = 1, 3, 0 do
                end
            "#,
        )
        .unwrap_err();

        assert!(matches!(error.kind, VmErrorKind::ZeroForStep));
    }

    #[test]
    fn truthy_to_be_closed_values_report_unsupported_metamethods() {
        let error = execute_source(
            r#"
                local value <close> = true
            "#,
        )
        .unwrap_err();

        assert!(matches!(
            error.kind,
            VmErrorKind::UnsupportedToBeClosedLocal
        ));
    }

    #[test]
    fn debug_formatting_does_not_follow_table_cycles() {
        let values = execute_source(
            r#"
                local table = {}
                table.self = table
                table[table] = table
                return table
            "#,
        )
        .unwrap();

        let rendered = format!("{values:?}");
        assert!(rendered.contains("Table"));
        assert!(rendered.len() < 256);

        let Value::Table(table) = &values[0] else {
            panic!("expected table");
        };

        assert_eq!(format!("{:?}", table.borrow()), "Table { len: 2, .. }");
    }

    #[test]
    fn deep_lua_calls_use_the_explicit_vm_stack() {
        assert_execute(
            r#"
                local function descend(value)
                    if value == 0 then
                        return 42
                    end

                    return descend(value - 1)
                end

                return descend(20000)
            "#,
            vec![Value::Integer(42)],
        );
    }

    #[test]
    fn consuming_open_results_releases_the_dynamic_register_tail() {
        let chunk = compile_source(SourceId::new(0), "return");
        let environment = Environment::new();
        let mut frame = CallFrame::new(&chunk, &environment).unwrap();
        let declared_registers = frame.registers.len();
        let values = vec![Value::String(Rc::from(&b"payload"[..])); 32];

        frame.set_open_results(declared_registers, &values).unwrap();
        assert_eq!(frame.registers.len(), declared_registers + values.len());

        let consumed = frame.take_open_results(declared_registers).unwrap();
        assert_eq!(consumed.len(), values.len());
        assert_eq!(frame.registers.len(), declared_registers);
        assert!(frame.open_results.is_none());
    }

    #[test]
    fn replacing_open_results_releases_the_previous_dynamic_tail() {
        let chunk = compile_source(SourceId::new(0), "return");
        let environment = Environment::new();
        let mut frame = CallFrame::new(&chunk, &environment).unwrap();
        let declared_registers = frame.registers.len();
        let retained = Rc::new(RefCell::new(Table {
            entries: HashMap::new(),
        }));
        let weak = Rc::downgrade(&retained);
        let values = vec![Value::Table(Rc::clone(&retained)); 32];

        frame.set_open_results(declared_registers, &values).unwrap();
        drop(values);
        drop(retained);
        assert!(weak.upgrade().is_some());

        frame
            .set_open_results(declared_registers, &[Value::Integer(1)])
            .unwrap();

        assert_eq!(frame.registers.len(), declared_registers + 1);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn resumes_lua_after_successful_native_calls() {
        let environment = Environment::new();
        environment
            .set(
                "values",
                Value::native_function("values", |_| {
                    Ok(vec![Value::Integer(20), Value::Integer(22)])
                }),
            )
            .unwrap();
        environment
            .set(
                "count",
                Value::native_function("count", |arguments| {
                    Ok(vec![Value::Integer(arguments.len() as i64)])
                }),
            )
            .unwrap();

        let chunk = compile_source(
            SourceId::new(0),
            r#"
                local first, second, missing = values()
                return first, second, missing, count(values())
            "#,
        );

        assert_eq!(
            execute(&chunk, &environment).unwrap(),
            vec![
                Value::Integer(20),
                Value::Integer(22),
                Value::Nil,
                Value::Integer(2),
            ]
        );
    }

    #[test]
    fn resumes_generic_for_after_a_successful_native_call() {
        let environment = Environment::new();
        environment
            .set(
                "iterator",
                Value::native_function("iterator", |arguments| {
                    let [Value::Integer(limit), Value::Integer(control)] = arguments else {
                        return Err(VmErrorKind::InvalidForControl.into());
                    };
                    let next = control + 1;

                    if next > *limit {
                        Ok(vec![Value::Nil])
                    } else {
                        Ok(vec![Value::Integer(next), Value::Integer(next * next)])
                    }
                }),
            )
            .unwrap();

        let chunk = compile_source(
            SourceId::new(0),
            r#"
                local total = 0
                for index, square in iterator, 4, 0 do
                    total = total + index + square
                end
                return total
            "#,
        );

        assert_eq!(
            execute(&chunk, &environment).unwrap(),
            vec![Value::Integer(40)]
        );
    }

    #[test]
    fn formats_floats_like_lua_5_5() {
        let cases = [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1e-4, "0.0001"),
            (1e-5, "1e-05"),
            (1e-7, "1e-07"),
            (1e14, "100000000000000.0"),
            (1e15, "1e+15"),
            (1e20, "1e+20"),
            (std::f64::consts::PI, "3.1415926535897931"),
            (1.23456789012345, "1.23456789012345"),
            (1.2345678901234567, "1.2345678901234567"),
            (9_007_199_254_740_992.0, "9007199254740992.0"),
            (f64::from_bits(1), "4.94065645841247e-324"),
            (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
            (f64::NAN, "nan"),
        ];

        for (value, expected) in cases {
            assert_eq!(format_lua_float(value), expected, "value: {value:?}");
        }

        assert_execute(
            r#"
                return 1e20 .. "", 1e-7 .. "", (0 / 0) .. "",
                    (1 / 0) .. "", (-1 / 0) .. ""
            "#,
            vec![
                string_value("1e+20"),
                string_value("1e-07"),
                string_value("nan"),
                string_value("inf"),
                string_value("-inf"),
            ],
        );
    }

    #[test]
    fn runtime_errors_retain_exact_source_maps_across_chunks() {
        let environment = Environment::new();
        let failing_source = "function failing()\n    return 1 + true\nend";
        let middle_source = "function middle()\n    return failing()\nend";
        let calling_source = "return middle()";

        {
            let defining_chunk = compile_source(SourceId::new(1), failing_source);
            assert!(matches!(
                defining_chunk.entry.children[0].code[2],
                Instruction::Binary {
                    op: BinaryOp::Add,
                    ..
                }
            ));
            execute(&defining_chunk, &environment).unwrap();
        }

        {
            let middle_chunk = compile_source(SourceId::new(2), middle_source);
            assert!(matches!(
                middle_chunk.entry.children[0].code[3],
                Instruction::Call { .. }
            ));
            execute(&middle_chunk, &environment).unwrap();
        }

        let calling_chunk = compile_source(SourceId::new(3), calling_source);
        assert!(matches!(
            calling_chunk.entry.code[3],
            Instruction::Call { .. }
        ));
        let error = execute(&calling_chunk, &environment).unwrap_err();

        assert!(matches!(error.kind, VmErrorKind::InvalidAddOperands { .. }));
        assert_eq!(error.frames.len(), 3);

        let VmTraceFrame::Lua {
            function_span,
            pc,
            instruction_span,
        } = &error.frames[0]
        else {
            panic!("expected innermost Lua frame");
        };
        assert_eq!(function_span.source, SourceId::new(1));
        assert_eq!(*pc, 2);
        assert_eq!(
            *instruction_span,
            Some(source_span(SourceId::new(1), failing_source, "1 + true"))
        );

        let VmTraceFrame::Lua {
            function_span,
            pc,
            instruction_span,
        } = &error.frames[1]
        else {
            panic!("expected middle Lua frame");
        };
        assert_eq!(function_span.source, SourceId::new(2));
        assert_eq!(*pc, 3);
        assert_eq!(
            *instruction_span,
            Some(source_span(SourceId::new(2), middle_source, "failing()"))
        );

        let VmTraceFrame::Lua {
            function_span,
            pc,
            instruction_span,
        } = &error.frames[2]
        else {
            panic!("expected outermost Lua frame");
        };
        assert_eq!(function_span.source, SourceId::new(3));
        assert_eq!(*pc, 3);
        assert_eq!(
            *instruction_span,
            Some(source_span(SourceId::new(3), calling_source, "middle()"))
        );
    }

    #[test]
    fn source_map_lookup_respects_transitions_empty_maps_and_code_bounds() {
        let source_id = SourceId::new(11);
        let first = Span::new(source_id, 10, 20);
        let second = Span::new(source_id, 30, 40);
        let mut chunk = compile_source(source_id, "return 1, 2, 3");
        assert!(chunk.entry.code.len() >= 4);
        chunk.entry.source_map = vec![
            SourceMapEntry { pc: 0, span: first },
            SourceMapEntry {
                pc: 2,
                span: second,
            },
        ]
        .into_boxed_slice();

        let environment = Environment::new();
        let frame = CallFrame::new(&chunk, &environment).unwrap();
        assert_eq!(frame.prototype.instruction_span(0), Some(first));
        assert_eq!(frame.prototype.instruction_span(1), Some(first));
        assert_eq!(frame.prototype.instruction_span(2), Some(second));
        assert_eq!(frame.prototype.instruction_span(3), Some(second));
        assert_eq!(
            frame.prototype.instruction_span(frame.prototype.code.len()),
            None
        );

        let mut chunk = compile_source(source_id, "return 1");
        chunk.entry.source_map = Box::new([]);
        let frame = CallFrame::new(&chunk, &environment).unwrap();
        assert_eq!(frame.prototype.instruction_span(0), None);

        chunk.entry.code = Box::new([]);
        chunk.entry.source_map = vec![SourceMapEntry { pc: 0, span: first }].into_boxed_slice();
        let error = execute(&chunk, &environment).unwrap_err();
        assert!(matches!(
            error.kind,
            VmErrorKind::ProgramCounterOutOfBounds { pc: 0 }
        ));
        assert!(matches!(
            error.frames.as_ref(),
            [VmTraceFrame::Lua {
                pc: 0,
                instruction_span: None,
                ..
            }]
        ));
    }

    #[test]
    fn native_errors_include_the_function_name_and_call_site() {
        let environment = Environment::new();
        environment
            .set(
                "explode",
                Value::native_function("explode", |_| {
                    Err(VmErrorKind::IntegerDivisionByZero.into())
                }),
            )
            .unwrap();

        let chunk = compile_source(SourceId::new(7), "return explode()");
        let error = execute(&chunk, &environment).unwrap_err();

        assert!(matches!(error.kind, VmErrorKind::IntegerDivisionByZero));
        assert_eq!(error.frames.len(), 2);
        assert!(matches!(
            &error.frames[0],
            VmTraceFrame::Native { name } if name.as_ref() == "explode"
        ));
        assert!(matches!(
            &error.frames[1],
            VmTraceFrame::Lua {
                function_span,
                pc: 3,
                instruction_span: Some(instruction_span),
            } if function_span.source == SourceId::new(7)
                && *instruction_span
                    == source_span(SourceId::new(7), "return explode()", "explode()")
        ));
        assert!(error.to_string().contains("[native: explode]"));

        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<VmError>();
    }

    #[test]
    fn native_errors_preserve_nested_vm_frames() {
        let nested_source = "return 1 + true";
        let nested_chunk = compile_source(SourceId::new(20), nested_source);
        let nested_environment = Environment::new();
        let environment = Environment::new();
        environment
            .set(
                "nested",
                Value::native_function("nested", move |_| {
                    execute(&nested_chunk, &nested_environment)
                }),
            )
            .unwrap();

        let outer_source = "return nested()";
        let outer_chunk = compile_source(SourceId::new(21), outer_source);
        let error = execute(&outer_chunk, &environment).unwrap_err();

        assert!(matches!(error.kind, VmErrorKind::InvalidAddOperands { .. }));
        assert!(matches!(
            error.frames.as_ref(),
            [
                VmTraceFrame::Lua {
                    function_span: nested_function,
                    pc: 2,
                    instruction_span: Some(nested_instruction),
                },
                VmTraceFrame::Native { name },
                VmTraceFrame::Lua {
                    function_span: outer_function,
                    pc: 3,
                    instruction_span: Some(outer_instruction),
                },
            ] if nested_function.source == SourceId::new(20)
                && *nested_instruction
                    == source_span(SourceId::new(20), nested_source, "1 + true")
                && name.as_ref() == "nested"
                && outer_function.source == SourceId::new(21)
                && *outer_instruction
                    == source_span(SourceId::new(21), outer_source, "nested()")
        ));
    }
}
