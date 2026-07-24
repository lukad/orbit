mod sub;

use orbit_vm::{LuaString, State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let string = state.create_table(0, 2)?;

    let sub = state.create_native_function("string.sub", sub::callback, &[])?;
    set_field(state, &string, b"sub", &Value::Function(sub))?;

    let metatable = state.create_table(0, 1)?;
    set_field(state, &metatable, b"__index", &Value::Table(string.clone()))?;

    state.set_metatable(&Value::String(LuaString::new(b"")), Some(&metatable))?;

    state.set_global(b"string", &Value::Table(string))
}
