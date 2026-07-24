mod sub;

use orbit_vm::{State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let string = state.create_table(0, 2)?;

    let sub = state.create_native_function("string.sub", sub::callback, &[])?;
    set_field(state, &string, b"sub", &Value::Function(sub))?;

    state.set_global(b"string", &Value::Table(string))
}
