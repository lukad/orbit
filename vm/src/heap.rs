use crate::{
    LuaString,
    error::{FaultResult, VmErrorKind},
    function::FunctionData,
    id::{FunctionId, ObjectId, TableId, UpvalueId},
    table::TableData,
    upvalue::UpvalueData,
    value::RawValue,
};

const INITIAL_GENERATION: u32 = 1;
const DEFAULT_GC_THRESHOLD: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WeakMode {
    Strong,
    Keys,
    Values,
    KeysAndValues,
}

impl WeakMode {
    fn weak_keys(self) -> bool {
        matches!(self, Self::Keys | Self::KeysAndValues)
    }

    fn weak_values(self) -> bool {
        matches!(self, Self::Values | Self::KeysAndValues)
    }
}

#[derive(Clone, Copy)]
struct WeakTable {
    table: TableId,
    mode: WeakMode,
}

struct MarkState {
    pending: Vec<ObjectId>,
    weak_tables: Vec<WeakTable>,
    ephemerons: Vec<TableId>,
    marked_count: usize,
}

impl MarkState {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
            weak_tables: Vec::new(),
            ephemerons: Vec::new(),
            marked_count: 0,
        }
    }

    fn enqueue(&mut self, id: ObjectId) -> FaultResult<()> {
        push_pending(&mut self.pending, id)
    }

    fn record_weak_table(&mut self, table: TableId, mode: WeakMode) -> FaultResult<()> {
        let weak_requested = self.weak_tables.len().saturating_add(1);
        self.weak_tables
            .try_reserve(1)
            .map_err(|_| VmErrorKind::HeapCapacityExceeded {
                requested: weak_requested,
            })?;

        if mode == WeakMode::Keys {
            let ephemeron_requested = self.ephemerons.len().saturating_add(1);
            self.ephemerons
                .try_reserve(1)
                .map_err(|_| VmErrorKind::HeapCapacityExceeded {
                    requested: ephemeron_requested,
                })?;
        }

        self.weak_tables.push(WeakTable { table, mode });

        if mode == WeakMode::Keys {
            self.ephemerons.push(table);
        }

        Ok(())
    }
}

struct MarkSnapshot {
    generations: Vec<Option<u32>>,
}

impl MarkSnapshot {
    fn new(slots: &[HeapSlot]) -> FaultResult<Self> {
        let requested = slots.len();
        let mut generations = Vec::new();

        generations
            .try_reserve(requested)
            .map_err(|_| VmErrorKind::HeapCapacityExceeded { requested })?;

        generations.extend(slots.iter().map(|slot| {
            if slot.marked && slot.object.is_some() {
                Some(slot.generation)
            } else {
                None
            }
        }));

        Ok(Self { generations })
    }

    fn is_marked(&self, id: ObjectId) -> bool {
        self.generations
            .get(id.slot() as usize)
            .is_some_and(|generation| *generation == Some(id.generation()))
    }
}

pub(crate) struct Heap {
    slots: Vec<HeapSlot>,
    free: Vec<u32>,
    allocation_debt: usize,
    next_gc: usize,
}

impl Heap {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            allocation_debt: 0,
            next_gc: DEFAULT_GC_THRESHOLD,
        }
    }

    pub(crate) fn allocate_table(&mut self, table: TableData) -> FaultResult<TableId> {
        self.allocate_object(HeapObject::Table(table))
            .map(TableId::from_object)
    }

    pub(crate) fn allocate_function(&mut self, function: FunctionData) -> FaultResult<FunctionId> {
        self.allocate_object(HeapObject::Function(function))
            .map(FunctionId::from_object)
    }

    pub(crate) fn allocate_upvalue(&mut self, upvalue: UpvalueData) -> FaultResult<UpvalueId> {
        self.allocate_object(HeapObject::Upvalue(upvalue))
            .map(UpvalueId::from_object)
    }

    pub(crate) fn table(&self, id: TableId) -> FaultResult<&TableData> {
        match self.object(id.object())? {
            HeapObject::Table(table) => Ok(table),
            object => Err(wrong_kind("table", object.kind())),
        }
    }

    pub(crate) fn table_mut(&mut self, id: TableId) -> FaultResult<&mut TableData> {
        match self.object_mut(id.object())? {
            HeapObject::Table(table) => Ok(table),
            object => Err(wrong_kind("table", object.kind())),
        }
    }

    pub(crate) fn function(&self, id: FunctionId) -> FaultResult<&FunctionData> {
        match self.object(id.object())? {
            HeapObject::Function(function) => Ok(function),
            object => Err(wrong_kind("function", object.kind())),
        }
    }

    pub(crate) fn upvalue(&self, id: UpvalueId) -> FaultResult<&UpvalueData> {
        match self.object(id.object())? {
            HeapObject::Upvalue(upvalue) => Ok(upvalue),
            object => Err(wrong_kind("upvalue", object.kind())),
        }
    }

    pub(crate) fn upvalue_mut(&mut self, id: UpvalueId) -> FaultResult<&mut UpvalueData> {
        match self.object_mut(id.object())? {
            HeapObject::Upvalue(upvalue) => Ok(upvalue),
            object => Err(wrong_kind("upvalue", object.kind())),
        }
    }

    pub(crate) fn occupied_len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.object.is_some())
            .count()
    }

    pub(crate) fn memory_bytes_estimate(&self) -> usize {
        self.occupied_len() * size_of::<HeapSlot>()
    }

    pub(crate) fn record_allocation_debt(&mut self, units: usize) {
        self.allocation_debt = self.allocation_debt.saturating_add(units);
    }

    #[cfg(test)]
    pub(crate) fn allocation_debt(&self) -> usize {
        self.allocation_debt
    }

    pub(crate) fn reset_allocation_debt(&mut self) {
        self.allocation_debt = 0;
    }

    pub(crate) fn collection_due(&self) -> bool {
        self.allocation_debt >= self.next_gc
    }

    pub(crate) fn record_collection(&mut self, next_gc: usize) {
        self.allocation_debt = 0;
        self.next_gc = next_gc.max(1);
    }

    pub(crate) fn collect_garbage(
        &mut self,
        roots: impl IntoIterator<Item = ObjectId>,
    ) -> FaultResult<usize> {
        self.clear_marks();

        let weak_tables = match self.mark_reachable(roots) {
            Ok(weak_tables) => weak_tables,
            Err(error) => {
                self.clear_marks();
                return Err(error);
            }
        };

        let reusable = self
            .slots
            .iter()
            .filter(|slot| slot.object.is_some() && !slot.marked && slot.generation != u32::MAX)
            .count();

        let requested = self.free.len().saturating_add(reusable);

        if self.free.try_reserve(reusable).is_err() {
            self.clear_marks();

            return Err(VmErrorKind::HeapCapacityExceeded { requested });
        }

        let marks = match MarkSnapshot::new(&self.slots) {
            Ok(marks) => marks,
            Err(error) => {
                self.clear_marks();
                return Err(error);
            }
        };

        for weak in weak_tables {
            let table = match self.table_mut(weak.table) {
                Ok(table) => table,
                Err(error) => {
                    self.clear_marks();
                    return Err(error);
                }
            };

            table.clear_weak_entries(weak.mode.weak_keys(), weak.mode.weak_values(), |id| {
                marks.is_marked(id)
            });
        }

        let mut reclaimed = 0;

        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.object.is_none() {
                continue;
            }

            if slot.marked {
                slot.marked = false;
                continue;
            }

            slot.object = None;
            reclaimed += 1;

            let Some(next_generation) = slot.generation.checked_add(1) else {
                // Retire an exhausted slot permanently. Wrapping would
                // allow an ancient ObjectId to become valid again.
                continue;
            };

            slot.generation = next_generation;
            self.free
                .push(u32::try_from(index).expect("allocated heap slot indices fit in u32"));
        }

        let next_gc = self
            .occupied_len()
            .saturating_mul(2)
            .max(DEFAULT_GC_THRESHOLD);

        self.record_collection(next_gc);

        Ok(reclaimed)
    }

    fn mark_reachable(
        &mut self,
        roots: impl IntoIterator<Item = ObjectId>,
    ) -> FaultResult<Vec<WeakTable>> {
        let mut state = MarkState::new();

        for root in roots {
            state.enqueue(root)?;
        }

        self.drain_pending(&mut state)?;

        loop {
            let marked_before = state.marked_count;
            let mut ephemeron_index = 0;

            while ephemeron_index < state.ephemerons.len() {
                let table = self.table(state.ephemerons[ephemeron_index])?;
                ephemeron_index += 1;

                let mut trace_error = None;

                table.visit_hash_entries(|key, value| {
                    if trace_error.is_some() {
                        return;
                    }

                    let key_reachable = match key.object_id() {
                        None => true,
                        Some(key) => match self.is_marked(key) {
                            Ok(marked) => marked,
                            Err(error) => {
                                trace_error = Some(error);
                                return;
                            }
                        },
                    };

                    if key_reachable
                        && let Some(value) = value.object_id()
                        && let Err(error) = state.enqueue(value)
                    {
                        trace_error = Some(error);
                    }
                });

                if let Some(error) = trace_error {
                    return Err(error);
                }
            }

            self.drain_pending(&mut state)?;

            if state.marked_count == marked_before {
                break;
            }
        }

        Ok(state.weak_tables)
    }

    fn drain_pending(&mut self, state: &mut MarkState) -> FaultResult<()> {
        while let Some(id) = state.pending.pop() {
            let slot = self.resolve_slot_mut(id)?;

            if slot.marked {
                continue;
            }

            slot.marked = true;
            state.marked_count = state.marked_count.saturating_add(1);

            match self.object(id)? {
                HeapObject::Table(table) => {
                    self.trace_table(TableId::from_object(id), table, state)?;
                }
                object => {
                    let mut capacity_error = None;

                    object.visit_objects(|child| {
                        if capacity_error.is_some() {
                            return;
                        }

                        if let Err(error) = state.enqueue(child) {
                            capacity_error = Some(error);
                        }
                    });

                    if let Some(error) = capacity_error {
                        return Err(error);
                    }
                }
            }
        }

        Ok(())
    }

    fn trace_table(
        &self,
        id: TableId,
        table: &TableData,
        state: &mut MarkState,
    ) -> FaultResult<()> {
        let mode = self.table_weak_mode(table)?;
        let mut trace_error = None;

        table.visit_metatable(|metatable| {
            if let Err(error) = state.enqueue(metatable) {
                trace_error = Some(error);
            }
        });

        if let Some(error) = trace_error.take() {
            return Err(error);
        }

        if mode != WeakMode::Strong {
            state.record_weak_table(id, mode)?;
        }

        if !mode.weak_values() {
            table.visit_array_values(|value| {
                if trace_error.is_some() {
                    return;
                }

                if let Err(error) = state.enqueue(value) {
                    trace_error = Some(error);
                }
            });
        }

        if let Some(error) = trace_error.take() {
            return Err(error);
        }

        table.visit_hash_entries(|key, value| {
            if trace_error.is_some() {
                return;
            }

            // Keys are strong in Strong and Values modes.
            if !mode.weak_keys()
                && let Some(key) = key.object_id()
                && let Err(error) = state.enqueue(key)
            {
                trace_error = Some(error);
                return;
            }

            let value_is_strong = match mode {
                WeakMode::Strong => true,
                WeakMode::Values | WeakMode::KeysAndValues => false,
                WeakMode::Keys => match key.object_id() {
                    None => true,
                    Some(key) => match self.is_marked(key) {
                        Ok(marked) => marked,
                        Err(error) => {
                            trace_error = Some(error);
                            return;
                        }
                    },
                },
            };

            if value_is_strong
                && let Some(value) = value.object_id()
                && let Err(error) = state.enqueue(value)
            {
                trace_error = Some(error);
            }
        });

        if let Some(error) = trace_error {
            return Err(error);
        }

        Ok(())
    }

    fn is_marked(&self, id: ObjectId) -> FaultResult<bool> {
        Ok(self.resolve_slot(id)?.marked)
    }

    fn clear_marks(&mut self) {
        for slot in &mut self.slots {
            slot.marked = false;
        }
    }

    fn allocate_object(&mut self, object: HeapObject) -> FaultResult<ObjectId> {
        if let Some(slot_index) = self.free.pop() {
            let slot = self
                .slots
                .get_mut(slot_index as usize)
                .expect("free-list slot always exists");

            debug_assert!(
                slot.object.is_none(),
                "free-list slot must not contain a live object"
            );

            slot.marked = false;
            slot.object = Some(object);

            let generation = slot.generation;

            self.record_allocation_debt(1);

            return Ok(ObjectId::new(slot_index, generation));
        }

        let slot_index =
            u32::try_from(self.slots.len()).map_err(|_| VmErrorKind::HeapCapacityExceeded {
                requested: self.slots.len().saturating_add(1),
            })?;

        self.slots
            .try_reserve(1)
            .map_err(|_| VmErrorKind::HeapCapacityExceeded {
                requested: self.slots.len().saturating_add(1),
            })?;

        self.slots.push(HeapSlot {
            generation: INITIAL_GENERATION,
            marked: false,
            object: Some(object),
        });

        self.record_allocation_debt(1);

        Ok(ObjectId::new(slot_index, INITIAL_GENERATION))
    }

    fn object(&self, id: ObjectId) -> FaultResult<&HeapObject> {
        self.resolve_slot(id)?
            .object
            .as_ref()
            .ok_or_else(|| dangling(id))
    }

    fn object_mut(&mut self, id: ObjectId) -> FaultResult<&mut HeapObject> {
        self.resolve_slot_mut(id)?
            .object
            .as_mut()
            .ok_or_else(|| dangling(id))
    }

    fn resolve_slot(&self, id: ObjectId) -> FaultResult<&HeapSlot> {
        let slot = self
            .slots
            .get(id.slot() as usize)
            .ok_or_else(|| dangling(id))?;

        if slot.generation != id.generation() || slot.object.is_none() {
            return Err(dangling(id));
        }

        Ok(slot)
    }

    fn resolve_slot_mut(&mut self, id: ObjectId) -> FaultResult<&mut HeapSlot> {
        let slot = self
            .slots
            .get_mut(id.slot() as usize)
            .ok_or_else(|| dangling(id))?;

        if slot.generation != id.generation() || slot.object.is_none() {
            return Err(dangling(id));
        }

        Ok(slot)
    }

    fn table_weak_mode(&self, table: &TableData) -> FaultResult<WeakMode> {
        let Some(metatable) = table.metatable() else {
            return Ok(WeakMode::Strong);
        };

        let mode = self
            .table(metatable)?
            .raw_get(&RawValue::String(LuaString::from("__mode")));

        let RawValue::String(mode) = mode else {
            return Ok(WeakMode::Strong);
        };

        let weak_keys = mode.as_bytes().contains(&b'k');
        let weak_values = mode.as_bytes().contains(&b'v');

        Ok(match (weak_keys, weak_values) {
            (false, false) => WeakMode::Strong,
            (true, false) => WeakMode::Keys,
            (false, true) => WeakMode::Values,
            (true, true) => WeakMode::KeysAndValues,
        })
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

enum HeapObject {
    Table(TableData),
    Function(FunctionData),
    Upvalue(UpvalueData),
}

impl HeapObject {
    fn kind(&self) -> &'static str {
        match self {
            Self::Table(_) => "table",
            Self::Function(_) => "function",
            Self::Upvalue(_) => "upvalue",
        }
    }

    fn visit_objects(&self, visit: impl FnMut(ObjectId)) {
        match self {
            Self::Table(_table) => unreachable!("tables require mode-aware tracing"),
            Self::Function(function) => function.visit_objects(visit),
            Self::Upvalue(upvalue) => upvalue.visit_objects(visit),
        }
    }
}

struct HeapSlot {
    generation: u32,
    marked: bool,
    object: Option<HeapObject>,
}

fn dangling(id: ObjectId) -> VmErrorKind {
    VmErrorKind::DanglingObject {
        slot: id.slot(),
        generation: id.generation(),
    }
}

fn wrong_kind(expected: &'static str, actual: &'static str) -> VmErrorKind {
    VmErrorKind::WrongObjectKind { expected, actual }
}

fn push_pending(pending: &mut Vec<ObjectId>, id: ObjectId) -> FaultResult<()> {
    let requested = pending.len().saturating_add(1);

    pending
        .try_reserve(1)
        .map_err(|_| VmErrorKind::HeapCapacityExceeded { requested })?;

    pending.push(id);

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        error::VmErrorKind,
        id::{ObjectId, TableId},
        table::TableData,
        upvalue::UpvalueData,
        value::RawValue,
    };

    use super::Heap;

    #[test]
    fn allocates_and_resolves_typed_objects() {
        let mut heap = Heap::new();

        let table_id = heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();

        let upvalue_id = heap
            .allocate_upvalue(UpvalueData::new(RawValue::Integer(42)))
            .unwrap();

        assert_eq!(heap.occupied_len(), 2);

        heap.table_mut(table_id)
            .unwrap()
            .raw_set(RawValue::Integer(1), RawValue::Integer(10))
            .unwrap();

        assert_eq!(
            heap.table(table_id).unwrap().raw_get(&RawValue::Integer(1)),
            RawValue::Integer(10)
        );

        assert_eq!(
            heap.upvalue(upvalue_id).unwrap().value(),
            &RawValue::Integer(42)
        );
    }

    #[test]
    fn rejects_an_id_with_the_wrong_object_kind() {
        let mut heap = Heap::new();

        let upvalue_id = heap
            .allocate_upvalue(UpvalueData::new(RawValue::Nil))
            .unwrap();

        let forged_table_id = TableId::from_object(upvalue_id.object());

        assert_eq!(
            heap.table(forged_table_id).unwrap_err(),
            VmErrorKind::WrongObjectKind {
                expected: "table",
                actual: "upvalue",
            }
        );
    }

    #[test]
    fn rejects_an_out_of_bounds_object_id() {
        let heap = Heap::new();
        let id = TableId::from_object(ObjectId::new(100, 1));

        assert_eq!(
            heap.table(id).unwrap_err(),
            VmErrorKind::DanglingObject {
                slot: 100,
                generation: 1,
            }
        );
    }

    #[test]
    fn rejects_the_wrong_generation() {
        let mut heap = Heap::new();

        let table_id = heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();

        let object = table_id.object();
        let wrong_generation = object.generation().wrapping_add(1);

        let stale_id = TableId::from_object(ObjectId::new(object.slot(), wrong_generation));

        assert_eq!(
            heap.table(stale_id).unwrap_err(),
            VmErrorKind::DanglingObject {
                slot: object.slot(),
                generation: wrong_generation,
            }
        );
    }

    #[test]
    fn tracks_allocation_debt_without_collecting_implicitly() {
        let mut heap = Heap::new();

        assert_eq!(heap.allocation_debt(), 0);
        assert!(!heap.collection_due());

        heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();

        assert_eq!(heap.allocation_debt(), 1);

        heap.record_collection(1);

        assert_eq!(heap.allocation_debt(), 0);
        assert!(!heap.collection_due());

        heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();

        assert!(heap.collection_due());
    }

    #[test]
    fn collection_traces_cycles_and_reclaims_unreachable_objects() {
        let mut heap = Heap::new();
        let first = heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();
        let second = heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();

        heap.table_mut(first)
            .unwrap()
            .raw_set(RawValue::Integer(1), RawValue::Table(second))
            .unwrap();

        heap.table_mut(second)
            .unwrap()
            .raw_set(RawValue::Integer(1), RawValue::Table(first))
            .unwrap();

        assert_eq!(heap.collect_garbage([first.object()]).unwrap(), 0);
        assert!(heap.table(first).is_ok());
        assert!(heap.table(second).is_ok());

        assert_eq!(heap.collect_garbage([]).unwrap(), 2);

        assert!(matches!(
            heap.table(first),
            Err(VmErrorKind::DanglingObject { .. })
        ));
        assert!(matches!(
            heap.table(second),
            Err(VmErrorKind::DanglingObject { .. })
        ));
    }

    #[test]
    fn collection_increments_generation_before_reusing_a_slot() {
        let mut heap = Heap::new();
        let stale = heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();

        assert_eq!(heap.collect_garbage([]).unwrap(), 1);

        let replacement = heap.allocate_table(TableData::new(0, 0).unwrap()).unwrap();

        assert_eq!(stale.object().slot(), replacement.object().slot());
        assert_ne!(
            stale.object().generation(),
            replacement.object().generation()
        );

        assert!(matches!(
            heap.table(stale),
            Err(VmErrorKind::DanglingObject { .. })
        ));
        assert!(heap.table(replacement).is_ok());
    }
}
