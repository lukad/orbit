mod upvalueid;

use orbit_vm::{LuaString, State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let debug = state.create_table(0, 0)?;

    let upvalueid =
        state.create_native_function(upvalueid::FUNCTION_NAME, upvalueid::callback, &[])?;
    set_field(
        state,
        &debug,
        upvalueid::FUNCTION,
        &Value::Function(upvalueid),
    )?;

    let Value::Table(package) = state.get_global(b"package")? else {
        unreachable!("package is installed before the debug library");
    };

    let loaded_key = Value::String(LuaString::from("loaded"));
    let Value::Table(loaded) = state.raw_get(&package, &loaded_key)? else {
        unreachable!("package.loaded is installed as a table");
    };

    let debug_value = Value::Table(debug);
    set_field(state, &loaded, b"debug", &debug_value)?;
    state.set_global(b"debug", &debug_value)
}
