use std::collections::HashMap;

use orbit_common::Span;

use crate::{
    bytecode::{Constant, ConstantIndex, StringIndex},
    error::{CompileError, CompileErrorKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ConstantKey {
    Integer(i64),
    FloatBits(u64),
    String(StringIndex),
}

pub(crate) struct ConstantPoolBuilder {
    values: Vec<Constant>,
    indices: HashMap<ConstantKey, ConstantIndex>,
}

impl ConstantPoolBuilder {
    pub(crate) fn new() -> Self {
        Self {
            values: Vec::new(),
            indices: HashMap::new(),
        }
    }

    pub(crate) fn intern(
        &mut self,
        key: ConstantKey,
        span: Span,
    ) -> Result<ConstantIndex, CompileError> {
        if let Some(index) = self.indices.get(&key) {
            return Ok(*index);
        }

        let raw = u32::try_from(self.values.len()).map_err(|_| CompileError {
            span,
            kind: CompileErrorKind::TooManyConstants,
        })?;

        let index = ConstantIndex::new(raw);

        let value = match key {
            ConstantKey::Integer(value) => Constant::Integer(value),
            ConstantKey::FloatBits(bits) => Constant::FloatBits(bits),
            ConstantKey::String(index) => Constant::String(index),
        };

        self.values.push(value);
        self.indices.insert(key, index);

        Ok(index)
    }

    pub(crate) fn finish(self) -> Box<[Constant]> {
        self.values.into_boxed_slice()
    }
}
