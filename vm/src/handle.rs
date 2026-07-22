use std::{
    hash::{Hash, Hasher},
    marker::PhantomData,
    rc::{Rc, Weak},
};

use crate::id::{FunctionId, ObjectId, StateId, TableId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RootedObject {
    Table(TableId),
    Function(FunctionId),
}

impl RootedObject {
    pub(crate) const fn object_id(self) -> ObjectId {
        match self {
            Self::Table(id) => id.object(),
            Self::Function(id) => id.object(),
        }
    }
}

pub(crate) struct ExternalRoot {
    state: StateId,
    object: RootedObject,
}

impl ExternalRoot {
    fn new(state: StateId, object: RootedObject) -> Rc<Self> {
        Rc::new(Self { state, object })
    }

    pub(crate) const fn state(&self) -> StateId {
        self.state
    }

    pub(crate) const fn object(&self) -> RootedObject {
        self.object
    }
}

pub(crate) struct Handle<Tag> {
    root: Rc<ExternalRoot>,
    marker: PhantomData<fn() -> Tag>,
}

impl<Tag> Handle<Tag> {
    fn new(state: StateId, object: RootedObject) -> Self {
        Self {
            root: ExternalRoot::new(state, object),
            marker: PhantomData,
        }
    }

    pub(crate) fn state(&self) -> StateId {
        self.root.state()
    }

    pub(crate) fn rooted_object(&self) -> RootedObject {
        self.root.object()
    }

    pub(crate) fn downgrade_root(&self) -> Weak<ExternalRoot> {
        Rc::downgrade(&self.root)
    }
}

impl<Tag> Clone for Handle<Tag> {
    fn clone(&self) -> Self {
        Self {
            root: Rc::clone(&self.root),
            marker: PhantomData,
        }
    }
}

impl<Tag> PartialEq for Handle<Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.root.state == other.root.state && self.root.object == other.root.object
    }
}

impl<Tag> Eq for Handle<Tag> {}

impl<Tag> Hash for Handle<Tag> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root.state.hash(state);
        self.root.object.hash(state);
    }
}

impl<Tag> std::fmt::Debug for Handle<Tag> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("state", &self.root.state)
            .field("object", &self.root.object)
            .finish()
    }
}

enum TableTag {}
enum FunctionTag {}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Table(Handle<TableTag>);

impl Table {
    pub(crate) fn new(state: StateId, id: TableId) -> Self {
        Self(Handle::new(state, RootedObject::Table(id)))
    }

    pub(crate) fn state_id(&self) -> StateId {
        self.0.state()
    }

    pub(crate) fn id(&self) -> TableId {
        match self.0.rooted_object() {
            RootedObject::Table(id) => id,
            RootedObject::Function(_) => unreachable!("table handle contains a function root"),
        }
    }

    pub(crate) fn downgrade_root(&self) -> Weak<ExternalRoot> {
        self.0.downgrade_root()
    }
}

impl std::fmt::Debug for Table {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Table").field(&self.0).finish()
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Function(Handle<FunctionTag>);

impl Function {
    pub(crate) fn new(state: StateId, id: FunctionId) -> Self {
        Self(Handle::new(state, RootedObject::Function(id)))
    }

    pub(crate) fn state_id(&self) -> StateId {
        self.0.state()
    }

    pub(crate) fn id(&self) -> FunctionId {
        match self.0.rooted_object() {
            RootedObject::Function(id) => id,
            RootedObject::Table(_) => unreachable!("function handle contains a table root"),
        }
    }

    pub(crate) fn downgrade_root(&self) -> Weak<ExternalRoot> {
        self.0.downgrade_root()
    }
}

impl std::fmt::Debug for Function {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Function").field(&self.0).finish()
    }
}
