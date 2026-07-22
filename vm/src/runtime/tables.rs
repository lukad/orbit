use crate::{
    error::FaultResult, handle::Table, id::TableId, string::LuaString, table::TableData,
    value::RawValue,
};

use super::Runtime;

impl Runtime {
    pub(crate) fn allocate_table(
        &mut self,
        array_hint: usize,
        hash_hint: usize,
    ) -> FaultResult<TableId> {
        let table = TableData::new(array_hint, hash_hint)?;

        self.heap.allocate_table(table)
    }

    pub(crate) fn create_table(
        &mut self,
        array_hint: usize,
        hash_hint: usize,
    ) -> FaultResult<Table> {
        let id = self.allocate_table(array_hint, hash_hint)?;

        self.export_table(id)
    }

    pub(crate) fn raw_get(&self, table: TableId, key: &RawValue) -> FaultResult<RawValue> {
        Ok(self.heap.table(table)?.raw_get(key))
    }

    pub(crate) fn raw_set(
        &mut self,
        table: TableId,
        key: RawValue,
        value: RawValue,
    ) -> FaultResult<()> {
        self.heap.table_mut(table)?.raw_set(key, value)
    }

    pub(crate) fn raw_set_list(
        &mut self,
        table: TableId,
        first_index: u32,
        values: &[RawValue],
    ) -> FaultResult<()> {
        self.heap
            .table_mut(table)?
            .raw_set_list(first_index, values)
    }

    pub(crate) fn raw_len(&self, table: TableId) -> FaultResult<i64> {
        Ok(self.heap.table(table)?.raw_len())
    }

    pub(crate) fn next(
        &self,
        table: TableId,
        previous: &RawValue,
    ) -> FaultResult<Option<(RawValue, RawValue)>> {
        self.heap.table(table)?.next(previous)
    }

    pub(crate) fn get_global(&self, name: &[u8]) -> FaultResult<RawValue> {
        let key = RawValue::String(LuaString::new(name));

        self.raw_get(self.globals, &key)
    }

    pub(crate) fn set_global(&mut self, name: &[u8], value: RawValue) -> FaultResult<()> {
        let key = RawValue::String(LuaString::new(name));

        self.raw_set(self.globals, key, value)
    }
}
