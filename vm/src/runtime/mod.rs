mod functions;
mod gc;
mod metatables;
mod roots;
mod tables;
mod values;

#[cfg(test)]
mod tests;

use std::{collections::HashSet, rc::Weak};

use orbit_common::SourceId;

use crate::{
    LoadService,
    error::FaultResult,
    handle::ExternalRoot,
    heap::Heap,
    id::{StateId, TableId},
    table::TableData,
};

pub(crate) struct Runtime {
    id: StateId,
    heap: Heap,
    globals: TableId,
    type_metatables: metatables::TypeMetatables,
    external_roots: Vec<Weak<ExternalRoot>>,
    load_service: Box<dyn LoadService>,
    next_source_id: u64,
    source_ids: HashSet<SourceId>,
}

impl Runtime {
    pub(crate) fn new(load_service: Box<dyn LoadService>) -> FaultResult<Self> {
        let id = StateId::fresh();
        let mut heap = Heap::new();

        let globals = heap.allocate_table(TableData::new(0, 32)?)?;

        Ok(Self {
            id,
            heap,
            globals,
            type_metatables: metatables::TypeMetatables::new(),
            external_roots: Vec::new(),
            load_service,
            next_source_id: 0,
            source_ids: HashSet::new(),
        })
    }

    pub(crate) fn globals(&self) -> TableId {
        self.globals
    }

    pub(crate) fn file_exists(&self, filename: &[u8]) -> bool {
        self.load_service.file_exists(filename)
    }
}
