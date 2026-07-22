use crate::{error::VmErrorKind, string::LuaString, value::RawValue};

use super::TableData;

fn new_table() -> TableData {
    TableData::new(0, 0).unwrap()
}

#[test]
fn array_hint_reserves_capacity_without_creating_entries() {
    let table = TableData::new(32, 0).unwrap();

    assert_eq!(table.array.len(), 0);
    assert!(table.raw_get(&RawValue::Integer(1)).is_nil());
}

#[test]
fn sequential_integer_keys_use_the_array_part() {
    let mut table = new_table();

    table
        .raw_set(RawValue::Integer(1), RawValue::Integer(10))
        .unwrap();
    table
        .raw_set(RawValue::Integer(2), RawValue::Integer(20))
        .unwrap();

    assert_eq!(table.array.len(), 2);
    assert_eq!(table.hash.live_len(), 0);
}

#[test]
fn sparse_large_integer_stays_in_the_hash_part() {
    let mut table = new_table();

    table
        .raw_set(RawValue::Integer(1_000_000_000), RawValue::Boolean(true))
        .unwrap();

    assert_eq!(table.array.len(), 0);
    assert_eq!(table.hash.live_len(), 1);
    assert_eq!(
        table.raw_get(&RawValue::Integer(1_000_000_000)),
        RawValue::Boolean(true)
    );
}

#[test]
fn filling_a_gap_promotes_consecutive_hash_entries() {
    let mut table = new_table();

    table
        .raw_set(RawValue::Integer(4), RawValue::Integer(40))
        .unwrap();
    table
        .raw_set(RawValue::Integer(1), RawValue::Integer(10))
        .unwrap();
    table
        .raw_set(RawValue::Integer(2), RawValue::Integer(20))
        .unwrap();
    table
        .raw_set(RawValue::Integer(3), RawValue::Integer(30))
        .unwrap();

    assert_eq!(table.array.len(), 4);
    assert_eq!(table.hash.live_len(), 0);

    for index in 1..=4 {
        assert_eq!(
            table.raw_get(&RawValue::Integer(index)),
            RawValue::Integer(index * 10)
        );
    }
}

#[test]
fn integral_float_keys_are_integer_keys() {
    let mut table = new_table();

    table
        .raw_set(RawValue::Float(1.0), RawValue::Integer(42))
        .unwrap();

    assert_eq!(table.raw_get(&RawValue::Integer(1)), RawValue::Integer(42));

    table
        .raw_set(RawValue::Float(-0.0), RawValue::Integer(7))
        .unwrap();

    assert_eq!(table.raw_get(&RawValue::Integer(0)), RawValue::Integer(7));
}

#[test]
fn nil_and_nan_writes_fail_but_reads_return_nil() {
    let mut table = new_table();

    assert_eq!(
        table
            .raw_set(RawValue::Nil, RawValue::Integer(1))
            .unwrap_err(),
        VmErrorKind::NilTableKey
    );

    assert_eq!(
        table
            .raw_set(RawValue::Float(f64::NAN), RawValue::Integer(1))
            .unwrap_err(),
        VmErrorKind::NaNTableKey
    );

    assert!(table.raw_get(&RawValue::Nil).is_nil());
    assert!(table.raw_get(&RawValue::Float(f64::NAN)).is_nil());
}

#[test]
fn nil_assignment_deletes_entries() {
    let mut table = new_table();

    table
        .raw_set(
            RawValue::String(LuaString::from("name")),
            RawValue::Integer(42),
        )
        .unwrap();

    table
        .raw_set(RawValue::String(LuaString::from("name")), RawValue::Nil)
        .unwrap();

    assert!(
        table
            .raw_get(&RawValue::String(LuaString::from("name")))
            .is_nil()
    );
}

#[test]
fn set_list_writes_consecutive_one_based_keys() {
    let mut table = new_table();

    table
        .raw_set_list(
            2,
            &[
                RawValue::Integer(20),
                RawValue::Integer(30),
                RawValue::Integer(40),
            ],
        )
        .unwrap();

    assert_eq!(table.raw_get(&RawValue::Integer(2)), RawValue::Integer(20));
    assert_eq!(table.raw_get(&RawValue::Integer(3)), RawValue::Integer(30));
    assert_eq!(table.raw_get(&RawValue::Integer(4)), RawValue::Integer(40));
}

#[test]
fn sequence_length_finds_the_non_nil_border() {
    let mut table = new_table();

    for index in 1..=10 {
        table
            .raw_set(RawValue::Integer(index), RawValue::Boolean(true))
            .unwrap();
    }

    assert_eq!(table.raw_len(), 10);

    table.raw_set(RawValue::Integer(10), RawValue::Nil).unwrap();

    assert_eq!(table.raw_len(), 9);
}

#[test]
fn deleted_hash_key_remains_a_valid_next_cursor() {
    let mut table = new_table();
    let first = RawValue::String(LuaString::from("first"));
    let second = RawValue::String(LuaString::from("second"));

    table.raw_set(first.clone(), RawValue::Integer(1)).unwrap();
    table.raw_set(second.clone(), RawValue::Integer(2)).unwrap();

    table.raw_set(first.clone(), RawValue::Nil).unwrap();

    assert_eq!(
        table.next(&first).unwrap(),
        Some((second, RawValue::Integer(2)))
    );
}

#[test]
fn deleted_array_tail_remains_a_valid_next_cursor() {
    let mut table = new_table();

    table
        .raw_set(RawValue::Integer(1), RawValue::Integer(10))
        .unwrap();
    table
        .raw_set(RawValue::Integer(2), RawValue::Integer(20))
        .unwrap();
    table.raw_set(RawValue::Integer(2), RawValue::Nil).unwrap();

    assert_eq!(table.next(&RawValue::Integer(2)).unwrap(), None);
}

#[test]
fn next_rejects_a_key_that_never_belonged_to_the_table() {
    let table = new_table();

    assert_eq!(
        table
            .next(&RawValue::String(LuaString::from("missing")))
            .unwrap_err(),
        VmErrorKind::InvalidKeyToNext
    );
}
