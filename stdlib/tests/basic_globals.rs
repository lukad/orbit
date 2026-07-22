use orbit_stdlib::install;
use orbit_vm::{LuaString, NoLoadService, State, Value};

#[test]
fn install_registers_standard_base_library_globals() {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    assert_eq!(
        state.get_global(b"_VERSION").unwrap(),
        Value::String(LuaString::from("Lua 5.4"))
    );

    let Value::Table(installed_globals) = state.get_global(b"_G").unwrap() else {
        panic!("_G was not installed as a table");
    };

    assert_eq!(installed_globals, state.globals().unwrap());
}
