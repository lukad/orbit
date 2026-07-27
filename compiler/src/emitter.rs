use orbit_common::Span;

use crate::{
    bytecode::{ImmediateOperandSide, Instruction, RegisterRootMap, SourceMapEntry},
    error::{CompileError, CompileErrorKind},
    registers::VReg,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodeLabel(u32);

pub(crate) struct EmittedCode {
    pub(crate) instructions: Box<[Instruction]>,
    pub(crate) source_map: Box<[SourceMapEntry]>,
    pub(crate) register_root_maps: Box<[RegisterRootMap]>,
}

enum PatchField {
    Jump,
    JumpIfFalsy,
    JumpIfNotEqualSmallInt,
    ForPrepExit,
    ForLoopBody,
    TForLoopBody,
}

struct Relocation {
    instruction_pc: u32,
    target: CodeLabel,
    field: PatchField,
    span: Span,
}

pub(crate) struct Emitter {
    code: Vec<Instruction>,
    source_map: Vec<SourceMapEntry>,
    labels: Vec<Option<u32>>,
    relocations: Vec<Relocation>,
    root_maps: Vec<RegisterRootMap>,
}

impl Emitter {
    pub(crate) fn new() -> Self {
        Self {
            code: vec![],
            source_map: vec![],
            labels: vec![],
            relocations: vec![],
            root_maps: vec![],
        }
    }

    pub(crate) fn new_label(&mut self) -> CodeLabel {
        let raw = u32::try_from(self.labels.len()).expect("label count exceeded u32::MAX");
        self.labels.push(None);
        CodeLabel(raw)
    }

    pub(crate) fn bind(&mut self, label: CodeLabel) {
        let pc = u32::try_from(self.code.len()).expect("instruction count must be checked by emit");
        let slot = self
            .labels
            .get_mut(label.0 as usize)
            .expect("unknown code label");
        assert!(slot.replace(pc).is_none(), "code label bound twice");
    }

    pub(crate) fn emit(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        instruction: Instruction,
    ) -> Result<u32, CompileError> {
        let pc = u32::try_from(self.code.len()).map_err(|_| CompileError {
            span,
            kind: CompileErrorKind::TooManyInstructions,
        })?;

        if self
            .source_map
            .last()
            .is_none_or(|entry| entry.span != span)
        {
            self.source_map.push(SourceMapEntry { pc, span });
        }

        debug_assert_eq!(self.root_maps.len(), self.code.len());

        self.root_maps.push(roots);
        self.code.push(instruction);

        Ok(pc)
    }

    fn emit_relocated(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        instruction: Instruction,
        target: CodeLabel,
        field: PatchField,
    ) -> Result<(), CompileError> {
        let instruction_pc = self.emit(span, roots, instruction)?;

        self.relocations.push(Relocation {
            instruction_pc,
            target,
            field,
            span,
        });

        Ok(())
    }

    pub(crate) fn jump(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        target: CodeLabel,
    ) -> Result<(), CompileError> {
        self.emit_relocated(
            span,
            roots,
            Instruction::Jump { offset: 0 },
            target,
            PatchField::Jump,
        )
    }

    pub(crate) fn jump_if_falsy(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        condition: VReg,
        target: CodeLabel,
    ) -> Result<(), CompileError> {
        self.emit_relocated(
            span,
            roots,
            Instruction::JumpIfFalsy {
                condition: condition.to_bytecode(span)?,
                offset: 0,
            },
            target,
            PatchField::JumpIfFalsy,
        )
    }

    pub(crate) fn jump_if_not_equal_small_int(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        register: VReg,
        immediate: i16,
        side: ImmediateOperandSide,
        target: CodeLabel,
    ) -> Result<(), CompileError> {
        self.emit_relocated(
            span,
            roots,
            Instruction::JumpIfNotEqualSmallInt {
                register: register.to_bytecode(span)?,
                immediate,
                side,
                offset: 0,
            },
            target,
            PatchField::JumpIfNotEqualSmallInt,
        )
    }

    pub(crate) fn for_prep(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        base: VReg,
        exit: CodeLabel,
    ) -> Result<(), CompileError> {
        self.emit_relocated(
            span,
            roots,
            Instruction::ForPrep {
                base: base.to_bytecode(span)?,
                exit_offset: 0,
            },
            exit,
            PatchField::ForPrepExit,
        )
    }

    pub(crate) fn for_loop(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        base: VReg,
        body: CodeLabel,
    ) -> Result<(), CompileError> {
        self.emit_relocated(
            span,
            roots,
            Instruction::ForLoop {
                base: base.to_bytecode(span)?,
                body_offset: 0,
            },
            body,
            PatchField::ForLoopBody,
        )
    }

    pub(crate) fn tfor_loop(
        &mut self,
        span: Span,
        roots: RegisterRootMap,
        base: VReg,
        body: CodeLabel,
    ) -> Result<(), CompileError> {
        self.emit_relocated(
            span,
            roots,
            Instruction::TForLoop {
                base: base.to_bytecode(span)?,
                body_offset: 0,
            },
            body,
            PatchField::TForLoopBody,
        )
    }

    pub(crate) fn finish(
        self,
        terminal_roots: RegisterRootMap,
    ) -> Result<EmittedCode, CompileError> {
        let Self {
            mut code,
            source_map,
            labels,
            relocations,
            mut root_maps,
        } = self;

        for relocation in relocations {
            let target_pc =
                labels[relocation.target.0 as usize].expect("relocated code label was never bound");

            assert!(
                (target_pc as usize) < code.len(),
                "jump target does not point at an instruction"
            );

            let raw_offset = (target_pc as i64) - (relocation.instruction_pc as i64 + 1);

            let offset = i32::try_from(raw_offset).map_err(|_| CompileError {
                span: relocation.span,
                kind: CompileErrorKind::JumpTooFar,
            })?;

            let instruction = &mut code[relocation.instruction_pc as usize];

            match (relocation.field, instruction) {
                (PatchField::Jump, Instruction::Jump { offset: slot })
                | (PatchField::JumpIfFalsy, Instruction::JumpIfFalsy { offset: slot, .. })
                | (
                    PatchField::JumpIfNotEqualSmallInt,
                    Instruction::JumpIfNotEqualSmallInt { offset: slot, .. },
                )
                | (
                    PatchField::ForPrepExit,
                    Instruction::ForPrep {
                        exit_offset: slot, ..
                    },
                )
                | (
                    PatchField::ForLoopBody,
                    Instruction::ForLoop {
                        body_offset: slot, ..
                    },
                )
                | (
                    PatchField::TForLoopBody,
                    Instruction::TForLoop {
                        body_offset: slot, ..
                    },
                ) => *slot = offset,
                _ => panic!("relocation does not match emitted instruction"),
            }
        }

        assert_eq!(root_maps.len(), code.len());
        root_maps.push(terminal_roots);

        Ok(EmittedCode {
            instructions: code.into_boxed_slice(),
            source_map: source_map.into_boxed_slice(),
            register_root_maps: root_maps.into_boxed_slice(),
        })
    }
}
