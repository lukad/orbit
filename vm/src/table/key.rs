use crate::{
    id::{FunctionId, ObjectId, TableId},
    number::float_to_integer,
    string::LuaString,
    value::RawValue,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum TableKey {
    Boolean(bool),
    Integer(i64),
    Float(FloatKey),
    String(LuaString),
    Table(TableId),
    Function(FunctionId),
}

impl TableKey {
    pub(super) fn positive_integer_index(&self) -> Option<usize> {
        match self {
            Self::Integer(value) if *value > 0 => usize::try_from(*value).ok(),
            _ => None,
        }
    }

    pub(super) fn to_raw_value(&self) -> RawValue {
        match self {
            Self::Boolean(value) => RawValue::Boolean(*value),
            Self::Integer(value) => RawValue::Integer(*value),
            Self::Float(value) => RawValue::Float(value.value()),
            Self::String(value) => RawValue::String(value.clone()),
            Self::Table(id) => RawValue::Table(*id),
            Self::Function(id) => RawValue::Function(*id),
        }
    }

    pub(super) fn object_id(&self) -> Option<ObjectId> {
        match self {
            Self::Table(id) => Some(id.object()),
            Self::Function(id) => Some(id.object()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct FloatKey(u64);

impl FloatKey {
    fn new(value: f64) -> Self {
        debug_assert!(!value.is_nan());
        debug_assert!(float_to_integer(value).is_none());

        Self(value.to_bits())
    }

    fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

pub(super) enum KeyNormalization {
    Key(TableKey),
    Nil,
    NaN,
}

pub(super) fn normalize_key(value: &RawValue) -> KeyNormalization {
    match value {
        RawValue::Nil => KeyNormalization::Nil,
        RawValue::Boolean(value) => KeyNormalization::Key(TableKey::Boolean(*value)),
        RawValue::Integer(value) => KeyNormalization::Key(TableKey::Integer(*value)),
        RawValue::Float(value) => {
            if value.is_nan() {
                KeyNormalization::NaN
            } else if let Some(integer) = float_to_integer(*value) {
                KeyNormalization::Key(TableKey::Integer(integer))
            } else {
                KeyNormalization::Key(TableKey::Float(FloatKey::new(*value)))
            }
        }
        RawValue::String(value) => KeyNormalization::Key(TableKey::String(value.clone())),
        RawValue::Table(id) => KeyNormalization::Key(TableKey::Table(*id)),
        RawValue::Function(id) => KeyNormalization::Key(TableKey::Function(*id)),
    }
}
