use crate::{error::FaultResult, id::ObjectId, runtime::GcMode};

use super::Runtime;

impl Runtime {
    pub(crate) fn collection_due(&self) -> bool {
        !self.gc_stopped && self.heap.collection_due()
    }

    pub(crate) fn gc_running(&self) -> bool {
        !self.gc_stopped
    }

    pub(crate) fn stop_gc(&mut self) {
        self.gc_stopped = true;
    }

    pub(crate) fn restart_gc(&mut self) {
        self.gc_stopped = false;
        self.heap.reset_allocation_debt();
    }

    pub(crate) fn set_gc_mode(&mut self, mode: GcMode) -> GcMode {
        std::mem::replace(&mut self.gc_mode, mode)
    }

    pub(crate) fn memory_kbytes(&self) -> f64 {
        self.heap.memory_bytes_estimate() as f64 / 1024.0
    }

    pub(crate) fn collect_garbage(&mut self, transient_roots: &[ObjectId]) -> FaultResult<usize> {
        let persistent_roots = self.persistent_roots()?;

        let reclaimed = self.heap.collect_garbage(
            persistent_roots
                .iter()
                .copied()
                .chain(transient_roots.iter().copied()),
        )?;

        Ok(reclaimed)
    }
}
