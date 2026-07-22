use orbit_compiler::bytecode::{ConstantIndex, Count, PrototypeIndex, Register, UpvalueIndex};

use crate::{error::FaultResult, value::RawValue};

use super::{Activation, Execution};

impl Execution<'_> {
    pub(super) fn load_nil(&mut self, destination: Register) -> FaultResult<()> {
        self.write_register(destination, RawValue::Nil)
    }

    pub(super) fn load_boolean(&mut self, destination: Register, value: bool) -> FaultResult<()> {
        self.write_register(destination, RawValue::Boolean(value))
    }

    pub(super) fn load_small_integer(
        &mut self,
        destination: Register,
        value: i16,
    ) -> FaultResult<()> {
        self.write_register(destination, RawValue::Integer(i64::from(value)))
    }

    pub(super) fn load_constant(
        &mut self,
        destination: Register,
        constant: ConstantIndex,
    ) -> FaultResult<()> {
        let value = self.active_lua_frame().constant(constant)?;

        self.write_register(destination, value)
    }

    pub(super) fn move_value(
        &mut self,
        destination: Register,
        source: Register,
    ) -> FaultResult<()> {
        let value = self.read_register(source)?;

        self.write_register(destination, value)
    }

    pub(super) fn get_upvalue(
        &mut self,
        destination: Register,
        upvalue: UpvalueIndex,
    ) -> FaultResult<()> {
        let value = self
            .active_lua_frame()
            .get_upvalue(&*self.runtime, upvalue)?;

        self.write_register(destination, value)
    }

    pub(super) fn set_upvalue(
        &mut self,
        upvalue: UpvalueIndex,
        source: Register,
    ) -> FaultResult<()> {
        let value = self.read_register(source)?;

        let runtime = &mut *self.runtime;

        self.stack
            .last()
            .and_then(Activation::as_lua)
            .expect("active activation is Lua")
            .frame()
            .set_upvalue(runtime, upvalue, value)
    }

    pub(super) fn vararg(&mut self, base: Register, results: Count) -> FaultResult<()> {
        let runtime = &mut *self.runtime;

        self.stack
            .last_mut()
            .and_then(Activation::as_lua_mut)
            .expect("active activation is Lua")
            .frame_mut()
            .write_varargs(runtime, base, results)
    }

    pub(super) fn create_closure(
        &mut self,
        destination: Register,
        child: PrototypeIndex,
    ) -> FaultResult<()> {
        let function = {
            let runtime = &mut *self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .instantiate_child(runtime, child)?
        };

        self.write_register(destination, RawValue::Function(function))
    }
}
