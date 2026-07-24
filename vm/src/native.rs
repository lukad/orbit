use std::marker::PhantomData;

use orbit_common::number::parse_lua_number;

use crate::{
    error::{VmError, VmResult},
    format::format_lua_float,
    loading::LoadSource,
    string::LuaString,
    value::{RawValue, Value},
};

pub type NativeCallback = for<'context> fn(&mut NativeContext<'context>) -> VmResult<NativeAction>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeToken(u64);

impl NativeToken {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeEvent {
    Start,
    Resume { token: NativeToken },
    ResumeError { token: NativeToken },
}

#[derive(Debug, Clone, Default)]
pub struct LocalValue<'context> {
    raw: RawValue,
    marker: PhantomData<&'context mut ()>,
}

impl<'context> LocalValue<'context> {
    fn new(raw: RawValue) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    pub fn type_name(&self) -> &'static str {
        self.raw.type_name()
    }

    pub fn is_nil(&self) -> bool {
        self.raw.is_nil()
    }

    pub fn is_truthy(&self) -> bool {
        self.raw.is_truthy()
    }

    pub fn as_boolean(&self) -> Option<bool> {
        self.raw.as_boolean()
    }

    pub fn as_integer(&self) -> Option<i64> {
        self.raw.as_integer()
    }

    pub fn to_integer(&self) -> Option<i64> {
        match &self.raw {
            RawValue::String(value) => parse_lua_number(value.as_bytes())?.to_integer(),
            _ => self.raw.to_integer(),
        }
    }

    pub fn is_number(&self) -> bool {
        match &self.raw {
            RawValue::Integer(_) | RawValue::Float(_) => true,
            RawValue::String(value) => parse_lua_number(value.as_bytes()).is_some(),
            _ => false,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        self.raw.as_float()
    }

    pub fn as_string(&self) -> Option<&LuaString> {
        self.raw.as_string()
    }

    pub(crate) fn raw(&self) -> &RawValue {
        &self.raw
    }

    pub(crate) fn into_raw(self) -> RawValue {
        self.raw
    }
}

#[must_use]
#[derive(Debug)]
pub struct NativeAction {
    kind: NativeActionKind,
}

impl NativeAction {
    pub(crate) fn into_kind(self) -> NativeActionKind {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Equal,
    LessThan,
    LessEqual,
}

#[derive(Debug)]
pub(crate) enum NativeActionKind {
    Return {
        values: Box<[RawValue]>,
    },
    Call {
        callee: RawValue,
        arguments: Box<[RawValue]>,
        continuation: Box<[RawValue]>,
        token: NativeToken,
    },
    Get {
        target: RawValue,
        key: RawValue,
        continuation: Box<[RawValue]>,
        token: NativeToken,
    },
    Set {
        target: RawValue,
        key: RawValue,
        value: RawValue,
        continuation: Box<[RawValue]>,
        token: NativeToken,
    },
    Yield {
        values: Box<[RawValue]>,
        token: NativeToken,
    },
    Compare {
        operation: ComparisonOp,
        left: RawValue,
        right: RawValue,
        continuation: Box<[RawValue]>,
        token: NativeToken,
    },
}

pub(crate) enum NativeEventData<'context> {
    Start,
    Resume {
        token: NativeToken,
        values: &'context [RawValue],
        continuation: &'context [RawValue],
    },
    ResumeError {
        token: NativeToken,
        error: &'context VmError,
        continuation: &'context [RawValue],
    },
}

pub(crate) trait NativeServices {
    fn import_value(&mut self, value: Value) -> VmResult<RawValue>;
    fn export_value(&mut self, value: &RawValue) -> VmResult<Value>;
    fn create_table(&mut self, array_hint: usize, hash_hint: usize) -> VmResult<RawValue>;
    fn raw_get(&mut self, table: &RawValue, key: &RawValue) -> VmResult<RawValue>;
    fn raw_set(&mut self, table: &RawValue, key: RawValue, value: RawValue) -> VmResult<()>;
    fn raw_len(&self, table: &RawValue) -> VmResult<i64>;
    fn get_metatable(&mut self, value: &RawValue) -> VmResult<Option<RawValue>>;
    fn set_metatable(&mut self, value: &RawValue, metatable: Option<&RawValue>) -> VmResult<()>;
    fn next(
        &mut self,
        table: &RawValue,
        previous: &RawValue,
    ) -> VmResult<Option<(RawValue, RawValue)>>;
    fn load_source(
        &mut self,
        source: LoadSource<'_>,
        environment: Option<RawValue>,
    ) -> VmResult<RawValue>;
    fn file_exists(&self, filename: &[u8]) -> bool;
}

pub struct NativeContext<'context> {
    services: &'context mut dyn NativeServices,
    arguments: &'context [RawValue],
    captures: &'context [RawValue],
    event: NativeEventData<'context>,
}

impl<'context> NativeContext<'context> {
    pub(crate) fn new(
        services: &'context mut dyn NativeServices,
        arguments: &'context [RawValue],
        captures: &'context [RawValue],
        event: NativeEventData<'context>,
    ) -> Self {
        Self {
            services,
            arguments,
            captures,
            event,
        }
    }

    pub fn event(&self) -> NativeEvent {
        match &self.event {
            NativeEventData::Start => NativeEvent::Start,
            NativeEventData::Resume { token, .. } => NativeEvent::Resume { token: *token },
            NativeEventData::ResumeError { token, .. } => {
                NativeEvent::ResumeError { token: *token }
            }
        }
    }

    pub fn argument_count(&self) -> usize {
        self.arguments.len()
    }

    pub fn argument(&self, index: usize) -> Option<LocalValue<'context>> {
        self.arguments.get(index).cloned().map(LocalValue::new)
    }

    pub fn capture_count(&self) -> usize {
        self.captures.len()
    }

    pub fn capture(&self, index: usize) -> Option<LocalValue<'context>> {
        self.captures.get(index).cloned().map(LocalValue::new)
    }

    pub fn resume_value_count(&self) -> usize {
        match &self.event {
            NativeEventData::Resume { values, .. } => values.len(),
            NativeEventData::Start | NativeEventData::ResumeError { .. } => 0,
        }
    }

    pub fn resume_value(&self, index: usize) -> Option<LocalValue<'context>> {
        match &self.event {
            NativeEventData::Resume { values, .. } => {
                values.get(index).cloned().map(LocalValue::new)
            }
            NativeEventData::Start | NativeEventData::ResumeError { .. } => None,
        }
    }

    pub fn resume_error(&self) -> Option<&VmError> {
        match &self.event {
            NativeEventData::ResumeError { error, .. } => Some(error),
            NativeEventData::Start | NativeEventData::Resume { .. } => None,
        }
    }

    pub fn nil(&self) -> LocalValue<'context> {
        LocalValue::new(RawValue::Nil)
    }

    pub fn boolean(&self, value: bool) -> LocalValue<'context> {
        LocalValue::new(RawValue::Boolean(value))
    }

    pub fn integer(&self, value: i64) -> LocalValue<'context> {
        LocalValue::new(RawValue::Integer(value))
    }

    pub fn float(&self, value: f64) -> LocalValue<'context> {
        LocalValue::new(RawValue::Float(value))
    }

    pub fn string(&self, bytes: impl AsRef<[u8]>) -> LocalValue<'context> {
        LocalValue::new(RawValue::String(LuaString::new(bytes)))
    }

    pub fn import(&mut self, value: Value) -> VmResult<LocalValue<'context>> {
        self.services.import_value(value).map(LocalValue::new)
    }

    pub fn export(&mut self, value: &LocalValue<'context>) -> VmResult<Value> {
        self.services.export_value(value.raw())
    }

    pub fn create_table(
        &mut self,
        array_hint: usize,
        hash_hint: usize,
    ) -> VmResult<LocalValue<'context>> {
        self.services
            .create_table(array_hint, hash_hint)
            .map(LocalValue::new)
    }

    pub fn raw_get(
        &mut self,
        table: &LocalValue<'context>,
        key: &LocalValue<'context>,
    ) -> VmResult<LocalValue<'context>> {
        self.services
            .raw_get(table.raw(), key.raw())
            .map(LocalValue::new)
    }

    pub fn raw_set(
        &mut self,
        table: &LocalValue<'context>,
        key: LocalValue<'context>,
        value: LocalValue<'context>,
    ) -> VmResult<()> {
        self.services
            .raw_set(table.raw(), key.into_raw(), value.into_raw())
    }

    pub fn raw_len(&self, table: &LocalValue<'context>) -> VmResult<i64> {
        self.services.raw_len(table.raw())
    }

    pub fn get_metatable(
        &mut self,
        value: &LocalValue<'context>,
    ) -> VmResult<Option<LocalValue<'context>>> {
        self.services
            .get_metatable(value.raw())
            .map(|metatable| metatable.map(LocalValue::new))
    }

    pub fn set_metatable(
        &mut self,
        value: &LocalValue<'context>,
        metatable: Option<&LocalValue<'context>>,
    ) -> VmResult<()> {
        self.services
            .set_metatable(value.raw(), metatable.map(LocalValue::raw))
    }

    pub fn return_values<I>(&self, values: I) -> NativeAction
    where
        I: IntoIterator<Item = LocalValue<'context>>,
    {
        NativeAction {
            kind: NativeActionKind::Return {
                values: collect_values(values),
            },
        }
    }

    pub fn continuation_value_count(&self) -> usize {
        match &self.event {
            NativeEventData::Start => 0,
            NativeEventData::Resume { continuation, .. }
            | NativeEventData::ResumeError { continuation, .. } => continuation.len(),
        }
    }

    pub fn continuation_value(&self, index: usize) -> Option<LocalValue<'context>> {
        match &self.event {
            NativeEventData::Start => None,
            NativeEventData::Resume { continuation, .. }
            | NativeEventData::ResumeError { continuation, .. } => {
                continuation.get(index).cloned().map(LocalValue::new)
            }
        }
    }

    pub fn call<I>(
        &self,
        callee: LocalValue<'context>,
        arguments: I,
        token: NativeToken,
    ) -> NativeAction
    where
        I: IntoIterator<Item = LocalValue<'context>>,
    {
        self.call_with_continuation(callee, arguments, [], token)
    }

    pub fn call_with_continuation<I, C>(
        &self,
        callee: LocalValue<'context>,
        arguments: I,
        continuation: C,
        token: NativeToken,
    ) -> NativeAction
    where
        I: IntoIterator<Item = LocalValue<'context>>,
        C: IntoIterator<Item = LocalValue<'context>>,
    {
        NativeAction {
            kind: NativeActionKind::Call {
                callee: callee.into_raw(),
                arguments: collect_values(arguments),
                continuation: collect_values(continuation),
                token,
            },
        }
    }

    /// Gets `target[key]` using Lua's `__index` semantics.
    ///
    /// The callback resumes with exactly one value. A function metamethod's
    /// first result is used, or `nil` when it returns no values.
    pub fn get(
        &self,
        target: LocalValue<'context>,
        key: LocalValue<'context>,
        token: NativeToken,
    ) -> NativeAction {
        self.get_with_continuation(target, key, [], token)
    }

    /// Gets `target[key]` using Lua's `__index` semantics and preserves values
    /// for the resumed callback.
    pub fn get_with_continuation<C>(
        &self,
        target: LocalValue<'context>,
        key: LocalValue<'context>,
        continuation: C,
        token: NativeToken,
    ) -> NativeAction
    where
        C: IntoIterator<Item = LocalValue<'context>>,
    {
        NativeAction {
            kind: NativeActionKind::Get {
                target: target.into_raw(),
                key: key.into_raw(),
                continuation: collect_values(continuation),
                token,
            },
        }
    }

    /// Sets `target[key] = value` using Lua's `__newindex` semantics.
    ///
    /// The callback resumes with no values, including when a function
    /// metamethod returns values.
    pub fn set(
        &self,
        target: LocalValue<'context>,
        key: LocalValue<'context>,
        value: LocalValue<'context>,
        token: NativeToken,
    ) -> NativeAction {
        self.set_with_continuation(target, key, value, [], token)
    }

    /// Sets `target[key] = value` using Lua's `__newindex` semantics and
    /// preserves values for the resumed callback.
    pub fn set_with_continuation<C>(
        &self,
        target: LocalValue<'context>,
        key: LocalValue<'context>,
        value: LocalValue<'context>,
        continuation: C,
        token: NativeToken,
    ) -> NativeAction
    where
        C: IntoIterator<Item = LocalValue<'context>>,
    {
        NativeAction {
            kind: NativeActionKind::Set {
                target: target.into_raw(),
                key: key.into_raw(),
                value: value.into_raw(),
                continuation: collect_values(continuation),
                token,
            },
        }
    }

    pub fn yield_values<I>(&self, values: I, token: NativeToken) -> NativeAction
    where
        I: IntoIterator<Item = LocalValue<'context>>,
    {
        NativeAction {
            kind: NativeActionKind::Yield {
                values: collect_values(values),
                token,
            },
        }
    }

    pub fn next(
        &mut self,
        table: &LocalValue<'context>,
        previous: &LocalValue<'context>,
    ) -> VmResult<Option<(LocalValue<'context>, LocalValue<'context>)>> {
        self.services
            .next(table.raw(), previous.raw())
            .map(|entry| entry.map(|(key, value)| (LocalValue::new(key), LocalValue::new(value))))
    }

    pub fn default_tostring(
        &self,
        value: &LocalValue<'context>,
        object_name: Option<&[u8]>,
    ) -> LocalValue<'context> {
        let string = match value.raw() {
            RawValue::Nil => LuaString::from("nil"),
            RawValue::Boolean(false) => LuaString::from("false"),
            RawValue::Boolean(true) => LuaString::from("true"),
            RawValue::Integer(value) => LuaString::from(value.to_string().into_bytes()),
            RawValue::Float(value) => LuaString::from(format_lua_float(*value).into_bytes()),
            RawValue::String(value) => value.clone(),
            RawValue::Table(_) | RawValue::Function(_) => {
                let object = value
                    .raw()
                    .object_id()
                    .expect("tables and functions have object identities");

                let default_name = value.type_name().as_bytes();
                let name = object_name.unwrap_or(default_name);

                let identity = format!(": 0x{:08x}{:08x}", object.slot(), object.generation(),);

                let mut bytes = Vec::with_capacity(name.len() + identity.len());

                bytes.extend_from_slice(name);
                bytes.extend_from_slice(identity.as_bytes());

                LuaString::from(bytes)
            }
        };

        LocalValue::new(RawValue::String(string))
    }

    pub fn load_source(
        &mut self,
        source: LoadSource<'_>,
        environment: Option<LocalValue<'context>>,
    ) -> VmResult<LocalValue<'context>> {
        self.services
            .load_source(source, environment.map(LocalValue::into_raw))
            .map(LocalValue::new)
    }

    pub fn file_exists(&self, filename: impl AsRef<[u8]>) -> bool {
        self.services.file_exists(filename.as_ref())
    }

    pub fn compare_with_continuation<C>(
        &self,
        operation: ComparisonOp,
        left: LocalValue<'context>,
        right: LocalValue<'context>,
        continuation: C,
        token: NativeToken,
    ) -> NativeAction
    where
        C: IntoIterator<Item = LocalValue<'context>>,
    {
        NativeAction {
            kind: NativeActionKind::Compare {
                operation,
                left: left.into_raw(),
                right: right.into_raw(),
                continuation: collect_values(continuation),
                token,
            },
        }
    }
}

fn collect_values<'context, I>(values: I) -> Box<[RawValue]>
where
    I: IntoIterator<Item = LocalValue<'context>>,
{
    values
        .into_iter()
        .map(LocalValue::into_raw)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}
