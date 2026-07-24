use orbit_vm::{LuaString, State, Value, VmResult};

mod assert;
mod dofile;
mod getmetatable;
mod ipairs;
mod load;
mod next;
mod pairs;
mod pcall;
mod print;
mod rawget;
mod select;
mod setmetatable;
mod tostring;
mod typ;

pub fn install(state: &mut State) -> VmResult<()> {
    let globals = state.globals()?;
    state.set_global(b"_G", &Value::Table(globals))?;
    state.set_global(b"_VERSION", &Value::String(LuaString::from("Lua 5.4")))?;

    let tostring = state.create_native_function("tostring", tostring::callback, &[])?;
    state.set_global(b"tostring", &Value::Function(tostring.clone()))?;

    let print =
        state.create_native_function("print", print::callback, &[Value::Function(tostring)])?;
    state.set_global(b"print", &Value::Function(print))?;

    let getmetatable = state.create_native_function("getmetatable", getmetatable::callback, &[])?;
    state.set_global(b"getmetatable", &Value::Function(getmetatable))?;

    let setmetatable = state.create_native_function("setmetatable", setmetatable::callback, &[])?;
    state.set_global(b"setmetatable", &Value::Function(setmetatable))?;

    let typ = state.create_native_function("type", typ::callback, &[])?;
    state.set_global(b"type", &Value::Function(typ))?;

    let assert = state.create_native_function("assert", assert::callback, &[])?;
    state.set_global(b"assert", &Value::Function(assert))?;

    let next = state.create_native_function("next", next::callback, &[])?;
    state.set_global(b"next", &Value::Function(next.clone()))?;

    let pairs = state.create_native_function("pairs", pairs::callback, &[Value::Function(next)])?;
    state.set_global(b"pairs", &Value::Function(pairs))?;

    let load = state.create_native_function("load", load::callback, &[])?;
    state.set_global(b"load", &Value::Function(load))?;

    let ipairs_iterator = state.create_native_function("ipairs iterator", ipairs::iterator, &[])?;
    let ipairs = state.create_native_function(
        "ipairs",
        ipairs::callback,
        &[Value::Function(ipairs_iterator)],
    )?;
    state.set_global(b"ipairs", &Value::Function(ipairs))?;

    let rawget = state.create_native_function("rawget", rawget::callback, &[])?;
    state.set_global(b"rawget", &Value::Function(rawget))?;

    let dofile = state.create_native_function("dofile", dofile::callback, &[])?;
    state.set_global(b"dofile", &Value::Function(dofile))?;

    let select = state.create_native_function("select", select::callback, &[])?;
    state.set_global(b"select", &Value::Function(select))?;

    let pcall = state.create_native_function("pcall", pcall::callback, &[])?;
    state.set_global(b"pcall", &Value::Function(pcall))?;

    Ok(())
}
