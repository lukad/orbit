mod hash;
mod key;
mod length;
mod traversal;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use crate::{
    error::{FaultResult, VmErrorKind},
    id::{ObjectId, TableId},
    value::RawValue,
};

use self::{
    hash::HashPart,
    key::{KeyNormalization, TableKey, normalize_key},
};

#[derive(Debug)]
pub(crate) struct TableData {
    array: Vec<RawValue>,
    hash: HashPart,
    metatable: Option<TableId>,

    // Array entries can be removed while `next` is traversing. Keeping the
    // deleted positions lets that removed key remain a valid cursor even if
    // trimming shortened the array.
    deleted_array_keys: HashSet<usize>,
}

impl TableData {
    pub(crate) fn new(array_hint: usize, hash_hint: usize) -> FaultResult<Self> {
        let mut array = Vec::new();
        array
            .try_reserve(array_hint)
            .map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: array_hint,
            })?;

        Ok(Self {
            array,
            hash: HashPart::new(hash_hint)?,
            metatable: None,
            deleted_array_keys: HashSet::new(),
        })
    }

    pub(crate) fn raw_get(&self, key: &RawValue) -> RawValue {
        let KeyNormalization::Key(key) = normalize_key(key) else {
            return RawValue::Nil;
        };

        if let Some(one_based) = key.positive_integer_index() {
            let zero_based = one_based - 1;

            if let Some(value) = self.array.get(zero_based) {
                return value.clone();
            }
        }

        self.hash.get(&key).cloned().unwrap_or(RawValue::Nil)
    }

    pub(crate) fn raw_set(&mut self, key: RawValue, value: RawValue) -> FaultResult<()> {
        let key = match normalize_key(&key) {
            KeyNormalization::Key(key) => key,
            KeyNormalization::Nil => {
                return Err(VmErrorKind::NilTableKey);
            }
            KeyNormalization::NaN => {
                return Err(VmErrorKind::NaNTableKey);
            }
        };

        if let TableKey::Integer(integer) = key {
            return self.set_integer(integer, value);
        }

        self.set_hash_key(key, value)
    }

    pub(crate) fn raw_set_list(
        &mut self,
        first_index: u32,
        values: &[RawValue],
    ) -> FaultResult<()> {
        if first_index == 0 {
            return Err(VmErrorKind::InvalidListIndex { first_index });
        }

        if values.is_empty() {
            return Ok(());
        }

        let last_index = u128::from(first_index)
            + u128::try_from(values.len() - 1).map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: values.len(),
            })?;

        if last_index > i64::MAX as u128 {
            return Err(VmErrorKind::TableCapacityExceeded {
                requested: values.len(),
            });
        }

        for (offset, value) in values.iter().enumerate() {
            let index = u128::from(first_index) + offset as u128;
            let index = i64::try_from(index).map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: values.len(),
            })?;

            self.set_integer(index, value.clone())?;
        }

        Ok(())
    }

    pub(crate) fn raw_len(&self) -> i64 {
        length::raw_len(self)
    }

    pub(crate) fn next(&self, previous: &RawValue) -> FaultResult<Option<(RawValue, RawValue)>> {
        traversal::next(self, previous)
    }

    pub(crate) fn metatable(&self) -> Option<TableId> {
        self.metatable
    }

    pub(crate) fn set_metatable(&mut self, metatable: Option<TableId>) -> Option<TableId> {
        std::mem::replace(&mut self.metatable, metatable)
    }

    pub(crate) fn visit_objects(&self, mut visit: impl FnMut(ObjectId)) {
        if let Some(metatable) = self.metatable {
            visit(metatable.object());
        }

        for value in &self.array {
            if let Some(object) = value.object_id() {
                visit(object);
            }
        }

        self.hash.visit_objects(visit);
    }

    fn set_integer(&mut self, integer: i64, value: RawValue) -> FaultResult<()> {
        if integer <= 0 {
            return self.set_hash_key(TableKey::Integer(integer), value);
        }

        let Ok(one_based) = usize::try_from(integer) else {
            return self.set_hash_key(TableKey::Integer(integer), value);
        };

        let zero_based = one_based - 1;

        if zero_based < self.array.len() {
            let existed = !self.array[zero_based].is_nil();

            if value.is_nil() {
                self.array[zero_based] = RawValue::Nil;

                if existed {
                    self.deleted_array_keys.insert(zero_based);
                }

                self.trim_array_tail();
            } else {
                self.array[zero_based] = value;
                self.deleted_array_keys.remove(&zero_based);
            }

            return Ok(());
        }

        if zero_based == self.array.len() && !value.is_nil() {
            self.hash.delete(&TableKey::Integer(integer));

            self.array
                .try_reserve(1)
                .map_err(|_| VmErrorKind::TableCapacityExceeded {
                    requested: self.array.len().saturating_add(1),
                })?;

            self.array.push(value);
            self.deleted_array_keys.remove(&zero_based);
            self.promote_consecutive_hash_entries()?;

            return Ok(());
        }

        self.deleted_array_keys.remove(&zero_based);

        if value.is_nil() {
            self.hash.delete(&TableKey::Integer(integer));
            return Ok(());
        }

        self.hash.insert(TableKey::Integer(integer), value)?;

        self.promote_dense_integer_entries()
    }

    fn set_hash_key(&mut self, key: TableKey, value: RawValue) -> FaultResult<()> {
        if value.is_nil() {
            self.hash.delete(&key);
        } else {
            self.hash.insert(key, value)?;
        }

        Ok(())
    }

    fn promote_consecutive_hash_entries(&mut self) -> FaultResult<()> {
        loop {
            let Some(one_based) = self.array.len().checked_add(1) else {
                return Err(VmErrorKind::TableCapacityExceeded {
                    requested: usize::MAX,
                });
            };

            let Ok(integer) = i64::try_from(one_based) else {
                break;
            };

            let key = TableKey::Integer(integer);
            let Some(value) = self.hash.take_live(&key) else {
                break;
            };

            self.array
                .try_reserve(1)
                .map_err(|_| VmErrorKind::TableCapacityExceeded {
                    requested: one_based,
                })?;

            self.array.push(value);
            self.deleted_array_keys.remove(&(one_based - 1));
        }

        Ok(())
    }

    fn promote_dense_integer_entries(&mut self) -> FaultResult<()> {
        let mut integer_keys = Vec::new();
        integer_keys
            .try_reserve(self.hash.live_len())
            .map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: self.hash.live_len(),
            })?;

        for integer in self.hash.live_integer_keys() {
            if integer <= 0 {
                continue;
            }

            let Ok(one_based) = usize::try_from(integer) else {
                continue;
            };

            if one_based > self.array.len() {
                integer_keys.push(one_based);
            }
        }

        integer_keys.sort_unstable();

        let mut occupied = self.array.iter().filter(|value| !value.is_nil()).count();

        let mut target = self.array.len();

        for one_based in integer_keys {
            occupied = occupied.saturating_add(1);

            if one_based <= occupied.saturating_mul(2) {
                target = target.max(one_based);
            }
        }

        if target <= self.array.len() {
            return Ok(());
        }

        let old_len = self.array.len();

        self.array
            .try_reserve(target - old_len)
            .map_err(|_| VmErrorKind::TableCapacityExceeded { requested: target })?;

        self.array.resize(target, RawValue::Nil);

        for one_based in (old_len + 1)..=target {
            let integer =
                i64::try_from(one_based).expect("promoted indices originated from i64 table keys");
            let key = TableKey::Integer(integer);

            if let Some(value) = self.hash.take_live(&key) {
                self.array[one_based - 1] = value;
                self.deleted_array_keys.remove(&(one_based - 1));
            }
        }

        Ok(())
    }

    fn trim_array_tail(&mut self) {
        while self.array.last().is_some_and(RawValue::is_nil) {
            self.array.pop();
        }
    }

    fn contains_integer(&self, integer: i64) -> bool {
        !self.raw_get(&RawValue::Integer(integer)).is_nil()
    }

    fn is_array_cursor(&self, zero_based: usize) -> bool {
        self.array
            .get(zero_based)
            .is_some_and(|value| !value.is_nil())
            || self.deleted_array_keys.contains(&zero_based)
    }
}
