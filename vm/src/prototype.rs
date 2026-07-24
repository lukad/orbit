use orbit_common::Span;
use orbit_compiler::bytecode::{
    Chunk, Constant, ConstantIndex, Instruction, Prototype, PrototypeIndex, Register,
    UpvalueDescriptor, UpvalueIndex,
};

use crate::{
    error::{FaultResult, VmErrorKind},
    string::LuaString,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RuntimePrototypeIndex(usize);

impl RuntimePrototypeIndex {
    pub(crate) const ENTRY: Self = Self(0);

    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

#[derive(Debug)]
pub(crate) struct PrototypeBundle {
    prototypes: Box<[RuntimePrototype]>,
}

impl PrototypeBundle {
    pub(crate) fn load(chunk: Chunk) -> FaultResult<Self> {
        let Chunk { strings, entry } = chunk;

        let strings = strings
            .into_vec()
            .into_iter()
            .map(LuaString::from)
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let mut prototypes = Vec::new();
        let entry_index = load_prototype(entry, &strings, &mut prototypes)?;

        debug_assert_eq!(entry_index, RuntimePrototypeIndex::ENTRY);

        let prototypes = prototypes
            .into_iter()
            .map(|prototype| {
                prototype.expect("every reserved prototype slot is filled before loading succeeds")
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Ok(Self { prototypes })
    }

    pub(crate) fn entry(&self) -> RuntimePrototypeIndex {
        RuntimePrototypeIndex::ENTRY
    }

    pub(crate) fn prototype(&self, index: RuntimePrototypeIndex) -> Option<&RuntimePrototype> {
        self.prototypes.get(index.get())
    }
}

fn load_prototype(
    prototype: Prototype,
    strings: &[LuaString],
    prototypes: &mut Vec<Option<RuntimePrototype>>,
) -> FaultResult<RuntimePrototypeIndex> {
    if prototype.max_registers < u16::from(prototype.parameter_count) {
        return Err(VmErrorKind::InvalidPrototypeRegisters {
            parameters: prototype.parameter_count,
            registers: prototype.max_registers,
        });
    }

    let index = RuntimePrototypeIndex::new(prototypes.len());

    // Reserve before loading children. This makes the traversal pre-order and
    // guarantees the entry prototype occupies index zero.
    prototypes.push(None);

    let constants = prototype
        .constants
        .into_vec()
        .into_iter()
        .map(|constant| match constant {
            Constant::Integer(value) => Ok(RuntimeConstant::Integer(value)),
            Constant::FloatBits(bits) => Ok(RuntimeConstant::Float(f64::from_bits(bits))),
            Constant::String(string_index) => {
                let raw_index = string_index.get();
                let string = strings
                    .get(raw_index as usize)
                    .cloned()
                    .ok_or(VmErrorKind::InvalidString { string: raw_index })?;

                Ok(RuntimeConstant::String(string))
            }
        })
        .collect::<FaultResult<Vec<_>>>()?
        .into_boxed_slice();

    let upvalues = prototype
        .upvalues
        .into_vec()
        .into_iter()
        .map(|descriptor| match descriptor {
            UpvalueDescriptor::ExternalEnvironment => CaptureDescriptor::ExternalEnvironment,
            UpvalueDescriptor::ParentRegister(register) => {
                CaptureDescriptor::ParentRegister(register)
            }
            UpvalueDescriptor::ParentUpvalue(upvalue) => CaptureDescriptor::ParentUpvalue(upvalue),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();

    let mut children = Vec::new();
    children
        .try_reserve(prototype.children.len())
        .map_err(|_| VmErrorKind::HeapCapacityExceeded {
            requested: prototype.children.len(),
        })?;

    for child in prototype.children.into_vec() {
        children.push(load_prototype(child, strings, prototypes)?);
    }

    let source_map = prototype
        .source_map
        .into_vec()
        .into_iter()
        .map(|entry| (entry.pc, entry.span))
        .collect::<Vec<_>>()
        .into_boxed_slice();

    let runtime_prototype = RuntimePrototype {
        name: prototype.name,
        function_span: prototype.span,
        parameter_count: prototype.parameter_count,
        is_vararg: prototype.is_vararg,
        max_registers: prototype.max_registers,
        constants,
        upvalues,
        children: children.into_boxed_slice(),
        code: prototype.code,
        source_map,
    };

    prototypes[index.get()] = Some(runtime_prototype);

    Ok(index)
}

#[derive(Debug)]
pub(crate) struct RuntimePrototype {
    name: Option<Box<str>>,
    function_span: Span,
    parameter_count: u8,
    is_vararg: bool,
    max_registers: u16,
    constants: Box<[RuntimeConstant]>,
    upvalues: Box<[CaptureDescriptor]>,
    children: Box<[RuntimePrototypeIndex]>,
    code: Box<[Instruction]>,
    source_map: Box<[(u32, Span)]>,
}

impl RuntimePrototype {
    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub(crate) fn function_span(&self) -> Span {
        self.function_span
    }

    pub(crate) fn parameter_count(&self) -> u8 {
        self.parameter_count
    }

    pub(crate) fn is_vararg(&self) -> bool {
        self.is_vararg
    }

    pub(crate) fn max_registers(&self) -> u16 {
        self.max_registers
    }

    pub(crate) fn instruction(&self, pc: usize) -> Option<&Instruction> {
        self.code.get(pc)
    }

    pub(crate) fn constant(&self, index: ConstantIndex) -> Option<&RuntimeConstant> {
        self.constants.get(index.get() as usize)
    }

    pub(crate) fn capture_descriptors(&self) -> &[CaptureDescriptor] {
        &self.upvalues
    }

    pub(crate) fn child(&self, index: PrototypeIndex) -> Option<RuntimePrototypeIndex> {
        self.children.get(index.get() as usize).copied()
    }

    pub(crate) fn code_len(&self) -> usize {
        self.code.len()
    }

    pub(crate) fn instruction_span(&self, pc: usize) -> Option<Span> {
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
pub(crate) enum RuntimeConstant {
    Integer(i64),
    Float(f64),
    String(LuaString),
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CaptureDescriptor {
    ExternalEnvironment,
    ParentRegister(Register),
    ParentUpvalue(UpvalueIndex),
}

#[cfg(test)]
mod tests {
    use orbit_common::{SourceId, Span};
    use orbit_compiler::bytecode::{Chunk, SourceMapEntry};
    use orbit_parser::{lexer::lex, parser::parse_chunk};

    use super::PrototypeBundle;

    fn compile_source(source_id: SourceId, source: &str) -> Chunk {
        let tokens = lex(source_id, source).unwrap();
        let ast = parse_chunk(source_id, tokens).unwrap();
        let hir = orbit_resolver::resolve(&ast).unwrap();

        orbit_compiler::compile(hir).unwrap()
    }

    #[test]
    fn source_map_lookup_respects_transitions_and_code_bounds() {
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

        let bundle = PrototypeBundle::load(chunk).unwrap();
        let prototype = bundle.prototype(bundle.entry()).unwrap();

        assert_eq!(prototype.instruction_span(0), Some(first));
        assert_eq!(prototype.instruction_span(1), Some(first));
        assert_eq!(prototype.instruction_span(2), Some(second));
        assert_eq!(prototype.instruction_span(3), Some(second));
        assert_eq!(prototype.instruction_span(prototype.code_len()), None);

        let mut chunk = compile_source(source_id, "return 1");
        chunk.entry.source_map = Box::new([]);

        let bundle = PrototypeBundle::load(chunk).unwrap();
        let prototype = bundle.prototype(bundle.entry()).unwrap();

        assert_eq!(prototype.instruction_span(0), None);
    }
}
