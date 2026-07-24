#![forbid(unsafe_code)]

mod argument;
mod basic;
mod error;
mod math;
mod package;
mod string;
mod table;

use orbit_vm::{LuaString, State, Table, Value, VmResult};

pub fn install(state: &mut State) -> VmResult<()> {
    basic::install(state)?;
    package::install(state)?;
    table::install(state)?;
    math::install(state)?;
    string::install(state)
}

pub(crate) fn set_field(
    state: &mut State,
    table: &Table,
    name: impl AsRef<[u8]>,
    value: &Value,
) -> VmResult<()> {
    state.raw_set(table, &Value::String(LuaString::new(name)), value)
}
