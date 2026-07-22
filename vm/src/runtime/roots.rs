use std::rc::Weak;

use crate::{
    error::{FaultResult, VmErrorKind},
    handle::ExternalRoot,
    id::ObjectId,
    runtime::Runtime,
};

impl Runtime {
    pub(crate) fn register_external_root(&mut self, root: Weak<ExternalRoot>) -> FaultResult<()> {
        self.prune_external_roots();

        let requested = self.external_roots.len().saturating_add(1);

        self.external_roots
            .try_reserve(1)
            .map_err(|_| VmErrorKind::RootCapacityExceeded { requested })?;

        self.external_roots.push(root);

        Ok(())
    }

    pub(crate) fn prune_external_roots(&mut self) {
        self.external_roots.retain(|root| root.strong_count() != 0);
    }

    pub(crate) fn persistent_roots(&mut self) -> FaultResult<Box<[ObjectId]>> {
        self.prune_external_roots();

        let requested = 1_usize
            .saturating_add(self.type_metatables.tables().count())
            .saturating_add(self.external_roots.len());

        let mut roots = Vec::new();

        roots
            .try_reserve(requested)
            .map_err(|_| VmErrorKind::RootCapacityExceeded { requested })?;

        roots.push(self.globals.object());

        roots.extend(
            self.type_metatables
                .tables()
                .map(|metatable| metatable.object()),
        );

        for weak_root in &self.external_roots {
            let Some(root) = weak_root.upgrade() else {
                continue;
            };

            debug_assert_eq!(root.state(), self.id);
            roots.push(root.object().object_id());
        }

        Ok(roots.into_boxed_slice())
    }
}
