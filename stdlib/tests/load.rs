use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{CallOutcome, LuaString, State, Value};

fn load_error(state: &mut State, source: &str) -> String {
    let Value::Function(load) = state.get_global(b"load").unwrap() else {
        panic!("load was not installed as a function");
    };

    let outcome = state
        .call(&load, &[Value::String(LuaString::from(source))])
        .unwrap();
    let CallOutcome::Returned(values) = outcome else {
        panic!("load unexpectedly yielded");
    };
    let [Value::Nil, Value::String(message)] = values.as_slice() else {
        panic!("load did not return nil and an error message");
    };

    String::from_utf8(message.as_bytes().to_vec()).unwrap()
}

#[test]
fn syntax_errors_include_lua_compatible_names_lines_and_messages() {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let cases = [
        ("local x <XXX> = 10", ":1: unknown attribute 'XXX'"),
        (
            "local xxx <const> = 20; xxx = 10",
            ":1: attempt to assign to const variable 'xxx'",
        ),
        (
            "local xx;\nlocal xxx <const> = 20;\nlocal yyy;\nlocal function foo ()\nlocal abc = xx + yyy + xxx;\nreturn function () return function () xxx = yyy end end\nend",
            ":6: attempt to assign to const variable 'xxx'",
        ),
        (
            "local x <close> = nil\nx = io.open()",
            ":2: attempt to assign to const variable 'x'",
        ),
    ];

    for (source, expected) in cases {
        let message = load_error(&mut state, source);
        assert!(
            message.contains(expected),
            "expected {message:?} to contain {expected:?}"
        );
    }
}
