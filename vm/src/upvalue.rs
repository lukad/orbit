use crate::{id::UpvalueId, value::RawValue};

#[derive(Debug, Clone)]
pub(crate) struct UpvalueData {
    value: RawValue,
}

impl UpvalueData {
    pub(crate) fn new(value: RawValue) -> Self {
        Self { value }
    }

    pub(crate) fn value(&self) -> &RawValue {
        &self.value
    }

    pub(crate) fn replace(&mut self, value: RawValue) -> RawValue {
        std::mem::replace(&mut self.value, value)
    }

    pub(crate) fn visit_objects(&self, mut visit: impl FnMut(crate::id::ObjectId)) {
        if let Some(object) = self.value.object_id() {
            visit(object);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RegisterSlot {
    Direct(RawValue),
    Captured(UpvalueId),
}

impl RegisterSlot {
    pub(crate) fn direct(value: RawValue) -> Self {
        Self::Direct(value)
    }

    pub(crate) fn captured_id(&self) -> Option<UpvalueId> {
        match self {
            Self::Captured(upvalue) => Some(*upvalue),
            Self::Direct(_) => None,
        }
    }

    pub(crate) fn direct_value(&self) -> Option<&RawValue> {
        match self {
            Self::Direct(value) => Some(value),
            Self::Captured(_) => None,
        }
    }

    pub(crate) fn direct_value_mut(&mut self) -> Option<&mut RawValue> {
        match self {
            Self::Direct(value) => Some(value),
            Self::Captured(_) => None,
        }
    }

    pub(crate) fn capture(&mut self, upvalue: UpvalueId) -> Option<RawValue> {
        if matches!(self, Self::Captured(_)) {
            return None;
        }

        match std::mem::replace(self, Self::Captured(upvalue)) {
            Self::Direct(value) => Some(value),
            Self::Captured(_) => unreachable!("captured slot was rejected above"),
        }
    }
}
