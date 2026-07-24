use std::rc::Rc;

use orbit_common::SourceId;
use orbit_compiler::bytecode::{Chunk, Prototype};

use crate::{
    error::{FaultResult, VmErrorKind},
    function::{FunctionData, FunctionSnapshot},
    handle::Function,
    id::{FunctionId, UpvalueId},
    loading::{LoadError, LoadSource},
    native::NativeCallback,
    prototype::{CaptureDescriptor, PrototypeBundle, RuntimePrototypeIndex},
    upvalue::UpvalueData,
    value::RawValue,
};

use super::Runtime;

impl Runtime {
    pub(crate) fn load_chunk_raw(&mut self, chunk: Chunk) -> FaultResult<FunctionId> {
        self.reserve_chunk_source_ids(&chunk)?;
        self.instantiate_chunk(chunk, RawValue::Table(self.globals))
    }

    fn instantiate_chunk(
        &mut self,
        chunk: Chunk,
        environment: RawValue,
    ) -> FaultResult<FunctionId> {
        let bundle = Rc::new(PrototypeBundle::load(chunk)?);

        let entry = bundle.entry();
        let prototype = bundle
            .prototype(entry)
            .expect("loaded bundle always contains its entry prototype");

        let descriptors = prototype.capture_descriptors().to_vec();

        let mut upvalues = Vec::new();
        upvalues
            .try_reserve(descriptors.len())
            .map_err(|_| VmErrorKind::HeapCapacityExceeded {
                requested: descriptors.len(),
            })?;

        for (upvalue_index, descriptor) in descriptors.into_iter().enumerate() {
            match descriptor {
                CaptureDescriptor::ExternalEnvironment => {
                    let upvalue = self.allocate_upvalue(environment.clone())?;
                    upvalues.push(upvalue);
                }
                CaptureDescriptor::ParentRegister(_) | CaptureDescriptor::ParentUpvalue(_) => {
                    return Err(VmErrorKind::InvalidEntryUpvalue {
                        upvalue: upvalue_index,
                    });
                }
            }
        }

        self.allocate_lua_function(bundle, entry, upvalues.into_boxed_slice())
    }

    pub(crate) fn load_source_raw(
        &mut self,
        source: LoadSource<'_>,
        environment: Option<RawValue>,
    ) -> FaultResult<FunctionId> {
        let source_id = self.allocate_source_id()?;

        let loaded = self
            .load_service
            .compile(source_id, source)
            .map_err(|error| validate_load_error_source(error, source_id))?;

        self.validate_compiled_source(&loaded, source_id)?;

        let environment = environment.unwrap_or(RawValue::Table(self.globals));

        self.instantiate_chunk(loaded, environment)
    }

    fn allocate_source_id(&mut self) -> FaultResult<SourceId> {
        let raw = u32::try_from(self.next_source_id)
            .map_err(|_| VmErrorKind::from(LoadError::SourceIdExhausted))?;

        self.next_source_id += 1;

        let source = SourceId::new(raw);
        let inserted = self.source_ids.insert(source);
        debug_assert!(inserted, "the monotonic source identifier must be unused");

        Ok(source)
    }

    fn reserve_chunk_source_ids(&mut self, chunk: &Chunk) -> FaultResult<()> {
        let sources = chunk_source_ids(chunk);

        if let Some(source) = sources
            .iter()
            .find(|source| self.source_ids.contains(source))
        {
            return Err(LoadError::SourceIdCollision { source_id: *source }.into());
        }

        self.source_ids.try_reserve(sources.len()).map_err(|_| {
            VmErrorKind::RootCapacityExceeded {
                requested: self.source_ids.len().saturating_add(sources.len()),
            }
        })?;

        for source in sources {
            let next = u64::from(source.get()) + 1;
            self.next_source_id = self.next_source_id.max(next);
            self.source_ids.insert(source);
        }

        Ok(())
    }

    fn validate_compiled_source(&self, loaded: &Chunk, expected: SourceId) -> FaultResult<()> {
        if let Some(actual) = chunk_source_ids(loaded)
            .into_iter()
            .find(|source| *source != expected)
        {
            return Err(LoadError::UnexpectedSourceId { expected, actual }.into());
        }

        Ok(())
    }

    pub(crate) fn allocate_lua_function(
        &mut self,
        bundle: Rc<PrototypeBundle>,
        prototype: RuntimePrototypeIndex,
        upvalues: Box<[UpvalueId]>,
    ) -> FaultResult<FunctionId> {
        let runtime_prototype =
            bundle
                .prototype(prototype)
                .ok_or(VmErrorKind::InvalidChildPrototype {
                    child: u32::try_from(prototype.get()).unwrap_or(u32::MAX),
                })?;

        let expected = runtime_prototype.capture_descriptors().len();
        let actual = upvalues.len();

        if expected != actual {
            return Err(VmErrorKind::InvalidClosureUpvalueCount { expected, actual });
        }

        self.heap
            .allocate_function(FunctionData::lua(bundle, prototype, upvalues))
    }

    pub(crate) fn allocate_native_function(
        &mut self,
        name: impl Into<Box<str>>,
        callback: NativeCallback,
        captures: Box<[RawValue]>,
    ) -> FaultResult<FunctionId> {
        self.heap
            .allocate_function(FunctionData::native(name, callback, captures))
    }

    pub(crate) fn create_native_function(
        &mut self,
        name: impl Into<Box<str>>,
        callback: NativeCallback,
        captures: Box<[RawValue]>,
    ) -> FaultResult<Function> {
        let function = self.allocate_native_function(name, callback, captures)?;

        self.export_function(function)
    }

    pub(crate) fn function_snapshot(&self, function: FunctionId) -> FaultResult<FunctionSnapshot> {
        Ok(self.heap.function(function)?.snapshot())
    }

    pub(crate) fn allocate_upvalue(&mut self, value: RawValue) -> FaultResult<UpvalueId> {
        self.heap.allocate_upvalue(UpvalueData::new(value))
    }

    pub(crate) fn read_upvalue(&self, upvalue: UpvalueId) -> FaultResult<RawValue> {
        Ok(self.heap.upvalue(upvalue)?.value().clone())
    }

    pub(crate) fn write_upvalue(
        &mut self,
        upvalue: UpvalueId,
        value: RawValue,
    ) -> FaultResult<RawValue> {
        Ok(self.heap.upvalue_mut(upvalue)?.replace(value))
    }
}

fn chunk_source_ids(chunk: &Chunk) -> Vec<SourceId> {
    let mut sources = Vec::new();
    collect_prototype_source_ids(&chunk.entry, &mut sources);
    sources.sort_unstable_by_key(|source| source.get());
    sources.dedup();
    sources
}

fn collect_prototype_source_ids(prototype: &Prototype, sources: &mut Vec<SourceId>) {
    sources.push(prototype.span.source);
    sources.extend(prototype.source_map.iter().map(|entry| entry.span.source));

    for child in &prototype.children {
        collect_prototype_source_ids(child, sources);
    }
}

fn validate_load_error_source(error: LoadError, expected: SourceId) -> LoadError {
    let actual = match &error {
        LoadError::DynamicLoadingDisabled { source_id }
        | LoadError::InvalidFilenameEncoding { source_id }
        | LoadError::FileIo { source_id, .. }
        | LoadError::StdinIo { source_id, .. } => (*source_id != expected).then_some(*source_id),
        LoadError::InvalidUtf8 { span } => (span.source != expected).then_some(span.source),
        LoadError::Lex(error) => (error.span.source != expected).then_some(error.span.source),
        LoadError::Parse(error) => (error.span.source != expected).then_some(error.span.source),
        LoadError::Resolve { diagnostics } => diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.source)
            .find(|source| *source != expected),
        LoadError::Compile(error) => (error.span.source != expected).then_some(error.span.source),
        LoadError::SourceIdExhausted
        | LoadError::SourceIdCollision { .. }
        | LoadError::UnexpectedSourceId { .. } => None,
    };

    match actual {
        Some(actual) => LoadError::UnexpectedSourceId { expected, actual },
        None => error,
    }
}
