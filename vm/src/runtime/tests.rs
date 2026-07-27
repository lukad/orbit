use orbit_common::{SourceId, Span};
use orbit_compiler::bytecode::{Chunk, Prototype, RegisterRootMap};

use crate::{
    error::VmErrorKind,
    function::FunctionSnapshot,
    loading::NoLoadService,
    value::{RawValue, Value},
};

use super::Runtime;

fn empty_chunk() -> Chunk {
    Chunk {
        strings: Box::new([]),
        entry: Prototype {
            name: None,
            span: Span::new(SourceId::new(0), 0, 0),
            parameter_count: 0,
            is_vararg: true,
            max_registers: 0,
            constants: Box::new([]),
            upvalues: Box::new([]),
            children: Box::new([]),
            code: Box::new([]),
            register_root_maps: vec![RegisterRootMap::EMPTY].into_boxed_slice(),
            close_debug: Box::new([]),
            source_map: Box::new([]),
        },
    }
}

#[test]
fn public_table_round_trips_through_raw_value() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let table = runtime.create_table(0, 0).unwrap();
    let raw = runtime.import_value(Value::Table(table.clone())).unwrap();

    let exported = runtime.export_value(&raw).unwrap();

    assert_eq!(exported, Value::Table(table));
}

#[test]
fn rejects_a_table_from_another_runtime() {
    let mut first = Runtime::new(Box::new(NoLoadService)).unwrap();
    let second = Runtime::new(Box::new(NoLoadService)).unwrap();

    let table = first.create_table(0, 0).unwrap();

    assert_eq!(
        second.import_value(Value::Table(table)).unwrap_err(),
        VmErrorKind::ForeignObject {
            kind: "table",
            expected_state: second.id.get(),
            actual_state: first.id.get(),
        }
    );
}

#[test]
fn globals_are_stored_in_the_global_table() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    runtime
        .set_global(b"answer", RawValue::Integer(42))
        .unwrap();

    assert_eq!(
        runtime.get_global(b"answer").unwrap(),
        RawValue::Integer(42)
    );

    assert_eq!(runtime.get_global(b"missing").unwrap(), RawValue::Nil);
}

#[test]
fn live_public_handles_are_persistent_roots() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();
    let table = runtime.create_table(0, 0).unwrap();
    let object = table.id().object();

    assert!(runtime.persistent_roots().unwrap().contains(&object));

    drop(table);

    assert!(!runtime.persistent_roots().unwrap().contains(&object));
}

#[test]
fn loading_creates_an_ordinary_heap_function() {
    let mut runtime = Runtime::new(Box::new(NoLoadService)).unwrap();

    let function_id = runtime.load_chunk_raw(empty_chunk()).unwrap();

    assert!(matches!(
        runtime.function_snapshot(function_id).unwrap(),
        FunctionSnapshot::Lua(_)
    ));
}
