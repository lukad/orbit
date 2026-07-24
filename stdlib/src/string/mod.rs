mod pack;
mod packing;
mod packsize;
mod sub;
mod unpack;

use orbit_vm::{LuaString, State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let string = state.create_table(0, 4)?;

    let sub = state.create_native_function("string.sub", sub::callback, &[])?;
    set_field(state, &string, b"sub", &Value::Function(sub))?;

    let pack = state.create_native_function("string.pack", pack::callback, &[])?;
    set_field(state, &string, b"pack", &Value::Function(pack))?;

    let packsize = state.create_native_function("string.packsize", packsize::callback, &[])?;
    set_field(state, &string, b"packsize", &Value::Function(packsize))?;

    let unpack = state.create_native_function("string.unpack", unpack::callback, &[])?;
    set_field(state, &string, b"unpack", &Value::Function(unpack))?;

    let metatable = state.create_table(0, 1)?;
    set_field(state, &metatable, b"__index", &Value::Table(string.clone()))?;

    state.set_metatable(&Value::String(LuaString::new(b"")), Some(&metatable))?;

    state.set_global(b"string", &Value::Table(string))
}
