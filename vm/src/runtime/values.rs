use crate::{
    error::{FaultResult, VmErrorKind},
    handle::{Function, Table},
    id::{FunctionId, StateId, TableId},
    value::{RawValue, Value},
};

use super::Runtime;

impl Runtime {
    pub(crate) fn import_value(&self, value: Value) -> FaultResult<RawValue> {
        match value {
            Value::Nil => Ok(RawValue::Nil),
            Value::Boolean(value) => Ok(RawValue::Boolean(value)),
            Value::Integer(value) => Ok(RawValue::Integer(value)),
            Value::Float(value) => Ok(RawValue::Float(value)),
            Value::String(value) => Ok(RawValue::String(value)),
            Value::Table(table) => self.import_table(&table).map(RawValue::Table),
            Value::Function(function) => self.import_function(&function).map(RawValue::Function),
            Value::LightUserdata(value) => Ok(RawValue::LightUserdata(value)),
        }
    }

    pub(crate) fn import_values(
        &self,
        values: impl IntoIterator<Item = Value>,
    ) -> FaultResult<Box<[RawValue]>> {
        values
            .into_iter()
            .map(|value| self.import_value(value))
            .collect::<FaultResult<Vec<_>>>()
            .map(Vec::into_boxed_slice)
    }

    pub(crate) fn export_value(&mut self, value: &RawValue) -> FaultResult<Value> {
        match value {
            RawValue::Nil => Ok(Value::Nil),
            RawValue::Boolean(value) => Ok(Value::Boolean(*value)),
            RawValue::Integer(value) => Ok(Value::Integer(*value)),
            RawValue::Float(value) => Ok(Value::Float(*value)),
            RawValue::String(value) => Ok(Value::String(value.clone())),
            RawValue::Table(id) => self.export_table(*id).map(Value::Table),
            RawValue::Function(id) => self.export_function(*id).map(Value::Function),
            RawValue::LightUserdata(value) => Ok(Value::LightUserdata(*value)),
        }
    }

    pub(crate) fn export_values(&mut self, values: &[RawValue]) -> FaultResult<Vec<Value>> {
        let mut exported = Vec::new();

        for value in values {
            exported.push(self.export_value(value)?);
        }

        Ok(exported)
    }

    pub(crate) fn export_table(&mut self, id: TableId) -> FaultResult<Table> {
        self.heap.table(id)?;

        let table = Table::new(self.id, id);
        self.register_external_root(table.downgrade_root())?;

        Ok(table)
    }

    pub(crate) fn export_function(&mut self, id: FunctionId) -> FaultResult<Function> {
        self.heap.function(id)?;

        let function = Function::new(self.id, id);
        self.register_external_root(function.downgrade_root())?;

        Ok(function)
    }

    pub(crate) fn import_table(&self, table: &Table) -> FaultResult<TableId> {
        self.validate_state("table", table.state_id())?;

        let id = table.id();
        self.heap.table(id)?;

        Ok(id)
    }

    pub(crate) fn import_function(&self, function: &Function) -> FaultResult<FunctionId> {
        self.validate_state("function", function.state_id())?;

        let id = function.id();
        self.heap.function(id)?;

        Ok(id)
    }

    fn validate_state(&self, kind: &'static str, actual: StateId) -> FaultResult<()> {
        if actual == self.id {
            return Ok(());
        }

        Err(VmErrorKind::ForeignObject {
            kind,
            expected_state: self.id.get(),
            actual_state: actual.get(),
        })
    }
}
