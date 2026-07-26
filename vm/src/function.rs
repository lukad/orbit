use std::rc::Rc;

use crate::{
    id::{FunctionId, ObjectId, StateId, UpvalueId},
    native::NativeCallback,
    prototype::{PrototypeBundle, RuntimePrototypeIndex},
    value::{LightUserdata, RawValue},
};

#[derive(Debug)]
pub(crate) enum FunctionData {
    Lua(LuaClosureData),
    Native(NativeFunctionData),
}

impl FunctionData {
    pub(crate) fn lua(
        bundle: Rc<PrototypeBundle>,
        prototype: RuntimePrototypeIndex,
        upvalues: Box<[UpvalueId]>,
    ) -> Self {
        Self::Lua(LuaClosureData {
            bundle,
            prototype,
            upvalues: Rc::from(upvalues),
        })
    }

    pub(crate) fn native(
        name: impl Into<Box<str>>,
        callback: NativeCallback,
        captures: Box<[RawValue]>,
    ) -> Self {
        Self::Native(NativeFunctionData {
            name: name.into(),
            callback,
            captures,
        })
    }

    pub(crate) fn snapshot(&self, function_id: FunctionId) -> FunctionSnapshot {
        match self {
            Self::Lua(function) => FunctionSnapshot::Lua(LuaInvocation {
                function: function_id,
                bundle: Rc::clone(&function.bundle),
                prototype: function.prototype,
                upvalues: function.upvalues.clone(),
            }),
            Self::Native(function) => FunctionSnapshot::Native(NativeInvocation {
                name: function.name.clone(),
                callback: function.callback,
                captures: function.captures.clone(),
            }),
        }
    }

    pub(crate) fn visit_objects(&self, mut visit: impl FnMut(ObjectId)) {
        match self {
            Self::Lua(function) => {
                for upvalue in function.upvalues.iter() {
                    visit(upvalue.object());
                }
            }
            Self::Native(function) => {
                for capture in &function.captures {
                    if let Some(object) = capture.object_id() {
                        visit(object);
                    }
                }
            }
        }
    }

    pub(crate) fn upvalue_identity(
        &self,
        state: StateId,
        function: FunctionId,
        index: usize,
    ) -> Option<LightUserdata> {
        match self {
            Self::Lua(closure) => closure
                .upvalues
                .get(index)
                .copied()
                .map(|upvalue| LightUserdata::lua_upvalue(state, upvalue)),
            Self::Native(function_data) => {
                function_data.captures.get(index)?;
                let index = u32::try_from(index).ok()?;
                Some(LightUserdata::native_upvalue(state, function, index))
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct LuaClosureData {
    bundle: Rc<PrototypeBundle>,
    prototype: RuntimePrototypeIndex,
    upvalues: Rc<[UpvalueId]>,
}

#[derive(Debug)]
pub(crate) struct NativeFunctionData {
    name: Box<str>,
    callback: NativeCallback,
    captures: Box<[RawValue]>,
}

#[derive(Clone)]
pub(crate) enum FunctionSnapshot {
    Lua(LuaInvocation),
    Native(NativeInvocation),
}

#[derive(Clone)]
pub(crate) struct LuaInvocation {
    function: FunctionId,
    bundle: Rc<PrototypeBundle>,
    prototype: RuntimePrototypeIndex,
    upvalues: Rc<[UpvalueId]>,
}

impl LuaInvocation {
    pub(crate) fn into_parts(
        self,
    ) -> (
        FunctionId,
        Rc<PrototypeBundle>,
        RuntimePrototypeIndex,
        Rc<[UpvalueId]>,
    ) {
        (self.function, self.bundle, self.prototype, self.upvalues)
    }
}

#[derive(Clone)]
pub(crate) struct NativeInvocation {
    name: Box<str>,
    callback: NativeCallback,
    captures: Box<[RawValue]>,
}

impl NativeInvocation {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn callback(&self) -> NativeCallback {
        self.callback
    }

    pub(crate) fn captures(&self) -> &[RawValue] {
        &self.captures
    }
}
