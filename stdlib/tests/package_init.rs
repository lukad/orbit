use std::{
    ffi::{OsStr, OsString},
    sync::Mutex,
};

use orbit_stdlib::install;
use orbit_vm::{LuaString, NoLoadService, State, Table, Value};

const DEFAULT_PATH: &str = "./?.lua;./?/init.lua";
static ENVIRONMENT: Mutex<()> = Mutex::new(());

struct PathEnvironment {
    versioned: Option<OsString>,
    generic: Option<OsString>,
}

impl PathEnvironment {
    fn save() -> Self {
        Self {
            versioned: std::env::var_os("LUA_PATH_5_4"),
            generic: std::env::var_os("LUA_PATH"),
        }
    }

    fn set(versioned: Option<&OsStr>, generic: Option<&OsStr>) {
        set_variable("LUA_PATH_5_4", versioned);
        set_variable("LUA_PATH", generic);
    }
}

impl Drop for PathEnvironment {
    fn drop(&mut self) {
        set_variable("LUA_PATH_5_4", self.versioned.as_deref());
        set_variable("LUA_PATH", self.generic.as_deref());
    }
}

fn set_variable(name: &str, value: Option<&OsStr>) {
    // This integration-test process serializes all of its environment changes
    // with `ENVIRONMENT` and does not start any other threads.
    unsafe {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}

fn installed_package() -> (State, Table) {
    let mut state = State::new(NoLoadService).unwrap();
    install(&mut state).unwrap();

    let Value::Table(package) = state.get_global(b"package").unwrap() else {
        panic!("package was not installed as a table");
    };

    (state, package)
}

fn package_path(state: &mut State, package: &Table) -> LuaString {
    let Value::String(path) = state
        .raw_get(package, &Value::String(LuaString::from("path")))
        .unwrap()
    else {
        panic!("package.path was not installed as a string");
    };

    path
}

#[test]
fn package_initialization_matches_lua_path_rules_and_seeds_loaded() {
    let _environment_lock = ENVIRONMENT.lock().unwrap();
    let _saved_environment = PathEnvironment::save();

    PathEnvironment::set(
        Some(OsStr::new("versioned/?.lua;;tail/?.lua")),
        Some(OsStr::new("generic/?.lua")),
    );

    let (mut state, package) = installed_package();
    assert_eq!(
        package_path(&mut state, &package),
        LuaString::from(format!("versioned/?.lua;{DEFAULT_PATH};tail/?.lua").as_str())
    );

    let Value::Table(loaded) = state
        .raw_get(&package, &Value::String(LuaString::from("loaded")))
        .unwrap()
    else {
        panic!("package.loaded was not installed as a table");
    };

    assert_eq!(
        state
            .raw_get(&loaded, &Value::String(LuaString::from("package")))
            .unwrap(),
        Value::Table(package)
    );

    PathEnvironment::set(None, Some(OsStr::new("generic/?.lua;;generic/?/init.lua")));

    let (mut state, package) = installed_package();
    assert_eq!(
        package_path(&mut state, &package),
        LuaString::from(format!("generic/?.lua;{DEFAULT_PATH};generic/?/init.lua").as_str())
    );

    PathEnvironment::set(None, None);

    let (mut state, package) = installed_package();
    assert_eq!(
        package_path(&mut state, &package),
        LuaString::from(DEFAULT_PATH)
    );
}
