use orbit_vm::{LocalValue, NativeAction, NativeContext, NativeToken, VmResult};

use crate::error;

#[derive(Clone, Copy)]
pub(super) struct TableCapabilities(u8);

impl TableCapabilities {
    pub(super) const READ: Self = Self(1 << 0);
    pub(super) const LENGTH: Self = Self(1 << 1);

    pub(super) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

pub(super) enum LengthDispatch<'context> {
    Immediate(i64),
    Metamethod(LocalValue<'context>),
}

/// Looks up a metamethod without invoking `__index` on the metatable.
pub(super) fn metamethod<'context>(
    context: &mut NativeContext<'context>,
    value: &LocalValue<'context>,
    name: &'static str,
) -> VmResult<Option<LocalValue<'context>>> {
    let Some(metatable) = context.get_metatable(value)? else {
        return Ok(None);
    };

    let metamethod = context.raw_get(&metatable, &context.string(name))?;
    Ok((!metamethod.is_nil()).then_some(metamethod))
}

/// Resolves Lua's length operation without invoking a metamethod.
pub(super) fn resolve_length<'context>(
    context: &mut NativeContext<'context>,
    target: &LocalValue<'context>,
) -> VmResult<LengthDispatch<'context>> {
    if let Some(string) = target.as_string() {
        let length = i64::try_from(string.len()).expect("Lua string lengths fit in i64");
        return Ok(LengthDispatch::Immediate(length));
    }

    if let Some(metamethod) = metamethod(context, target, "__len")? {
        return Ok(LengthDispatch::Metamethod(metamethod));
    }

    if target.type_name() == "table" {
        return context.raw_len(target).map(LengthDispatch::Immediate);
    }

    Err(error::failure(format!(
        "attempt to get length of a {} value",
        target.type_name()
    )))
}

/// Calls `__len` with the duplicated operand used by Lua's unary operators.
pub(super) fn call_length_metamethod<'context, C>(
    context: &NativeContext<'context>,
    metamethod: LocalValue<'context>,
    target: LocalValue<'context>,
    continuation: C,
    token: NativeToken,
) -> NativeAction
where
    C: IntoIterator<Item = LocalValue<'context>>,
{
    context.call_with_continuation(metamethod, [target.clone(), target], continuation, token)
}

/// Returns an argument if it has the desired table capabilities.
pub(super) fn required_table_like<'context>(
    context: &mut NativeContext<'context>,
    function: &'static str,
    index: usize,
    capabilities: TableCapabilities,
) -> VmResult<LocalValue<'context>> {
    let value = context
        .argument(index)
        .ok_or_else(|| error::type_error(function, index + 1, "table", None))?;

    if value.type_name() == "table" {
        return Ok(value);
    }

    let actual = value.type_name();
    let Some(metatable) = context.get_metatable(&value)? else {
        return Err(error::type_error(
            function,
            index + 1,
            "table",
            Some(actual),
        ));
    };

    for (capability, metamethod) in [
        (TableCapabilities::READ, "__index"),
        (TableCapabilities::LENGTH, "__len"),
    ] {
        if capabilities.contains(capability)
            && context
                .raw_get(&metatable, &context.string(metamethod))?
                .is_nil()
        {
            return Err(error::type_error(
                function,
                index + 1,
                "table",
                Some(actual),
            ));
        }
    }

    Ok(value)
}
