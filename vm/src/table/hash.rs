use std::collections::HashMap;

use crate::{
    error::{FaultResult, VmErrorKind},
    table::TableKey,
    value::RawValue,
};

#[derive(Debug)]
pub(super) struct HashPart {
    positions: HashMap<TableKey, usize>,
    slots: Vec<HashSlot>,
    live: usize,
    dead: usize,
    rehash_at: usize,
}

impl HashPart {
    pub(super) fn new(capacity: usize) -> FaultResult<Self> {
        let mut positions = HashMap::new();
        positions
            .try_reserve(capacity)
            .map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: capacity,
            })?;

        let mut slots = Vec::new();
        slots
            .try_reserve(capacity)
            .map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: capacity,
            })?;

        Ok(Self {
            positions,
            slots,
            live: 0,
            dead: 0,
            rehash_at: capacity.max(8),
        })
    }

    pub(super) fn get(&self, key: &TableKey) -> Option<&RawValue> {
        let position = *self.positions.get(key)?;

        match self.slots.get(position)? {
            HashSlot::Live { value, .. } => Some(value),
            HashSlot::Dead { .. } => None,
        }
    }

    pub(super) fn insert(
        &mut self,
        key: TableKey,
        value: RawValue,
    ) -> FaultResult<Option<RawValue>> {
        debug_assert!(!value.is_nil());

        if let Some(&position) = self.positions.get(&key) {
            let slot = self
                .slots
                .get_mut(position)
                .expect("hash position always refers to an existing slot");

            match slot {
                HashSlot::Live {
                    value: existing, ..
                } => {
                    return Ok(Some(std::mem::replace(existing, value)));
                }
                HashSlot::Dead { key: dead_key } => {
                    let revived_key = dead_key.clone();
                    *slot = HashSlot::Live {
                        key: revived_key,
                        value,
                    };
                    self.dead -= 1;
                    self.live += 1;
                    return Ok(None);
                }
            }
        }

        self.compact_if_needed()?;

        let requested = self.slots.len().saturating_add(1);

        self.positions
            .try_reserve(1)
            .map_err(|_| VmErrorKind::TableCapacityExceeded { requested })?;

        self.slots
            .try_reserve(1)
            .map_err(|_| VmErrorKind::TableCapacityExceeded { requested })?;

        let position = self.slots.len();
        self.positions.insert(key.clone(), position);
        self.slots.push(HashSlot::Live { key, value });
        self.live += 1;

        Ok(None)
    }

    pub(super) fn delete(&mut self, key: &TableKey) -> Option<RawValue> {
        let position = *self.positions.get(key)?;
        let slot = self.slots.get_mut(position)?;

        if matches!(slot, HashSlot::Dead { .. }) {
            return None;
        }

        let previous = std::mem::replace(slot, HashSlot::Dead { key: key.clone() });

        match previous {
            HashSlot::Live { value, .. } => {
                self.live -= 1;
                self.dead += 1;
                Some(value)
            }
            HashSlot::Dead { .. } => {
                unreachable!("dead slot was rejected before replacement")
            }
        }
    }

    pub(super) fn take_live(&mut self, key: &TableKey) -> Option<RawValue> {
        self.delete(key)
    }

    pub(super) fn position(&self, key: &TableKey) -> Option<usize> {
        self.positions.get(key).copied()
    }

    pub(super) fn next_live_from(&self, start: usize) -> Option<(usize, &TableKey, &RawValue)> {
        self.slots
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(position, slot)| match slot {
                HashSlot::Live { key, value } => Some((position, key, value)),
                HashSlot::Dead { .. } => None,
            })
    }

    pub(super) fn live_integer_keys(&self) -> impl Iterator<Item = i64> + '_ {
        self.slots.iter().filter_map(|slot| match slot {
            HashSlot::Live {
                key: TableKey::Integer(value),
                ..
            } => Some(*value),
            _ => None,
        })
    }

    pub(super) fn live_len(&self) -> usize {
        self.live
    }

    pub(super) fn visit_live(&self, mut visit: impl FnMut(&TableKey, &RawValue)) {
        for slot in &self.slots {
            if let HashSlot::Live { key, value } = slot {
                visit(key, value);
            }
        }
    }

    pub(super) fn tombstone_where(
        &mut self,
        mut should_remove: impl FnMut(&TableKey, &RawValue) -> bool,
    ) {
        for slot in &mut self.slots {
            let key = match slot {
                HashSlot::Live { key, value } if should_remove(key, value) => key.clone(),
                HashSlot::Live { .. } | HashSlot::Dead { .. } => continue,
            };

            *slot = HashSlot::Dead { key };
            self.live -= 1;
            self.dead += 1;
        }
    }

    fn compact_if_needed(&mut self) -> FaultResult<()> {
        if self.dead <= self.live || self.slots.len() < self.rehash_at {
            return Ok(());
        }

        let mut positions = HashMap::new();
        positions
            .try_reserve(self.live)
            .map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: self.live,
            })?;

        let mut slots = Vec::new();
        slots
            .try_reserve(self.live)
            .map_err(|_| VmErrorKind::TableCapacityExceeded {
                requested: self.live,
            })?;

        for slot in &self.slots {
            let HashSlot::Live { key, value } = slot else {
                continue;
            };

            let position = slots.len();
            positions.insert(key.clone(), position);
            slots.push(HashSlot::Live {
                key: key.clone(),
                value: value.clone(),
            });
        }

        self.positions = positions;
        self.slots = slots;
        self.dead = 0;
        self.rehash_at = self.slots.len().saturating_mul(2).max(8);

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) enum HashSlot {
    Live { key: TableKey, value: RawValue },
    Dead { key: TableKey },
}
