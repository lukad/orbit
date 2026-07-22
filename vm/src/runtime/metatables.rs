use crate::{error::FaultResult, id::TableId, string::LuaString, value::RawValue};

use super::Runtime;

const TYPE_METATABLE_COUNT: usize = 5;

#[derive(Clone, Copy)]
enum TypeMetatable {
    Nil = 0,
    Boolean = 1,
    Number = 2,
    String = 3,
    Function = 4,
}

impl TypeMetatable {
    fn for_value(value: &RawValue) -> Option<Self> {
        match value {
            RawValue::Nil => Some(Self::Nil),
            RawValue::Boolean(_) => Some(Self::Boolean),
            RawValue::Integer(_) | RawValue::Float(_) => Some(Self::Number),
            RawValue::String(_) => Some(Self::String),
            RawValue::Function(_) => Some(Self::Function),
            RawValue::Table(_) => None,
        }
    }
}

pub(super) struct TypeMetatables {
    entries: [Option<TableId>; TYPE_METATABLE_COUNT],
}

impl TypeMetatables {
    pub(super) fn new() -> Self {
        Self {
            entries: [None; TYPE_METATABLE_COUNT],
        }
    }

    fn get(&self, value: &RawValue) -> Option<TableId> {
        let kind = TypeMetatable::for_value(value)?;

        self.entries[kind as usize]
    }

    fn replace(&mut self, value: &RawValue, metatable: Option<TableId>) -> Option<TableId> {
        let kind = TypeMetatable::for_value(value)
            .expect("tables store their metatable on the table object");

        std::mem::replace(&mut self.entries[kind as usize], metatable)
    }

    pub(super) fn tables(&self) -> impl Iterator<Item = TableId> + '_ {
        self.entries.iter().flatten().copied()
    }
}

impl Runtime {
    pub(crate) fn metatable(&self, value: &RawValue) -> FaultResult<Option<TableId>> {
        match value {
            RawValue::Table(table) => Ok(self.heap.table(*table)?.metatable()),
            value => Ok(self.type_metatables.get(value)),
        }
    }

    pub(crate) fn set_metatable(
        &mut self,
        value: &RawValue,
        metatable: Option<TableId>,
    ) -> FaultResult<Option<TableId>> {
        if let Some(metatable) = metatable {
            self.heap.table(metatable)?;
        }

        match value {
            RawValue::Table(table) => Ok(self.heap.table_mut(*table)?.set_metatable(metatable)),
            value => Ok(self.type_metatables.replace(value, metatable)),
        }
    }

    pub(crate) fn metamethod(
        &self,
        value: &RawValue,
        name: &'static [u8],
    ) -> FaultResult<RawValue> {
        let Some(metatable) = self.metatable(value)? else {
            return Ok(RawValue::Nil);
        };

        let key = RawValue::String(LuaString::new(name));

        self.raw_get(metatable, &key)
    }
}
