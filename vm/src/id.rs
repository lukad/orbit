use std::{
    num::{NonZero, NonZeroU64},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_STATE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct StateId(NonZeroU64);

impl StateId {
    pub(crate) fn fresh() -> Self {
        let value = NEXT_STATE_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("state id overflow");

        Self(NonZero::new(value).expect("state ids start at one"))
    }

    pub(crate) const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ObjectId {
    slot: u32,
    generation: u32,
}

impl ObjectId {
    pub(crate) const fn new(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }

    pub(crate) const fn slot(self) -> u32 {
        self.slot
    }

    pub(crate) const fn generation(self) -> u32 {
        self.generation
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TableId(ObjectId);

impl TableId {
    pub(crate) const fn from_object(object: ObjectId) -> Self {
        Self(object)
    }

    pub(crate) const fn object(self) -> ObjectId {
        self.0
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FunctionId(ObjectId);

impl FunctionId {
    pub(crate) const fn from_object(object: ObjectId) -> Self {
        Self(object)
    }

    pub(crate) const fn object(self) -> ObjectId {
        self.0
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct UpvalueId(ObjectId);

impl UpvalueId {
    pub(crate) const fn from_object(object: ObjectId) -> Self {
        Self(object)
    }

    pub(crate) const fn object(self) -> ObjectId {
        self.0
    }
}
