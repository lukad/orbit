use orbit_compiler::bytecode::Chunk;

use crate::{
    LoadService,
    error::{VmError, VmResult},
    execution::{Execution, ExecutionOutcome, SuspendedExecution},
    handle::{Function, Table},
    native::NativeCallback,
    runtime::Runtime,
    value::Value,
};

#[cfg(test)]
mod tests;

pub struct State {
    runtime: Runtime,
}

#[must_use]
pub enum CallOutcome<'state> {
    Returned(Vec<Value>),
    Yielded {
        values: Vec<Value>,
        suspension: SuspendedCall<'state>,
    },
}

#[must_use]
pub struct SuspendedCall<'state> {
    inner: SuspendedExecution<'state>,
}

impl State {
    pub fn new(load_service: impl LoadService + 'static) -> VmResult<Self> {
        let runtime = Runtime::new(Box::new(load_service))?;
        Ok(Self { runtime })
    }

    pub fn load(&mut self, chunk: Chunk) -> VmResult<Function> {
        self.collect_if_due()?;

        let function_id = self.runtime.load_raw(chunk)?;
        let function = self.runtime.export_function(function_id)?;

        Ok(function)
    }

    pub fn load_buffer(
        &mut self,
        name: impl AsRef<[u8]>,
        source: impl AsRef<[u8]>,
    ) -> VmResult<Function> {
        self.collect_if_due()?;

        let function_id = self
            .runtime
            .load_buffer_raw(name.as_ref(), source.as_ref())?;

        let function = self.runtime.export_function(function_id)?;
        Ok(function)
    }

    pub fn load_file(&mut self, filename: impl AsRef<[u8]>) -> VmResult<Function> {
        self.collect_if_due()?;

        let function_id = self.runtime.load_file_raw(filename.as_ref())?;
        let function = self.runtime.export_function(function_id)?;
        Ok(function)
    }

    pub fn load_stdin(&mut self) -> VmResult<Function> {
        self.collect_if_due()?;

        let function_id = self.runtime.load_stdin_raw()?;
        let function = self.runtime.export_function(function_id)?;
        Ok(function)
    }

    pub fn call<'state>(
        &'state mut self,
        function: &Function,
        arguments: &[Value],
    ) -> VmResult<CallOutcome<'state>> {
        self.collect_if_due()?;

        let function = self
            .runtime
            .import_function(function)
            .map_err(VmError::from)?;

        let arguments = self
            .runtime
            .import_values(arguments.iter().cloned())
            .map_err(VmError::from)?;

        let function = self
            .runtime
            .function_snapshot(function)
            .map_err(VmError::from)?;

        let execution =
            Execution::new(&mut self.runtime, function, arguments).map_err(VmError::from)?;

        export_outcome(execution.run()?)
    }

    pub fn create_table(&mut self, array_hint: usize, hash_hint: usize) -> VmResult<Table> {
        self.collect_if_due()?;

        self.runtime
            .create_table(array_hint, hash_hint)
            .map_err(VmError::from)
    }

    pub fn create_native_function(
        &mut self,
        name: impl Into<Box<str>>,
        callback: NativeCallback,
        captures: &[Value],
    ) -> VmResult<Function> {
        self.collect_if_due()?;

        let captures = self
            .runtime
            .import_values(captures.iter().cloned())
            .map_err(VmError::from)?;

        self.runtime
            .create_native_function(name, callback, captures)
            .map_err(VmError::from)
    }

    pub fn globals(&mut self) -> VmResult<Table> {
        self.collect_if_due()?;

        let globals = self.runtime.globals();

        self.runtime.export_table(globals).map_err(VmError::from)
    }

    pub fn get_global(&mut self, name: impl AsRef<[u8]>) -> VmResult<Value> {
        self.collect_if_due()?;

        let value = self
            .runtime
            .get_global(name.as_ref())
            .map_err(VmError::from)?;

        self.runtime.export_value(&value).map_err(VmError::from)
    }

    pub fn set_global(&mut self, name: impl AsRef<[u8]>, value: &Value) -> VmResult<()> {
        self.collect_if_due()?;

        let value = self
            .runtime
            .import_value(value.clone())
            .map_err(VmError::from)?;

        self.runtime
            .set_global(name.as_ref(), value)
            .map_err(VmError::from)
    }

    pub fn raw_get(&mut self, table: &Table, key: &Value) -> VmResult<Value> {
        self.collect_if_due()?;

        let table = self.runtime.import_table(table).map_err(VmError::from)?;

        let key = self
            .runtime
            .import_value(key.clone())
            .map_err(VmError::from)?;

        let value = self.runtime.raw_get(table, &key).map_err(VmError::from)?;

        self.runtime.export_value(&value).map_err(VmError::from)
    }

    pub fn raw_set(&mut self, table: &Table, key: &Value, value: &Value) -> VmResult<()> {
        self.collect_if_due()?;

        let table = self.runtime.import_table(table).map_err(VmError::from)?;

        let key = self
            .runtime
            .import_value(key.clone())
            .map_err(VmError::from)?;

        let value = self
            .runtime
            .import_value(value.clone())
            .map_err(VmError::from)?;

        self.runtime
            .raw_set(table, key, value)
            .map_err(VmError::from)
    }

    pub fn raw_len(&self, table: &Table) -> VmResult<i64> {
        let table = self.runtime.import_table(table).map_err(VmError::from)?;

        self.runtime.raw_len(table).map_err(VmError::from)
    }

    pub fn get_metatable(&mut self, value: &Value) -> VmResult<Option<Table>> {
        self.collect_if_due()?;

        let value = self
            .runtime
            .import_value(value.clone())
            .map_err(VmError::from)?;

        let Some(metatable) = self.runtime.metatable(&value).map_err(VmError::from)? else {
            return Ok(None);
        };

        self.runtime
            .export_table(metatable)
            .map(Some)
            .map_err(VmError::from)
    }

    pub fn set_metatable(
        &mut self,
        value: &Value,
        metatable: Option<&Table>,
    ) -> VmResult<Option<Table>> {
        self.collect_if_due()?;

        let value = self
            .runtime
            .import_value(value.clone())
            .map_err(VmError::from)?;

        let metatable = metatable
            .map(|metatable| self.runtime.import_table(metatable))
            .transpose()
            .map_err(VmError::from)?;

        let previous = self
            .runtime
            .set_metatable(&value, metatable)
            .map_err(VmError::from)?;

        match previous {
            Some(previous) => self
                .runtime
                .export_table(previous)
                .map(Some)
                .map_err(VmError::from),

            None => Ok(None),
        }
    }

    pub fn next(&mut self, table: &Table, previous: &Value) -> VmResult<Option<(Value, Value)>> {
        self.collect_if_due()?;

        let table = self.runtime.import_table(table).map_err(VmError::from)?;

        let previous = self
            .runtime
            .import_value(previous.clone())
            .map_err(VmError::from)?;

        let Some((key, value)) = self.runtime.next(table, &previous).map_err(VmError::from)? else {
            return Ok(None);
        };

        let key = self.runtime.export_value(&key).map_err(VmError::from)?;
        let value = self.runtime.export_value(&value).map_err(VmError::from)?;

        Ok(Some((key, value)))
    }

    pub fn collect_garbage(&mut self) -> VmResult<usize> {
        self.runtime.collect_garbage(&[]).map_err(VmError::from)
    }

    fn collect_if_due(&mut self) -> VmResult<()> {
        if self.runtime.collection_due() {
            self.collect_garbage()?;
        }

        Ok(())
    }
}

impl<'state> SuspendedCall<'state> {
    pub fn resume(self, values: &[Value]) -> VmResult<CallOutcome<'state>> {
        let values = self.inner.import_values(values).map_err(VmError::from)?;

        export_outcome(self.inner.resume(values)?)
    }

    pub fn resume_error(self, error: VmError) -> VmResult<CallOutcome<'state>> {
        export_outcome(self.inner.resume_error(error)?)
    }

    pub fn collect_garbage(&mut self) -> VmResult<usize> {
        self.inner.collect_garbage().map_err(VmError::from)
    }
}

fn export_outcome<'state>(outcome: ExecutionOutcome<'state>) -> VmResult<CallOutcome<'state>> {
    match outcome {
        ExecutionOutcome::Returned { values, runtime } => {
            let values = runtime.export_values(&values).map_err(VmError::from)?;

            Ok(CallOutcome::Returned(values))
        }

        ExecutionOutcome::Yielded {
            values,
            mut suspension,
        } => {
            let values = suspension.export_values(&values).map_err(VmError::from)?;

            Ok(CallOutcome::Yielded {
                values,
                suspension: SuspendedCall { inner: suspension },
            })
        }
    }
}
