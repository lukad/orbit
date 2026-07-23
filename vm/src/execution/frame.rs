use std::rc::Rc;

use orbit_compiler::bytecode::{
    ConstantIndex, Count, Instruction, PrototypeIndex, Register, UpvalueIndex,
};

use crate::{
    error::{FaultResult, VmErrorKind, VmTraceFrame},
    function::LuaInvocation,
    id::{FunctionId, ObjectId, UpvalueId},
    prototype::{
        CaptureDescriptor, PrototypeBundle, RuntimeConstant, RuntimePrototype,
        RuntimePrototypeIndex,
    },
    runtime::Runtime,
    upvalue::RegisterSlot,
    value::RawValue,
};

use super::activation::{OpenExtent, ResultTarget};

pub(crate) struct CallFrame {
    bundle: Rc<PrototypeBundle>,
    prototype: RuntimePrototypeIndex,
    upvalues: Box<[UpvalueId]>,
    varargs: Box<[RawValue]>,
    registers: Vec<RegisterSlot>,
    declared_registers: usize,
    open_results: Option<OpenExtent>,
    pc: usize,
    current_pc: Option<usize>,
}

impl CallFrame {
    pub(crate) fn new(invocation: LuaInvocation, arguments: &[RawValue]) -> FaultResult<Self> {
        let (bundle, prototype, upvalues) = invocation.into_parts();

        let (parameter_count, is_vararg, declared_registers, expected_upvalues) = {
            let prototype = bundle
                .prototype(prototype)
                .ok_or_else(|| invalid_prototype(prototype))?;

            (
                usize::from(prototype.parameter_count()),
                prototype.is_vararg(),
                usize::from(prototype.max_registers()),
                prototype.capture_descriptors().len(),
            )
        };

        if parameter_count > declared_registers {
            return Err(VmErrorKind::InvalidPrototypeRegisters {
                parameters: u8::try_from(parameter_count).unwrap_or(u8::MAX),
                registers: u16::try_from(declared_registers).unwrap_or(u16::MAX),
            });
        }

        if upvalues.len() != expected_upvalues {
            return Err(VmErrorKind::InvalidClosureUpvalueCount {
                expected: expected_upvalues,
                actual: upvalues.len(),
            });
        }

        let mut registers = Vec::new();
        registers.try_reserve(declared_registers).map_err(|_| {
            VmErrorKind::FrameCapacityExceeded {
                requested: declared_registers,
            }
        })?;

        for index in 0..declared_registers {
            let value = if index < parameter_count {
                arguments.get(index).cloned().unwrap_or(RawValue::Nil)
            } else {
                RawValue::Nil
            };

            registers.push(RegisterSlot::direct(value));
        }

        let varargs = if is_vararg {
            arguments
                .get(parameter_count..)
                .unwrap_or(&[])
                .to_vec()
                .into_boxed_slice()
        } else {
            Box::new([])
        };

        Ok(Self {
            bundle,
            prototype,
            upvalues,
            varargs,
            registers,
            declared_registers,
            open_results: None,
            pc: 0,
            current_pc: None,
        })
    }

    pub(crate) fn next_instruction(&mut self) -> FaultResult<Instruction> {
        let pc = self.pc;
        self.current_pc = Some(pc);

        let instruction = self
            .runtime_prototype()
            .instruction(pc)
            .cloned()
            .ok_or(VmErrorKind::ProgramCounterOutOfBounds { pc })?;

        self.pc = self
            .pc
            .checked_add(1)
            .ok_or(VmErrorKind::ProgramCounterOutOfBounds { pc })?;

        Ok(instruction)
    }

    pub(crate) fn get_register(
        &self,
        runtime: &Runtime,
        register: Register,
    ) -> FaultResult<RawValue> {
        let slot =
            self.registers
                .get(usize::from(register.0))
                .ok_or(VmErrorKind::InvalidRegister {
                    register: register.0,
                })?;

        read_slot(runtime, slot)
    }

    pub(crate) fn set_register(
        &mut self,
        runtime: &mut Runtime,
        register: Register,
        value: RawValue,
    ) -> FaultResult<()> {
        let slot = self.registers.get_mut(usize::from(register.0)).ok_or(
            VmErrorKind::InvalidRegister {
                register: register.0,
            },
        )?;

        write_slot(runtime, slot, value)
    }

    pub(crate) fn get_register_range(
        &self,
        runtime: &Runtime,
        start: usize,
        count: usize,
    ) -> FaultResult<Box<[RawValue]>> {
        let end = start
            .checked_add(count)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        let slots = self
            .registers
            .get(start..end)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        let mut values = Vec::new();
        values
            .try_reserve(count)
            .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested: count })?;

        for slot in slots {
            values.push(read_slot(runtime, slot)?);
        }

        Ok(values.into_boxed_slice())
    }

    pub(crate) fn set_register_range(
        &mut self,
        runtime: &mut Runtime,
        start: usize,
        count: usize,
        values: &[RawValue],
    ) -> FaultResult<()> {
        let end = start
            .checked_add(count)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        let slots = self
            .registers
            .get_mut(start..end)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        for (index, slot) in slots.iter_mut().enumerate() {
            let value = values.get(index).cloned().unwrap_or(RawValue::Nil);

            write_slot(runtime, slot, value)?;
        }

        Ok(())
    }

    pub(crate) fn get_upvalue(
        &self,
        runtime: &Runtime,
        upvalue: UpvalueIndex,
    ) -> FaultResult<RawValue> {
        let upvalue = self.upvalue_id(upvalue)?;
        runtime.read_upvalue(upvalue)
    }

    pub(crate) fn set_upvalue(
        &self,
        runtime: &mut Runtime,
        upvalue: UpvalueIndex,
        value: RawValue,
    ) -> FaultResult<()> {
        let upvalue = self.upvalue_id(upvalue)?;
        runtime.write_upvalue(upvalue, value)?;
        Ok(())
    }

    pub(crate) fn constant(&self, constant: ConstantIndex) -> FaultResult<RawValue> {
        let raw_index = constant.get();

        let constant =
            self.runtime_prototype()
                .constant(constant)
                .ok_or(VmErrorKind::InvalidConstant {
                    constant: raw_index,
                })?;

        Ok(match constant {
            RuntimeConstant::Integer(value) => RawValue::Integer(*value),
            RuntimeConstant::Float(value) => RawValue::Float(*value),
            RuntimeConstant::String(value) => RawValue::String(value.clone()),
        })
    }

    pub(crate) fn capture_register(
        &mut self,
        runtime: &mut Runtime,
        register: Register,
    ) -> FaultResult<UpvalueId> {
        let index = usize::from(register.0);

        let slot = self
            .registers
            .get(index)
            .ok_or(VmErrorKind::InvalidRegister {
                register: register.0,
            })?;

        if let Some(upvalue) = slot.captured_id() {
            return Ok(upvalue);
        }

        let value = slot
            .direct_value()
            .expect("uncaptured slot contains a direct value")
            .clone();

        let upvalue = runtime.allocate_upvalue(value)?;

        let slot = self
            .registers
            .get_mut(index)
            .expect("register was validated above");

        slot.capture(upvalue).expect("register remains uncaptured");

        Ok(upvalue)
    }

    pub(crate) fn instantiate_child(
        &mut self,
        runtime: &mut Runtime,
        child: PrototypeIndex,
    ) -> FaultResult<FunctionId> {
        let child_number = child.get();

        let (child_prototype, descriptors) = {
            let parent = self.runtime_prototype();

            let child_prototype =
                parent
                    .child(child)
                    .ok_or(VmErrorKind::InvalidChildPrototype {
                        child: child_number,
                    })?;

            let descriptors = self
                .bundle
                .prototype(child_prototype)
                .expect("child index belongs to the same bundle")
                .capture_descriptors()
                .to_vec();

            (child_prototype, descriptors)
        };

        let mut upvalues = Vec::new();
        upvalues.try_reserve(descriptors.len()).map_err(|_| {
            VmErrorKind::FrameCapacityExceeded {
                requested: descriptors.len(),
            }
        })?;

        for (upvalue_index, descriptor) in descriptors.into_iter().enumerate() {
            let upvalue = match descriptor {
                CaptureDescriptor::ParentRegister(register) => {
                    self.capture_register(runtime, register)?
                }
                CaptureDescriptor::ParentUpvalue(parent_upvalue) => {
                    self.upvalue_id(parent_upvalue)?
                }
                CaptureDescriptor::ExternalEnvironment => {
                    return Err(VmErrorKind::InvalidChildExternalEnvironment {
                        child: child_number,
                        upvalue: upvalue_index,
                    });
                }
            };

            upvalues.push(upvalue);
        }

        runtime.allocate_lua_function(
            Rc::clone(&self.bundle),
            child_prototype,
            upvalues.into_boxed_slice(),
        )
    }

    pub(crate) fn close_upvalues_from(
        &mut self,
        runtime: &Runtime,
        base: Register,
    ) -> FaultResult<()> {
        let start = usize::from(base.0);

        if start > self.registers.len() {
            return Err(VmErrorKind::InvalidRegister { register: base.0 });
        }

        for index in start..self.registers.len() {
            let Some(upvalue) = self.registers[index].captured_id() else {
                continue;
            };

            let value = runtime.read_upvalue(upvalue)?;

            self.registers[index] = RegisterSlot::direct(value);
        }

        Ok(())
    }

    pub(crate) fn set_open_results(
        &mut self,
        runtime: &mut Runtime,
        base: usize,
        values: &[RawValue],
    ) -> FaultResult<()> {
        let top = base
            .checked_add(values.len())
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: base,
                count: values.len(),
            })?;

        self.ensure_register_capacity(top)?;

        self.set_register_range(runtime, base, values.len(), values)?;

        self.registers.truncate(self.declared_registers.max(top));

        self.open_results = Some(OpenExtent { base, top });

        Ok(())
    }

    pub(crate) fn reset_open_results(&mut self) {
        self.open_results = None;
        self.registers.truncate(self.declared_registers);
    }

    pub(crate) fn take_open_results(
        &mut self,
        runtime: &Runtime,
        start: usize,
    ) -> FaultResult<Box<[RawValue]>> {
        let extent = self
            .open_results
            .take()
            .ok_or(VmErrorKind::MissingOpenResultExtent)?;

        if start > extent.base {
            self.registers.truncate(self.declared_registers);

            return Err(VmErrorKind::InvalidOpenResultStart {
                requested_start: start,
                result_base: extent.base,
            });
        }

        let values = self.get_register_range(runtime, start, extent.top - start)?;

        self.registers.truncate(self.declared_registers);

        Ok(values)
    }

    pub(crate) fn write_varargs(
        &mut self,
        runtime: &mut Runtime,
        base: Register,
        results: Count,
    ) -> FaultResult<()> {
        if !self.runtime_prototype().is_vararg() {
            return Err(VmErrorKind::InvalidVarargAccess);
        }

        let values = self.varargs.clone();
        let base = usize::from(base.0);

        match results {
            Count::Fixed(count) => {
                self.reset_open_results();

                self.set_register_range(runtime, base, usize::from(count), &values)
            }
            Count::Open => self.set_open_results(runtime, base, &values),
        }
    }

    pub(crate) fn collect_call(
        &mut self,
        runtime: &Runtime,
        base: Register,
        arguments: Count,
    ) -> FaultResult<(RawValue, Box<[RawValue]>)> {
        let callee = self.get_register(runtime, base)?;

        let base = usize::from(base.0);

        let argument_start = base
            .checked_add(1)
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: base,
                count: 1,
            })?;

        let arguments = match arguments {
            Count::Fixed(count) => {
                self.get_register_range(runtime, argument_start, usize::from(count))?
            }
            Count::Open => self.take_open_results(runtime, argument_start)?,
        };

        Ok((callee, arguments))
    }

    pub(crate) fn collect_return(
        &mut self,
        runtime: &Runtime,
        base: Register,
        values: Count,
    ) -> FaultResult<Box<[RawValue]>> {
        let base = usize::from(base.0);

        match values {
            Count::Fixed(count) => self.get_register_range(runtime, base, usize::from(count)),
            Count::Open => self.take_open_results(runtime, base),
        }
    }

    pub(crate) fn collect_list_values(
        &mut self,
        runtime: &Runtime,
        source: Register,
        count: Count,
    ) -> FaultResult<Box<[RawValue]>> {
        let start = usize::from(source.0);

        match count {
            Count::Fixed(count) => self.get_register_range(runtime, start, usize::from(count)),
            Count::Open => self.take_open_results(runtime, start),
        }
    }

    pub(crate) fn accept_results(
        &mut self,
        runtime: &mut Runtime,
        target: ResultTarget,
        values: &[RawValue],
    ) -> FaultResult<()> {
        match target {
            ResultTarget::Call { base, results } => match results {
                Count::Fixed(count) => {
                    self.reset_open_results();
                    self.set_register_range(runtime, base, usize::from(count), values)
                }
                Count::Open => self.set_open_results(runtime, base, values),
            },
            ResultTarget::GenericFor { start, variables } => {
                self.reset_open_results();
                self.set_register_range(runtime, start, variables, values)
            }
            ResultTarget::Index { destination } | ResultTarget::Operator { destination } => {
                let value = values.first().cloned().unwrap_or(RawValue::Nil);
                self.set_register(runtime, destination, value)
            }
            ResultTarget::Comparison { destination } => {
                let result = values.first().is_some_and(RawValue::is_truthy);
                self.set_register(runtime, destination, RawValue::Boolean(result))
            }
            ResultTarget::NewIndex => Ok(()),
        }
    }

    pub(crate) fn apply_jump(&mut self, offset: i32) -> FaultResult<()> {
        let target = self
            .pc
            .checked_add_signed(offset as isize)
            .filter(|target| *target < self.runtime_prototype().code_len())
            .ok_or(VmErrorKind::InvalidJump { offset })?;

        self.pc = target;

        Ok(())
    }

    pub(crate) fn trace_frame(&self) -> VmTraceFrame {
        let pc = self.current_pc.unwrap_or(self.pc);
        let prototype = self.runtime_prototype();

        VmTraceFrame::Lua {
            function_span: prototype.function_span(),
            pc,
            instruction_span: prototype.instruction_span(pc),
        }
    }

    pub(crate) fn visit_roots(&self, mut visit: impl FnMut(ObjectId)) {
        for upvalue in &self.upvalues {
            visit(upvalue.object());
        }

        for register in &self.registers {
            if let Some(upvalue) = register.captured_id() {
                visit(upvalue.object());
            } else if let Some(value) = register.direct_value()
                && let Some(object) = value.object_id()
            {
                visit(object);
            }
        }

        for value in &self.varargs {
            if let Some(object) = value.object_id() {
                visit(object);
            }
        }
    }

    fn ensure_register_capacity(&mut self, required: usize) -> FaultResult<()> {
        if required <= self.registers.len() {
            return Ok(());
        }

        self.registers
            .try_reserve(required - self.registers.len())
            .map_err(|_| VmErrorKind::FrameCapacityExceeded {
                requested: required,
            })?;

        self.registers
            .resize_with(required, || RegisterSlot::direct(RawValue::Nil));

        Ok(())
    }

    fn upvalue_id(&self, upvalue: UpvalueIndex) -> FaultResult<UpvalueId> {
        let raw_index = upvalue.get();

        self.upvalues
            .get(raw_index as usize)
            .copied()
            .ok_or(VmErrorKind::InvalidUpvalue { upvalue: raw_index })
    }

    fn runtime_prototype(&self) -> &RuntimePrototype {
        self.bundle
            .prototype(self.prototype)
            .expect("frame prototype belongs to its bundle")
    }
}

pub(crate) fn offset_register(base: Register, offset: u8) -> FaultResult<Register> {
    let register = base
        .0
        .checked_add(offset)
        .ok_or(VmErrorKind::InvalidRegisterOffset {
            base: base.0,
            offset,
        })?;

    Ok(Register(register))
}

fn read_slot(runtime: &Runtime, slot: &RegisterSlot) -> FaultResult<RawValue> {
    match slot.captured_id() {
        Some(upvalue) => runtime.read_upvalue(upvalue),
        None => Ok(slot
            .direct_value()
            .expect("uncaptured register has a direct value")
            .clone()),
    }
}

fn write_slot(runtime: &mut Runtime, slot: &mut RegisterSlot, value: RawValue) -> FaultResult<()> {
    match slot.captured_id() {
        Some(upvalue) => {
            runtime.write_upvalue(upvalue, value)?;
        }
        None => {
            *slot
                .direct_value_mut()
                .expect("uncaptured register has a direct value") = value;
        }
    }

    Ok(())
}

fn invalid_prototype(prototype: RuntimePrototypeIndex) -> VmErrorKind {
    VmErrorKind::InvalidChildPrototype {
        child: u32::try_from(prototype.get()).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::SourceId;
    use orbit_compiler::bytecode::Chunk;
    use orbit_parser::{lexer::lex, parser::parse_chunk};

    use crate::{
        function::FunctionSnapshot, loading::NoLoadService, runtime::Runtime, string::LuaString,
        value::RawValue,
    };

    use super::CallFrame;

    fn compile_source(source: &str) -> Chunk {
        let source_id = SourceId::new(0);
        let tokens = lex(source_id, source).unwrap();
        let ast = parse_chunk(source_id, &tokens).unwrap();
        let hir = orbit_resolver::resolve(&ast).unwrap();

        orbit_compiler::compile(hir).unwrap()
    }

    fn frame(source: &str) -> (Runtime, CallFrame) {
        let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();
        let function = runtime.load_raw(compile_source(source)).unwrap();

        let invocation = match runtime.function_snapshot(function).unwrap() {
            FunctionSnapshot::Lua(invocation) => invocation,
            FunctionSnapshot::Native(_) => {
                panic!("compiled chunks produce Lua functions")
            }
        };

        let frame = CallFrame::new(invocation, &[]).unwrap();

        (runtime, frame)
    }

    #[test]
    fn consuming_open_results_truncates_the_dynamic_register_tail() {
        let (mut runtime, mut frame) = frame("return");
        let declared = frame.declared_registers;
        let values = vec![RawValue::String(LuaString::from("payload")); 32];

        frame
            .set_open_results(&mut runtime, declared, &values)
            .unwrap();

        assert_eq!(frame.registers.len(), declared + values.len());

        let consumed = frame.take_open_results(&runtime, declared).unwrap();

        assert_eq!(consumed.as_ref(), values.as_slice());
        assert_eq!(frame.registers.len(), declared);
    }

    #[test]
    fn replacing_open_results_truncates_the_previous_dynamic_tail() {
        let (mut runtime, mut frame) = frame("return");
        let declared = frame.declared_registers;
        let previous = vec![RawValue::String(LuaString::from("payload")); 32];

        frame
            .set_open_results(&mut runtime, declared, &previous)
            .unwrap();

        frame
            .set_open_results(&mut runtime, declared, &[RawValue::Integer(1)])
            .unwrap();

        assert_eq!(frame.registers.len(), declared + 1);

        let consumed = frame.take_open_results(&runtime, declared).unwrap();

        assert_eq!(consumed.as_ref(), &[RawValue::Integer(1)]);
        assert_eq!(frame.registers.len(), declared);
    }
}
