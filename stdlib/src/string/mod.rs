mod arithmetic;
mod find;
mod format;
mod formatting;
mod offsets;
mod pack;
mod packing;
mod packsize;
mod pattern;
mod rep;
mod sub;
mod unpack;

use orbit_vm::{LuaString, NativeCallback, State, Value, VmResult};

use crate::set_field;

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let string = state.create_table(0, 8)?;

    let sub = state.create_native_function("string.sub", sub::callback, &[])?;
    set_field(state, &string, sub::FUNCTION_NAME, &Value::Function(sub))?;

    let pack = state.create_native_function("string.pack", pack::callback, &[])?;
    set_field(state, &string, pack::FUNCTION_NAME, &Value::Function(pack))?;

    let packsize = state.create_native_function("string.packsize", packsize::callback, &[])?;
    set_field(
        state,
        &string,
        packsize::FUNCTION_NAME,
        &Value::Function(packsize),
    )?;

    let unpack = state.create_native_function("string.unpack", unpack::callback, &[])?;
    set_field(
        state,
        &string,
        unpack::FUNCTION_NAME,
        &Value::Function(unpack),
    )?;

    let Value::Function(tostring) = state.get_global(b"tostring")? else {
        unreachable!("tostring is installed before the string library");
    };
    let format = state.create_native_function(
        "string.format",
        format::callback,
        &[Value::Function(tostring)],
    )?;
    set_field(
        state,
        &string,
        format::FUNCTION_NAME,
        &Value::Function(format),
    )?;

    let rep = state.create_native_function("string.rep", rep::callback, &[])?;
    set_field(state, &string, rep::FUNCTION, &Value::Function(rep))?;

    let find = state.create_native_function("string.find", find::callback, &[])?;
    set_field(state, &string, find::FUNCTION, &Value::Function(find))?;

    let metatable = state.create_table(0, 8)?;
    set_field(state, &metatable, b"__index", &Value::Table(string.clone()))?;

    let arithemtic_metamethods: [(&[u8], &'static str, NativeCallback); 7] = [
        (b"__add", "string.__add", arithmetic::add),
        (b"__sub", "string.__sub", arithmetic::subtract),
        (b"__mul", "string.__mul", arithmetic::multiply),
        (b"__div", "string.__div", arithmetic::divide),
        (b"__idiv", "string.__idiv", arithmetic::floor_divide),
        (b"__mod", "string.__mod", arithmetic::modulo),
        (b"__pow", "string.__pow", arithmetic::power),
    ];
    for (name, function_name, callback) in arithemtic_metamethods.into_iter() {
        let function = state.create_native_function(function_name, callback, &[])?;
        set_field(state, &metatable, name, &Value::Function(function))?;
    }

    state.set_metatable(&Value::String(LuaString::new(b"")), Some(&metatable))?;

    state.set_global(b"string", &Value::Table(string))
}
