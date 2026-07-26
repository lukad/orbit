use crate::{
    handle::{Function, Table},
    id::{FunctionId, ObjectId, StateId, TableId, UpvalueId},
    string::LuaString,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LightUserdataIdentity {
    LuaUpvalue(UpvalueId),
    NativeUpvalue { function: FunctionId, index: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightUserdata {
    state: StateId,
    identity: LightUserdataIdentity,
}

impl LightUserdata {
    pub(crate) const fn lua_upvalue(state: StateId, upvalue: UpvalueId) -> Self {
        Self {
            state,
            identity: LightUserdataIdentity::LuaUpvalue(upvalue),
        }
    }

    pub(crate) const fn native_upvalue(state: StateId, function: FunctionId, index: u32) -> Self {
        Self {
            state,
            identity: LightUserdataIdentity::NativeUpvalue { function, index },
        }
    }

    pub(crate) fn format_pointer(self) -> String {
        match self.identity {
            LightUserdataIdentity::LuaUpvalue(upvalue) => {
                let object = upvalue.object();
                format!(
                    "0x01{:016x}{:08x}{:08x}",
                    self.state.get(),
                    object.slot(),
                    object.generation()
                )
            }
            LightUserdataIdentity::NativeUpvalue { function, index } => {
                let object = function.object();
                format!(
                    "0x02{:016x}{:08x}{:08x}{:08x}",
                    self.state.get(),
                    object.slot(),
                    object.generation(),
                    index
                )
            }
        }
    }
}

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
    LightUserdata(LightUserdata),
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
            Self::LightUserdata(_) => "userdata",
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
    LightUserdata(LightUserdata),
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
            Self::LightUserdata(_) => "userdata",
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

    pub(crate) fn is_integer(&self) -> bool {
        matches!(self, Self::Integer(_))
    }

    pub(crate) fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
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
