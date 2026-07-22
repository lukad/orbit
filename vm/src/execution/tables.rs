use orbit_compiler::bytecode::{Count, Register};

use crate::{
    error::{FaultResult, VmErrorKind},
    execution::Activation,
    value::RawValue,
};

use super::{Execution, FrameBoundary, ResultTarget};

const MAX_METAMETHOD_REDIRECTS: usize = 2_000;
const INDEX_METAMETHOD: &[u8] = b"__index";
const NEW_INDEX_METAMETHOD: &[u8] = b"__newindex";

pub(super) enum IndexOutcome {
    Value(RawValue),
    Invoke {
        callee: RawValue,
        arguments: Box<[RawValue]>,
    },
}

pub(super) enum NewIndexOutcome {
    Done,
    Invoke {
        callee: RawValue,
        arguments: Box<[RawValue]>,
    },
}

impl Execution<'_> {
    pub(super) fn new_table(
        &mut self,
        destination: Register,
        array_hint: u32,
        hash_hint: u32,
    ) -> FaultResult<()> {
        let array_hint =
            usize::try_from(array_hint).map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: usize::MAX,
            })?;

        let hash_hint =
            usize::try_from(hash_hint).map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: usize::MAX,
            })?;

        let table = self.runtime.allocate_table(array_hint, hash_hint)?;

        self.write_register(destination, RawValue::Table(table))
    }

    pub(super) fn get_table(
        &mut self,
        destination: Register,
        table: Register,
        key: Register,
    ) -> FaultResult<Option<FrameBoundary>> {
        let target = self.read_register(table)?;
        let key = self.read_register(key)?;

        match self.resolve_index(target, key)? {
            IndexOutcome::Value(value) => {
                self.write_register(destination, value)?;
                Ok(None)
            }
            IndexOutcome::Invoke { callee, arguments } => Ok(Some(FrameBoundary::Invoke {
                callee,
                arguments,
                target: ResultTarget::Index { destination },
            })),
        }
    }

    pub(super) fn set_table(
        &mut self,
        table: Register,
        key: Register,
        value: Register,
    ) -> FaultResult<Option<FrameBoundary>> {
        let target = self.read_register(table)?;
        let key = self.read_register(key)?;
        let value = self.read_register(value)?;

        match self.resolve_new_index(target, key, value)? {
            NewIndexOutcome::Done => Ok(None),
            NewIndexOutcome::Invoke { callee, arguments } => Ok(Some(FrameBoundary::Invoke {
                callee,
                arguments,
                target: ResultTarget::NewIndex,
            })),
        }
    }

    pub(super) fn resolve_index(
        &mut self,
        mut target: RawValue,
        key: RawValue,
    ) -> FaultResult<IndexOutcome> {
        let mut redirects = 0;

        loop {
            if let Some(table) = target.as_table() {
                let result = self.runtime.raw_get(table, &key)?;

                if !result.is_nil() {
                    return Ok(IndexOutcome::Value(result));
                }
            }

            if redirects == MAX_METAMETHOD_REDIRECTS {
                return Err(VmErrorKind::MetamethodChainTooLong {
                    metamethod: "__index",
                });
            }

            let metamethod = self.runtime.metamethod(&target, INDEX_METAMETHOD)?;

            if metamethod.is_nil() {
                if target.as_table().is_some() {
                    return Ok(IndexOutcome::Value(RawValue::Nil));
                }

                return Err(VmErrorKind::InvalidTableOperand {
                    kind: target.type_name(),
                });
            }

            if matches!(metamethod, RawValue::Function(_)) {
                let arguments = vec![target, key].into_boxed_slice();

                return Ok(IndexOutcome::Invoke {
                    callee: metamethod,
                    arguments,
                });
            }

            redirects += 1;
            target = metamethod;
        }
    }

    pub(super) fn resolve_new_index(
        &mut self,
        mut target: RawValue,
        key: RawValue,
        value: RawValue,
    ) -> FaultResult<NewIndexOutcome> {
        let mut redirects = 0;

        loop {
            if let Some(table) = target.as_table()
                && !self.runtime.raw_get(table, &key)?.is_nil()
            {
                self.runtime.raw_set(table, key, value)?;

                return Ok(NewIndexOutcome::Done);
            }

            if redirects == MAX_METAMETHOD_REDIRECTS {
                return Err(VmErrorKind::MetamethodChainTooLong {
                    metamethod: "__newindex",
                });
            }

            let metamethod = self.runtime.metamethod(&target, NEW_INDEX_METAMETHOD)?;

            if metamethod.is_nil() {
                let Some(table) = target.as_table() else {
                    return Err(VmErrorKind::InvalidTableOperand {
                        kind: target.type_name(),
                    });
                };

                self.runtime.raw_set(table, key, value)?;

                return Ok(NewIndexOutcome::Done);
            }

            if matches!(metamethod, RawValue::Function(_)) {
                let arguments = vec![target, key, value].into_boxed_slice();

                return Ok(NewIndexOutcome::Invoke {
                    callee: metamethod,
                    arguments,
                });
            }

            redirects += 1;
            target = metamethod;
        }
    }

    pub(super) fn set_list(
        &mut self,
        table: Register,
        source: Register,
        first_index: u32,
        count: Count,
    ) -> FaultResult<()> {
        if first_index == 0 {
            return Err(VmErrorKind::InvalidListIndex { first_index });
        }

        let table_value = self.read_register(table)?;

        let table = table_value
            .as_table()
            .ok_or(VmErrorKind::InvalidTableOperand {
                kind: table_value.type_name(),
            })?;

        let values = {
            let runtime = &*self.runtime;

            self.stack
                .last_mut()
                .and_then(Activation::as_lua_mut)
                .expect("active activation is Lua")
                .frame_mut()
                .collect_list_values(runtime, source, count)?
        };

        self.runtime.raw_set_list(table, first_index, &values)
    }
}
