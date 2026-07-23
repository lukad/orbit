mod unpack;

use orbit_vm::{State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let table = state.create_table(0, 1)?;

    let unpack = state.create_native_function("table.unpack", unpack::callback, &[])?;
    set_field(state, &table, b"unpack", &Value::Function(unpack))?;

    state.set_global(b"table", &Value::Table(table))
}
