use crate::{error::FaultResult, id::ObjectId};

use super::Runtime;

impl Runtime {
    pub(crate) fn collection_due(&self) -> bool {
        self.heap.collection_due()
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
