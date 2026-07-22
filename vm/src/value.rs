use crate::{
    handle::{Function, Table},
    id::{FunctionId, ObjectId, TableId},
    string::LuaString,
};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(LuaString),
    Table(Table),
    Function(Function),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Integer(_) | Self::Float(_) => "number",
            Self::String(_) => "string",
            Self::Table(_) => "table",
            Self::Function(_) => "function",
        }
    }

    pub fn is_truthy(&self) -> bool {
        !self.is_falsy()
    }

    pub fn is_falsy(&self) -> bool {
        matches!(self, Self::Nil | Self::Boolean(false))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum RawValue {
    #[default]
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(LuaString),
    Table(TableId),
    Function(FunctionId),
}

impl RawValue {
    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Integer(_) | Self::Float(_) => "number",
            Self::String(_) => "string",
            Self::Table(_) => "table",
            Self::Function(_) => "function",
        }
    }

    pub(crate) fn is_nil(&self) -> bool {
        matches!(self, Self::Nil)
    }

    pub(crate) fn is_falsy(&self) -> bool {
        matches!(self, Self::Nil | Self::Boolean(false))
    }

    pub(crate) fn is_truthy(&self) -> bool {
        !self.is_falsy()
    }

    pub(crate) fn object_id(&self) -> Option<ObjectId> {
        match self {
            Self::Table(id) => Some(id.object()),
            Self::Function(id) => Some(id.object()),
            _ => None,
        }
    }

    pub(crate) fn as_boolean(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn as_string(&self) -> Option<&LuaString> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn as_table(&self) -> Option<TableId> {
        match self {
            Self::Table(id) => Some(*id),
            _ => None,
        }
    }

    pub(crate) fn to_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Float(value) => crate::number::float_to_integer(*value),
            _ => None,
        }
    }

    pub(crate) fn to_float(&self) -> Option<f64> {
        match self {
            Self::Integer(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }
}
