use std::rc::Rc;

use orbit_compiler::bytecode::{
    ConstantIndex, Count, Instruction, PrototypeIndex, Register, UpvalueIndex,
};

use crate::{
    error::{FaultResult, LuaTraceFunction, VmErrorKind, VmTraceFrame},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingDefinitions {
    start: usize,
    end: usize,
}

impl PendingDefinitions {
    fn for_target(target: ResultTarget, declared_registers: usize) -> FaultResult<Option<Self>> {
        let (start, count) = match target {
            ResultTarget::Call {
                base,
                results: Count::Fixed(count),
            } => (base, usize::from(count)),

            ResultTarget::Call {
                results: Count::Open,
                ..
            } => return Ok(None),

            ResultTarget::GenericFor { start, variables } => (start, variables),

            ResultTarget::Index { destination }
            | ResultTarget::Operator { destination }
            | ResultTarget::Comparison { destination } => (usize::from(destination.0), 1),

            ResultTarget::NewIndex | ResultTarget::Close => return Ok(None),
        };

        if count == 0 {
            return Ok(None);
        }

        let end = start
            .checked_add(count)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        if end > declared_registers {
            return Err(VmErrorKind::InvalidRegisterRange { start, count });
        }

        Ok(Some(Self { start, end }))
    }

    fn contains(self, index: usize) -> bool {
        self.start <= index && index < self.end
    }
}

pub(crate) struct CallFrame {
    function: FunctionId,
    bundle: Rc<PrototypeBundle>,
    prototype: RuntimePrototypeIndex,
    runtime_prototype: Rc<RuntimePrototype>,
    upvalues: Rc<[UpvalueId]>,
    varargs: Vec<RawValue>,
    registers: Vec<RegisterSlot>,
    close: Vec<Register>,
    declared_registers: usize,
    open_results: Option<OpenExtent>,
    pc: usize,
    current_pc: Option<usize>,
    pending_definitions: Option<PendingDefinitions>,
    gc_pc: usize,
}

/// Reusable heap-backed portions of a completed Lua frame.
#[derive(Default)]
pub(crate) struct CallFrameStorage {
    varargs: Vec<RawValue>,
    registers: Vec<RegisterSlot>,
    close: Vec<Register>,
}

impl CallFrame {
    pub(crate) fn new(invocation: LuaInvocation, arguments: &[RawValue]) -> FaultResult<Self> {
        Self::new_reusing(invocation, arguments, CallFrameStorage::default())
    }

    pub(crate) fn new_reusing(
        invocation: LuaInvocation,
        arguments: &[RawValue],
        storage: CallFrameStorage,
    ) -> FaultResult<Self> {
        Self::new_with_arguments(
            invocation,
            arguments.len(),
            |index| Ok(arguments[index].clone()),
            storage,
        )
    }

    pub(crate) fn new_from_frame(
        invocation: LuaInvocation,
        runtime: &Runtime,
        source: &Self,
        argument_start: usize,
        argument_count: usize,
        storage: CallFrameStorage,
    ) -> FaultResult<Self> {
        Self::new_with_arguments(
            invocation,
            argument_count,
            |index| source.get_register_index(runtime, argument_start + index),
            storage,
        )
    }

    fn new_with_arguments(
        invocation: LuaInvocation,
        argument_count: usize,
        mut argument: impl FnMut(usize) -> FaultResult<RawValue>,
        storage: CallFrameStorage,
    ) -> FaultResult<Self> {
        let (function, bundle, prototype, upvalues) = invocation.into_parts();

        let runtime_prototype = bundle
            .prototype_handle(prototype)
            .ok_or_else(|| invalid_prototype(prototype))?;
        let (parameter_count, is_vararg, declared_registers, expected_upvalues) = {
            let prototype = &runtime_prototype;

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

        let CallFrameStorage {
            mut varargs,
            mut registers,
            mut close,
        } = storage;

        registers.clear();
        registers.try_reserve(declared_registers).map_err(|_| {
            VmErrorKind::FrameCapacityExceeded {
                requested: declared_registers,
            }
        })?;

        for index in 0..declared_registers {
            let value = if index < parameter_count && index < argument_count {
                argument(index)?
            } else {
                RawValue::Nil
            };

            registers.push(RegisterSlot::direct(value));
        }

        let extra_argument_start = parameter_count.min(argument_count);
        let extra_argument_count = if is_vararg {
            argument_count - extra_argument_start
        } else {
            0
        };

        varargs.clear();
        varargs.try_reserve(extra_argument_count).map_err(|_| {
            VmErrorKind::FrameCapacityExceeded {
                requested: extra_argument_count,
            }
        })?;

        for index in extra_argument_start..argument_count {
            if is_vararg {
                varargs.push(argument(index)?);
            }
        }

        close.clear();

        Ok(Self {
            function,
            bundle,
            prototype,
            runtime_prototype,
            upvalues,
            varargs,
            registers,
            close,
            declared_registers,
            open_results: None,
            pc: 0,
            current_pc: None,
            pending_definitions: None,
            gc_pc: 0,
        })
    }

    pub(crate) fn into_storage(mut self) -> CallFrameStorage {
        self.varargs.clear();
        self.registers.clear();
        self.close.clear();

        CallFrameStorage {
            varargs: self.varargs,
            registers: self.registers,
            close: self.close,
        }
    }

    pub(crate) fn replace(
        &mut self,
        invocation: LuaInvocation,
        arguments: &[RawValue],
    ) -> FaultResult<()> {
        let (function, bundle, prototype, upvalues) = invocation.into_parts();

        let runtime_prototype = bundle
            .prototype_handle(prototype)
            .ok_or_else(|| invalid_prototype(prototype))?;
        let (parameter_count, is_vararg, declared_registers, expected_upvalues) = {
            let prototype = &runtime_prototype;

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

        self.reset_values(parameter_count, is_vararg, declared_registers, arguments)?;

        self.function = function;
        self.bundle = bundle;
        self.prototype = prototype;
        self.runtime_prototype = runtime_prototype;
        self.upvalues = upvalues;

        Ok(())
    }

    pub(crate) fn function(&self) -> FunctionId {
        self.function
    }

    pub(crate) fn restart(&mut self, arguments: &[RawValue]) -> FaultResult<()> {
        let (parameter_count, is_vararg, declared_registers) = {
            let prototype = self.runtime_prototype();
            (
                usize::from(prototype.parameter_count()),
                prototype.is_vararg(),
                usize::from(prototype.max_registers()),
            )
        };

        self.reset_values(parameter_count, is_vararg, declared_registers, arguments)
    }

    fn reset_values(
        &mut self,
        parameter_count: usize,
        is_vararg: bool,
        declared_registers: usize,
        arguments: &[RawValue],
    ) -> FaultResult<()> {
        self.registers
            .try_reserve(declared_registers.saturating_sub(self.registers.len()))
            .map_err(|_| VmErrorKind::FrameCapacityExceeded {
                requested: declared_registers,
            })?;

        let extra_arguments = if is_vararg {
            arguments.get(parameter_count..).unwrap_or(&[])
        } else {
            &[]
        };

        self.varargs
            .try_reserve(extra_arguments.len().saturating_sub(self.varargs.len()))
            .map_err(|_| VmErrorKind::FrameCapacityExceeded {
                requested: extra_arguments.len(),
            })?;

        self.declared_registers = declared_registers;
        self.open_results = None;
        self.pc = 0;
        self.current_pc = None;

        self.registers.clear();
        for index in 0..declared_registers {
            let value = if index < parameter_count {
                arguments.get(index).cloned().unwrap_or(RawValue::Nil)
            } else {
                RawValue::Nil
            };

            self.registers.push(RegisterSlot::direct(value));
        }

        self.varargs.clear();
        self.varargs.extend_from_slice(extra_arguments);

        self.pending_definitions = None;
        self.gc_pc = 0;

        Ok(())
    }

    pub(crate) fn next_instruction(&mut self) -> FaultResult<Instruction> {
        debug_assert_eq!(self.gc_pc, self.pc);
        debug_assert!(self.pending_definitions.is_none());

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

    fn get_register_index(&self, runtime: &Runtime, index: usize) -> FaultResult<RawValue> {
        let slot = self
            .registers
            .get(index)
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: index,
                count: 1,
            })?;

        read_slot(runtime, slot)
    }

    fn set_register_index(
        &mut self,
        runtime: &mut Runtime,
        index: usize,
        value: RawValue,
    ) -> FaultResult<()> {
        let slot = self
            .registers
            .get_mut(index)
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: index,
                count: 1,
            })?;

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

    pub(crate) fn call_register_range(
        &self,
        runtime: &Runtime,
        base: Register,
        arguments: Count,
    ) -> FaultResult<(RawValue, usize, usize)> {
        let callee = self.get_register(runtime, base)?;
        let base = usize::from(base.0);
        let start = base
            .checked_add(1)
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: base,
                count: 1,
            })?;
        let count = match arguments {
            Count::Fixed(count) => usize::from(count),
            Count::Open => {
                let extent = self
                    .open_results
                    .ok_or(VmErrorKind::MissingOpenResultExtent)?;

                if start > extent.base {
                    return Err(VmErrorKind::InvalidOpenResultStart {
                        requested_start: start,
                        result_base: extent.base,
                    });
                }

                extent.top - start
            }
        };
        let end = start
            .checked_add(count)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        self.registers
            .get(start..end)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        Ok((callee, start, count))
    }

    pub(crate) fn consume_open_call_arguments(&mut self, arguments: Count) {
        if matches!(arguments, Count::Open) {
            self.reset_open_results();
        }
    }

    pub(crate) fn collect_call_into(
        &mut self,
        runtime: &Runtime,
        base: Register,
        arguments: Count,
        destination: &mut Vec<RawValue>,
    ) -> FaultResult<RawValue> {
        let callee = self.get_register(runtime, base)?;
        let base = usize::from(base.0);
        let argument_start = base
            .checked_add(1)
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: base,
                count: 1,
            })?;

        destination.clear();

        match arguments {
            Count::Fixed(count) => {
                let count = usize::from(count);
                let end =
                    argument_start
                        .checked_add(count)
                        .ok_or(VmErrorKind::InvalidRegisterRange {
                            start: argument_start,
                            count,
                        })?;
                let slots = self.registers.get(argument_start..end).ok_or(
                    VmErrorKind::InvalidRegisterRange {
                        start: argument_start,
                        count,
                    },
                )?;

                destination
                    .try_reserve(count)
                    .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested: count })?;

                for slot in slots {
                    destination.push(read_slot(runtime, slot)?);
                }
            }
            Count::Open => {
                let values = self.take_open_results(runtime, argument_start)?;
                destination.try_reserve(values.len()).map_err(|_| {
                    VmErrorKind::FrameCapacityExceeded {
                        requested: values.len(),
                    }
                })?;
                destination.extend(values);
            }
        }

        Ok(callee)
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

    fn return_register_range(&self, base: Register, values: Count) -> FaultResult<(usize, usize)> {
        let start = usize::from(base.0);
        let count = match values {
            Count::Fixed(count) => usize::from(count),
            Count::Open => {
                let extent = self
                    .open_results
                    .ok_or(VmErrorKind::MissingOpenResultExtent)?;

                if start > extent.base {
                    return Err(VmErrorKind::InvalidOpenResultStart {
                        requested_start: start,
                        result_base: extent.base,
                    });
                }

                extent.top - start
            }
        };
        let end = start
            .checked_add(count)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        self.registers
            .get(start..end)
            .ok_or(VmErrorKind::InvalidRegisterRange { start, count })?;

        Ok((start, count))
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
        let result = match target {
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
            ResultTarget::NewIndex | ResultTarget::Close => Ok(()),
        };

        if result.is_ok() {
            self.complete_pending_results(target)?;
        }

        result
    }

    pub(crate) fn accept_results_from_frame(
        &mut self,
        runtime: &mut Runtime,
        target: ResultTarget,
        source: &Self,
        source_base: Register,
        source_values: Count,
    ) -> FaultResult<()> {
        let (source_start, source_count) =
            source.return_register_range(source_base, source_values)?;

        let result = match target {
            ResultTarget::Call { base, results } => match results {
                Count::Fixed(count) => {
                    self.reset_open_results();
                    self.copy_results_from_frame(
                        runtime,
                        base,
                        usize::from(count),
                        source,
                        source_start,
                        source_count,
                    )
                }
                Count::Open => self.set_open_results_from_frame(
                    runtime,
                    base,
                    source,
                    source_start,
                    source_count,
                ),
            },
            ResultTarget::GenericFor { start, variables } => {
                self.reset_open_results();
                self.copy_results_from_frame(
                    runtime,
                    start,
                    variables,
                    source,
                    source_start,
                    source_count,
                )
            }
            ResultTarget::Index { destination } | ResultTarget::Operator { destination } => {
                let value = if source_count == 0 {
                    RawValue::Nil
                } else {
                    source.get_register_index(runtime, source_start)?
                };
                self.set_register(runtime, destination, value)
            }
            ResultTarget::Comparison { destination } => {
                let result = source_count != 0
                    && source
                        .get_register_index(runtime, source_start)?
                        .is_truthy();
                self.set_register(runtime, destination, RawValue::Boolean(result))
            }
            ResultTarget::NewIndex | ResultTarget::Close => Ok(()),
        };

        if result.is_ok() {
            self.complete_pending_results(target)?;
        }

        result
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
        let function = if self.prototype == RuntimePrototypeIndex::ENTRY {
            LuaTraceFunction::MainChunk
        } else if let Some(name) = prototype.name() {
            LuaTraceFunction::Named(name.into())
        } else {
            LuaTraceFunction::Anonymous
        };

        VmTraceFrame::Lua {
            function,
            function_span: prototype.function_span(),
            pc,
            instruction_span: prototype.instruction_span(pc),
        }
    }

    pub(crate) fn visit_roots(&self, mut visit: impl FnMut(ObjectId)) {
        visit(self.function.object());

        for upvalue in self.upvalues.iter() {
            visit(upvalue.object());
        }

        let roots = self
            .runtime_prototype
            .register_root_map(self.gc_pc)
            .expect("validated register-root map for GC PC");

        let declared = self
            .registers
            .get(..self.declared_registers)
            .expect("frame contains every declared register");

        for (index, slot) in declared.iter().enumerate() {
            if let Some(upvalue) = slot.captured_id() {
                visit(upvalue.object());
                continue;
            }

            let register =
                Register(u8::try_from(index).expect("validated frame has at most 256 registers"));

            if !roots.contains(register) {
                continue;
            }

            if self
                .pending_definitions
                .is_some_and(|pending| pending.contains(index))
            {
                continue;
            }

            if let Some(value) = slot.direct_value()
                && let Some(object) = value.object_id()
            {
                visit(object);
            }
        }

        if let Some(extent) = self.open_results {
            for index in extent.base..extent.top {
                if let Some(slot) = self.registers.get(index) {
                    visit_register_slot(slot, &mut visit);
                }
            }
        }

        for register in &self.close {
            if let Some(slot) = self.registers.get(usize::from(register.0)) {
                visit_register_slot(slot, &mut visit);
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

    fn copy_results_from_frame(
        &mut self,
        runtime: &mut Runtime,
        destination_start: usize,
        destination_count: usize,
        source: &Self,
        source_start: usize,
        source_count: usize,
    ) -> FaultResult<()> {
        let destination_end = destination_start.checked_add(destination_count).ok_or(
            VmErrorKind::InvalidRegisterRange {
                start: destination_start,
                count: destination_count,
            },
        )?;

        self.registers
            .get(destination_start..destination_end)
            .ok_or(VmErrorKind::InvalidRegisterRange {
                start: destination_start,
                count: destination_count,
            })?;

        for index in 0..destination_count {
            let value = if index < source_count {
                source.get_register_index(runtime, source_start + index)?
            } else {
                RawValue::Nil
            };

            self.set_register_index(runtime, destination_start + index, value)?;
        }

        Ok(())
    }

    fn set_open_results_from_frame(
        &mut self,
        runtime: &mut Runtime,
        destination_start: usize,
        source: &Self,
        source_start: usize,
        source_count: usize,
    ) -> FaultResult<()> {
        let top = destination_start.checked_add(source_count).ok_or(
            VmErrorKind::InvalidRegisterRange {
                start: destination_start,
                count: source_count,
            },
        )?;

        self.ensure_register_capacity(top)?;

        for index in 0..source_count {
            let value = source.get_register_index(runtime, source_start + index)?;
            self.set_register_index(runtime, destination_start + index, value)?;
        }

        self.registers.truncate(self.declared_registers.max(top));
        self.open_results = Some(OpenExtent {
            base: destination_start,
            top,
        });

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
        &self.runtime_prototype
    }

    pub(crate) fn close_name(&self) -> Option<&str> {
        self.runtime_prototype.close_name(self.current_pc?)
    }

    pub(crate) fn mark_to_close(&mut self, register: Register) -> FaultResult<()> {
        let requested = self.close.len().saturating_add(1);

        self.close
            .try_reserve(1)
            .map_err(|_| VmErrorKind::FrameCapacityExceeded { requested })?;

        if let Some(previous) = self.close.last()
            && previous.0 >= register.0
        {
            return Err(VmErrorKind::InvalidToCloseOrder {
                previous: previous.0,
                register: register.0,
            });
        }

        self.close.push(register);

        Ok(())
    }

    pub(crate) fn pop_to_close_from(&mut self, base: Register) -> Option<Register> {
        if self.close.last().is_some_and(|register| *register >= base) {
            self.close.pop()
        } else {
            None
        }
    }

    pub(crate) fn begin_pending_results(&mut self, target: ResultTarget) -> FaultResult<()> {
        let pending = PendingDefinitions::for_target(target, self.declared_registers)?;

        if let Some(pending) = pending {
            debug_assert!(self.pending_definitions.is_none());
            self.pending_definitions = Some(pending);
        }

        Ok(())
    }

    fn complete_pending_results(&mut self, target: ResultTarget) -> FaultResult<()> {
        let expected = PendingDefinitions::for_target(target, self.declared_registers)?;

        if let Some(expected) = expected {
            debug_assert_eq!(self.pending_definitions, Some(expected));
            self.pending_definitions = None;
        }

        Ok(())
    }

    pub(crate) fn commit_instruction(&mut self) {
        self.gc_pc = self.pc;
    }

    #[cfg(test)]
    pub(crate) fn gc_pc_for_test(&self) -> usize {
        self.gc_pc
    }
}

fn visit_register_slot(slot: &RegisterSlot, visit: &mut impl FnMut(ObjectId)) {
    if let Some(upvalue) = slot.captured_id() {
        visit(upvalue.object());
    } else if let Some(value) = slot.direct_value()
        && let Some(object) = value.object_id()
    {
        visit(object);
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
    use orbit_compiler::bytecode::{Chunk, Instruction, Register};
    use orbit_parser::{lexer::lex, parser::parse_chunk};

    use crate::{
        function::FunctionSnapshot, loading::NoLoadService, runtime::Runtime, string::LuaString,
        value::RawValue,
    };

    use super::{CallFrame, ResultTarget};

    fn compile_source(source: &str) -> Chunk {
        let source_id = SourceId::new(0);
        let tokens = lex(source_id, source).unwrap();
        let ast = parse_chunk(source_id, tokens).unwrap();
        let hir = orbit_resolver::resolve(&ast).unwrap();

        orbit_compiler::compile(hir).unwrap()
    }

    fn frame(source: &str) -> (Runtime, CallFrame) {
        let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();
        let function = runtime.load_chunk_raw(compile_source(source)).unwrap();

        let invocation = match runtime.function_snapshot(function).unwrap() {
            FunctionSnapshot::Lua(invocation) => invocation,
            FunctionSnapshot::Native(_) => {
                panic!("compiled chunks produce Lua functions")
            }
        };

        let frame = CallFrame::new(invocation, &[]).unwrap();

        (runtime, frame)
    }

    fn return_pc(frame: &CallFrame) -> usize {
        (0..frame.runtime_prototype.code_len())
            .find(|&pc| {
                matches!(
                    frame.runtime_prototype.instruction(pc),
                    Some(Instruction::Return { .. })
                )
            })
            .expect("test function has a return instruction")
    }

    fn roots(frame: &CallFrame) -> Vec<crate::id::ObjectId> {
        let mut roots = Vec::new();
        frame.visit_roots(|root| roots.push(root));
        roots
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

    #[test]
    fn register_root_map_omits_released_direct_registers() {
        let (mut runtime, mut frame) = frame(
            r#"
                local live = {}
                do local dead = {} end
                return live
            "#,
        );
        let live = runtime.allocate_table(0, 0).unwrap();
        let dead = runtime.allocate_table(0, 0).unwrap();

        frame
            .set_register(&mut runtime, Register(0), RawValue::Table(live))
            .unwrap();
        frame
            .set_register(&mut runtime, Register(1), RawValue::Table(dead))
            .unwrap();
        frame.gc_pc = return_pc(&frame) - 1;

        let roots = roots(&frame);

        assert!(roots.contains(&frame.function.object()));
        assert!(roots.contains(&live.object()));
        assert!(!roots.contains(&dead.object()));
    }

    #[test]
    fn pending_direct_result_is_suppressed_until_acceptance() {
        let (mut runtime, mut frame) = frame("local value = {}; return value");
        let stale = runtime.allocate_table(0, 0).unwrap();
        let result = runtime.allocate_table(0, 0).unwrap();
        let target = ResultTarget::Index {
            destination: Register(0),
        };

        frame
            .set_register(&mut runtime, Register(0), RawValue::Table(stale))
            .unwrap();
        frame.gc_pc = return_pc(&frame);
        frame.begin_pending_results(target).unwrap();

        assert!(!roots(&frame).contains(&stale.object()));

        frame
            .accept_results(&mut runtime, target, &[RawValue::Table(result)])
            .unwrap();

        let roots = roots(&frame);
        assert!(!roots.contains(&stale.object()));
        assert!(roots.contains(&result.object()));
    }

    #[test]
    fn captured_pending_result_remains_an_upvalue_root() {
        let (mut runtime, mut frame) = frame("local value = {}; return value");
        let value = runtime.allocate_table(0, 0).unwrap();

        frame
            .set_register(&mut runtime, Register(0), RawValue::Table(value))
            .unwrap();
        let upvalue = frame.capture_register(&mut runtime, Register(0)).unwrap();
        frame.gc_pc = return_pc(&frame);
        frame
            .begin_pending_results(ResultTarget::Index {
                destination: Register(0),
            })
            .unwrap();

        assert!(roots(&frame).contains(&upvalue.object()));
    }

    #[test]
    fn close_register_overrides_an_empty_static_root_map() {
        let (mut runtime, mut frame) = frame("local value = {}; return value");
        let value = runtime.allocate_table(0, 0).unwrap();

        frame
            .set_register(&mut runtime, Register(0), RawValue::Table(value))
            .unwrap();
        frame.mark_to_close(Register(0)).unwrap();
        frame.gc_pc = 0;

        assert!(roots(&frame).contains(&value.object()));
    }

    #[test]
    fn open_results_override_an_empty_terminal_root_map() {
        let (mut runtime, mut frame) = frame("return");
        let value = runtime.allocate_table(0, 0).unwrap();
        let base = frame.declared_registers;

        frame
            .set_open_results(&mut runtime, base, &[RawValue::Table(value)])
            .unwrap();
        frame.gc_pc = frame.runtime_prototype.code_len();

        assert!(roots(&frame).contains(&value.object()));
    }

    #[test]
    fn varargs_and_active_function_are_always_roots() {
        let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();
        let function = runtime
            .load_chunk_raw(compile_source("return ..."))
            .unwrap();
        let argument = runtime.allocate_table(0, 0).unwrap();
        let invocation = match runtime.function_snapshot(function).unwrap() {
            FunctionSnapshot::Lua(invocation) => invocation,
            FunctionSnapshot::Native(_) => {
                panic!("compiled chunks produce Lua functions")
            }
        };
        let frame = CallFrame::new(invocation, &[RawValue::Table(argument)]).unwrap();
        let roots = roots(&frame);

        assert!(roots.contains(&function.object()));
        assert!(roots.contains(&argument.object()));
    }

    #[test]
    fn result_targets_define_the_expected_pending_ranges() {
        use orbit_compiler::bytecode::Count;

        assert_eq!(
            super::PendingDefinitions::for_target(
                ResultTarget::Call {
                    base: 2,
                    results: Count::Fixed(3),
                },
                8,
            )
            .unwrap(),
            Some(super::PendingDefinitions { start: 2, end: 5 })
        );
        assert_eq!(
            super::PendingDefinitions::for_target(
                ResultTarget::GenericFor {
                    start: 4,
                    variables: 2,
                },
                8,
            )
            .unwrap(),
            Some(super::PendingDefinitions { start: 4, end: 6 })
        );

        for target in [
            ResultTarget::Index {
                destination: Register(3),
            },
            ResultTarget::Operator {
                destination: Register(3),
            },
            ResultTarget::Comparison {
                destination: Register(3),
            },
        ] {
            assert_eq!(
                super::PendingDefinitions::for_target(target, 8).unwrap(),
                Some(super::PendingDefinitions { start: 3, end: 4 })
            );
        }

        for target in [
            ResultTarget::Call {
                base: 2,
                results: Count::Open,
            },
            ResultTarget::Call {
                base: 2,
                results: Count::Fixed(0),
            },
            ResultTarget::NewIndex,
            ResultTarget::Close,
        ] {
            assert_eq!(
                super::PendingDefinitions::for_target(target, 8).unwrap(),
                None
            );
        }
    }
}
