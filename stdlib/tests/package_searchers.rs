use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use orbit_loader::Loader;
use orbit_stdlib::install;
use orbit_vm::{
    CallOutcome, LoadError, LuaString, NativeAction, NativeContext, NativeEvent, NativeToken,
    State, Value, VmErrorKind, VmResult,
};

static NEXT_SCRIPT_ID: AtomicU64 = AtomicU64::new(0);
const INDEX_RESUME: NativeToken = NativeToken::new(1);

struct Script {
    path: PathBuf,
}

impl Script {
    fn new(source: &str) -> Self {
        let id = NEXT_SCRIPT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orbit-package-searcher-test-{}-{id}.lua",
            std::process::id(),
        ));

        fs::write(&path, source).unwrap();
        Self { path }
    }

    fn value(&self) -> Value {
        Value::String(LuaString::new(self.path.to_string_lossy().as_bytes()))
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn execute(state: &mut State, source: &str) -> VmResult<Vec<Value>> {
    let function = state.load_buffer("package-searcher-test", source)?;

    match state.call(&function, &[])? {
        CallOutcome::Returned(values) => Ok(values),
        CallOutcome::Yielded { .. } => panic!("test chunk unexpectedly yielded"),
    }
}

fn yielding_index(context: &mut NativeContext<'_>) -> VmResult<NativeAction> {
    match context.event() {
        NativeEvent::Start => {
            Ok(context.yield_values([context.string("path requested")], INDEX_RESUME))
        }
        NativeEvent::Resume {
            token: INDEX_RESUME,
        } => Ok(context.return_values([context.resume_value(0).unwrap_or_else(|| context.nil())])),
        NativeEvent::ResumeError {
            token: INDEX_RESUME,
        } => Err(context
            .resume_error()
            .expect("ResumeError must contain an error")
            .clone()),
        NativeEvent::Resume { .. } | NativeEvent::ResumeError { .. } => {
            Err(VmErrorKind::InvalidNativeContinuation {
                message: "unexpected yielding __index continuation",
            }
            .into())
        }
    }
}

#[test]
fn preload_searcher_honors_the_preload_tables_index_metamethod() {
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let values = execute(
        &mut state,
        r#"
            setmetatable(package.preload, {
                __index = function(_, key)
                    if key == "virtual" then
                        return function(name)
                            return name
                        end
                    end
                end,
            })

            local module, loader_data = require("virtual")
            return module, loader_data
        "#,
    )
    .unwrap();

    assert_eq!(
        values,
        vec![
            Value::String(LuaString::from("virtual")),
            Value::String(LuaString::from(":preload:")),
        ]
    );
}

#[test]
fn lua_searcher_preserves_state_while_package_path_index_yields() {
    let module = Script::new("return 73");
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();

    let Value::Table(package) = state.get_global(b"package").unwrap() else {
        panic!("package was not installed as a table");
    };

    state
        .raw_set(
            &package,
            &Value::String(LuaString::from("path")),
            &Value::Nil,
        )
        .unwrap();

    let index = state
        .create_native_function("yielding package __index", yielding_index, &[])
        .unwrap();
    let metatable = state.create_table(0, 1).unwrap();
    state
        .raw_set(
            &metatable,
            &Value::String(LuaString::from("__index")),
            &Value::Function(index),
        )
        .unwrap();
    state
        .set_metatable(&Value::Table(package), Some(&metatable))
        .unwrap();

    let Value::Function(require) = state.get_global(b"require").unwrap() else {
        panic!("require was not installed as a function");
    };

    let CallOutcome::Yielded {
        values,
        mut suspension,
    } = state
        .call(&require, &[Value::String(LuaString::from("yielded-path"))])
        .unwrap()
    else {
        panic!("package.path __index did not yield");
    };

    assert_eq!(
        values,
        vec![Value::String(LuaString::from("path requested"))]
    );
    suspension.collect_garbage().unwrap();

    let CallOutcome::Returned(values) = suspension.resume(&[module.value()]).unwrap() else {
        panic!("resumed require unexpectedly yielded again");
    };

    assert_eq!(values, vec![Value::Integer(73), module.value()]);
}

#[test]
fn lua_searcher_does_not_flatten_structured_load_errors() {
    let module = Script::new("return (");
    let mut state = State::new(Loader::new()).unwrap();
    install(&mut state).unwrap();
    state.set_global(b"module_path", &module.value()).unwrap();

    let error = execute(
        &mut state,
        r#"
            package.path = module_path
            return require("broken")
        "#,
    )
    .unwrap_err();

    assert!(matches!(
        error.kind,
        VmErrorKind::LoadFailure(LoadError::Parse(_))
    ));
}
