mod basic;
mod error;
mod package;
mod table;

use orbit_vm::{LuaString, State, Table, Value, VmResult};

pub fn install(state: &mut State) -> VmResult<()> {
    basic::install(state)?;
    package::install(state)?;
    table::install(state)
}

pub(crate) fn set_field(
    state: &mut State,
    table: &Table,
    name: &[u8],
    value: &Value,
) -> VmResult<()> {
    state.raw_set(table, &Value::String(LuaString::new(name)), value)
}
