mod path;
mod require;
mod searcher_lua;
mod searcher_preload;
mod searchpath;

use std::env;

use orbit_vm::{LocalValue, LuaString, NativeContext, State, Value, VmResult};

use crate::{error, set_field};

const DEFAULT_PATH: &[u8] = b"./?.lua;./?/init.lua";

#[cfg(windows)]
const PACKAGE_CONFIG: &[u8] = b"\\\n;\n?\n!\n-\n";

#[cfg(not(windows))]
const PACKAGE_CONFIG: &[u8] = b"/\n;\n?\n!\n-\n";

pub(crate) fn install(state: &mut State) -> VmResult<()> {
    let package = state.create_table(0, 7)?;
    let loaded = state.create_table(0, 16)?;
    let preload = state.create_table(0, 16)?;
    let searchers = state.create_table(2, 0)?;

    let preload_searcher = state.create_native_function(
        "package preload searcher",
        searcher_preload::callback,
        &[Value::Table(preload.clone())],
    )?;

    let lua_searcher = state.create_native_function(
        "package Lua searcher",
        searcher_lua::callback,
        &[Value::Table(package.clone())],
    )?;

    state.raw_set(
        &searchers,
        &Value::Integer(1),
        &Value::Function(preload_searcher),
    )?;

    state.raw_set(
        &searchers,
        &Value::Integer(2),
        &Value::Function(lua_searcher),
    )?;

    let searchpath =
        state.create_native_function("package.searchpath", searchpath::callback, &[])?;

    set_field(state, &package, b"loaded", &Value::Table(loaded.clone()))?;
    set_field(state, &loaded, b"package", &Value::Table(package.clone()))?;
    set_field(state, &package, b"preload", &Value::Table(preload))?;
    set_field(state, &package, b"searchers", &Value::Table(searchers))?;
    set_field(state, &package, b"path", &Value::String(initial_path()))?;

    set_field(
        state,
        &package,
        b"cpath",
        &Value::String(LuaString::from("")),
    )?;

    set_field(
        state,
        &package,
        b"config",
        &Value::String(LuaString::from(PACKAGE_CONFIG)),
    )?;

    set_field(state, &package, b"searchpath", &Value::Function(searchpath))?;

    let require = state.create_native_function(
        "require",
        require::callback,
        &[Value::Table(package.clone()), Value::Table(loaded)],
    )?;

    state.set_global(b"package", &Value::Table(package))?;
    state.set_global(b"require", &Value::Function(require))?;

    Ok(())
}

/// Resolves the initial Lua module path using Lua 5.4's environment-variable
/// precedence and `;;` default-path expansion.
fn initial_path() -> LuaString {
    let Some(configured) = env::var_os("LUA_PATH_5_4").or_else(|| env::var_os("LUA_PATH")) else {
        return LuaString::from(DEFAULT_PATH);
    };

    let configured = configured.as_encoded_bytes();
    let mut expanded = Vec::new();
    let mut remaining = configured;

    while let Some(index) = remaining.windows(2).position(|bytes| bytes == b";;") {
        expanded.extend_from_slice(&remaining[..index]);
        expanded.push(b';');
        expanded.extend_from_slice(DEFAULT_PATH);
        expanded.push(b';');
        remaining = &remaining[index + 2..];
    }

    expanded.extend_from_slice(remaining);
    LuaString::from(expanded)
}

pub(crate) fn check_string<'context>(
    context: &NativeContext<'context>,
    index: usize,
    function: &'static str,
) -> VmResult<LocalValue<'context>> {
    let value = context
        .argument(index)
        .ok_or_else(|| error::type_error(function, index + 1, "string", None))?;

    match value.type_name() {
        "string" => Ok(value),
        "number" => Ok(context.default_tostring(&value, None)),
        _ => Err(error::type_error(
            function,
            index + 1,
            "string",
            Some(value.type_name()),
        )),
    }
}
