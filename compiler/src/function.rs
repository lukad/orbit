use std::num::NonZeroU8;

use orbit_common::Span;
use orbit_resolver::hir::{
    BinaryOperator, Binding, BlockId, ChildFunctionId, ExitPlan, ExprId, HirExprKind, HirFunction,
    HirPlace, HirPlaceKind, HirStmtKind, HirTableField, LabelId, LocalAttribute, LocalId, LoopId,
    ScopeId, StmtId, UnaryOperator, UpvalueSource,
};

use crate::{
    bytecode::{
        BinaryOp as BytecodeBinaryOp, Count, Instruction, Prototype, PrototypeIndex, Register,
        StringIndex, UnaryOp as BytecodeUnaryOp, UpvalueDescriptor, UpvalueIndex,
    },
    constants::{ConstantKey, ConstantPoolBuilder},
    emitter::{CodeLabel, Emitter},
    error::{CompileError, CompileErrorKind},
    registers::{RegRange, RegisterStack, VReg},
};

struct ParentLayout<'a> {
    local_registers: &'a [Option<Register>],
    upvalue_indices: &'a [UpvalueIndex],
}

enum ReachabilityStmt {
    Straight,
    Block(BlockId),
    If {
        branches: Vec<BlockId>,
        else_block: Option<BlockId>,
    },
    While {
        loop_id: LoopId,
        body: BlockId,
    },
    Repeat {
        loop_id: LoopId,
        body: BlockId,
    },
    For {
        loop_id: LoopId,
        body: BlockId,
    },
    Return,
    Break(LoopId),
    Goto(LabelId),
}

/// Builds the structured statement CFG before bytecode emission. This lets a
/// forward label restore reachability after a terminator without forcing dead
/// declarations and child closures to be emitted. Synthetic nodes model the
/// post-body condition point of `repeat`, whose first iteration cannot be
/// skipped.
struct ReachabilityBuilder<'hir> {
    function: &'hir HirFunction,
    statement_count: usize,
    successors: Vec<Vec<usize>>,
    built: Vec<bool>,
    label_nodes: Vec<Option<usize>>,
    loop_exits: Vec<Option<Vec<usize>>>,
}

impl<'hir> ReachabilityBuilder<'hir> {
    fn new(function: &'hir HirFunction) -> Self {
        let statement_count = function.statements.len();
        let mut label_nodes = vec![None; function.labels.len()];

        for (statement, hir_statement) in function.statements.iter() {
            if let HirStmtKind::Label { label } = hir_statement.kind {
                let slot = &mut label_nodes[label.0 as usize];
                assert!(
                    slot.replace(statement.0 as usize).is_none(),
                    "HIR label has more than one statement"
                );
            }
        }

        assert!(
            label_nodes.iter().all(Option::is_some),
            "HIR label has no statement"
        );

        Self {
            function,
            statement_count,
            successors: vec![vec![]; statement_count],
            built: vec![false; statement_count],
            label_nodes,
            loop_exits: vec![None; function.loop_count],
        }
    }

    fn finish(mut self) -> Vec<bool> {
        let entry = self.build_block(self.function.body, &[]);
        let mut reachable = vec![false; self.successors.len()];
        let mut pending = entry;

        while let Some(node) = pending.pop() {
            if std::mem::replace(&mut reachable[node], true) {
                continue;
            }

            pending.extend(self.successors[node].iter().copied());
        }

        reachable.truncate(self.statement_count);
        reachable
    }

    fn new_node(&mut self) -> usize {
        let node = self.successors.len();
        self.successors.push(vec![]);
        self.built.push(false);
        node
    }

    fn set_successors(&mut self, node: usize, mut successors: Vec<usize>) {
        assert!(!self.built[node], "reachability node was built twice");

        let mut unique = Vec::with_capacity(successors.len());
        for successor in successors.drain(..) {
            if !unique.contains(&successor) {
                unique.push(successor);
            }
        }

        self.successors[node] = unique;
        self.built[node] = true;
    }

    fn build_block(&mut self, block: BlockId, continuation: &[usize]) -> Vec<usize> {
        let statements = self.function.blocks[block].statements.clone();
        let mut next = continuation.to_vec();

        for statement in statements.into_iter().rev() {
            self.build_statement(statement, &next);
            next.clear();
            next.push(statement.0 as usize);
        }

        next
    }

    fn snapshot_statement(&self, statement: StmtId) -> ReachabilityStmt {
        match &self.function.statements[statement].kind {
            HirStmtKind::Block(block) => ReachabilityStmt::Block(*block),
            HirStmtKind::If {
                branches,
                else_block,
            } => ReachabilityStmt::If {
                branches: branches.iter().map(|branch| branch.body).collect(),
                else_block: *else_block,
            },
            HirStmtKind::While { loop_id, body, .. } => ReachabilityStmt::While {
                loop_id: *loop_id,
                body: *body,
            },
            HirStmtKind::Repeat { loop_id, body, .. } => ReachabilityStmt::Repeat {
                loop_id: *loop_id,
                body: *body,
            },
            HirStmtKind::NumericFor { loop_id, body, .. }
            | HirStmtKind::GenericFor { loop_id, body, .. } => ReachabilityStmt::For {
                loop_id: *loop_id,
                body: *body,
            },
            HirStmtKind::Return { .. } => ReachabilityStmt::Return,
            HirStmtKind::Break { target, .. } => ReachabilityStmt::Break(*target),
            HirStmtKind::Goto { target, .. } => ReachabilityStmt::Goto(*target),
            HirStmtKind::Local { .. }
            | HirStmtKind::Assign { .. }
            | HirStmtKind::Call { .. }
            | HirStmtKind::Label { .. } => ReachabilityStmt::Straight,
        }
    }

    fn build_statement(&mut self, statement: StmtId, next: &[usize]) {
        let node = statement.0 as usize;
        let successors = match self.snapshot_statement(statement) {
            ReachabilityStmt::Straight => next.to_vec(),
            ReachabilityStmt::Block(block) => self.build_block(block, next),
            ReachabilityStmt::If {
                branches,
                else_block,
            } => {
                let mut successors = Vec::new();

                for branch in branches {
                    successors.extend(self.build_block(branch, next));
                }

                match else_block {
                    Some(block) => successors.extend(self.build_block(block, next)),
                    None => successors.extend_from_slice(next),
                }

                successors
            }
            ReachabilityStmt::While { loop_id, body } | ReachabilityStmt::For { loop_id, body } => {
                let slot = &mut self.loop_exits[loop_id.0 as usize];
                assert!(
                    slot.replace(next.to_vec()).is_none(),
                    "loop exit was built twice"
                );

                let mut successors = self.build_block(body, &[node]);
                successors.extend_from_slice(next);
                successors
            }
            ReachabilityStmt::Repeat { loop_id, body } => {
                let slot = &mut self.loop_exits[loop_id.0 as usize];
                assert!(
                    slot.replace(next.to_vec()).is_none(),
                    "loop exit was built twice"
                );

                let condition = self.new_node();
                let body_entry = self.build_block(body, &[condition]);
                let mut condition_successors = body_entry.clone();
                condition_successors.extend_from_slice(next);
                self.set_successors(condition, condition_successors);
                body_entry
            }
            ReachabilityStmt::Return => vec![],
            ReachabilityStmt::Break(target) => self.loop_exits[target.0 as usize]
                .clone()
                .expect("break target exit was not built"),
            ReachabilityStmt::Goto(target) => vec![
                self.label_nodes[target.0 as usize].expect("goto target has no label statement"),
            ],
        };

        self.set_successors(node, successors);
    }
}

fn statement_reachability(function: &HirFunction) -> Vec<bool> {
    ReachabilityBuilder::new(function).finish()
}

#[derive(Debug)]
enum SingleExpr {
    Nil,
    Bool(bool),
    Integer(i64),
    FloatBits(u64),
    String(StringIndex),
    Read(Binding),
    Unary {
        operator: UnaryOperator,
        operand: ExprId,
    },
    Binary {
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
    },
    Index {
        table: ExprId,
        key: ExprId,
    },
    Closure(ChildFunctionId),
    Table(Vec<TableFieldSnapshot>),
}

#[derive(Debug, Clone, Copy)]
enum TableFieldSnapshot {
    List {
        span: Span,
        value: ExprId,
    },
    Record {
        span: Span,
        name: StringIndex,
        value: ExprId,
    },
    Computed {
        span: Span,
        key: ExprId,
        value: ExprId,
    },
}

#[derive(Debug, Clone, Copy)]
struct ConditionalBranchSnapshot {
    span: Span,
    condition: ExprId,
    body: BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResultContext {
    Discard,
    // Only an already allocated one-result context accepts an arbitrary
    // destination. Fixed and open results always append at the register top.
    One(VReg),
    Fixed(NonZeroU8),
    Open,
}

const ONE_FIXED_RESULT: NonZeroU8 = NonZeroU8::MIN;

#[derive(Debug, Clone, Copy)]
enum AssignmentTargetSnapshot {
    Local {
        span: Span,
        register: VReg,
    },
    Upvalue {
        span: Span,
        upvalue: UpvalueIndex,
    },
    Index {
        span: Span,
        table: ExprId,
        key: ExprId,
    },
}

#[derive(Debug, Clone, Copy)]
enum PreparedAssignmentTarget {
    Local { span: Span, register: VReg },
    Upvalue { span: Span, upvalue: UpvalueIndex },
    Index { span: Span, table: VReg, key: VReg },
}

#[derive(Debug)]
enum ResultExpr {
    Single(SingleExpr),
    Vararg,
    Call {
        callee: ExprId,
        arguments: Vec<ExprId>,
    },
    MethodCall {
        receiver: ExprId,
        method: StringIndex,
        arguments: Vec<ExprId>,
    },
    AdjustToOne(ExprId),
}

#[derive(Debug, Clone, Copy)]
enum ListKind {
    Arguments,
    Results,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalState {
    Unseen,
    Reserved,
    Active,
    Dead,
}

struct LocalSlot {
    register: Option<VReg>,
    state: LocalState,
}

#[derive(Debug, Clone, Copy)]
struct ActiveScope {
    id: ScopeId,
    active_local_base: usize,
    register_base: VReg,
}

#[derive(Debug, Clone, Copy)]
struct LoopState {
    break_label: CodeLabel,
    break_active_local_count: usize,
    body_scope: ScopeId,
    generic_closer: Option<VReg>,
    break_is_targeted: bool,
}

struct FunctionCompiler<'hir> {
    function: &'hir HirFunction,
    emitter: Emitter,
    constants: ConstantPoolBuilder,
    registers: RegisterStack,
    locals: Vec<LocalSlot>,
    active_locals: Vec<LocalId>,
    scopes: Vec<ActiveScope>,
    hir_labels: Vec<CodeLabel>,
    reachable_statements: Vec<bool>,
    loop_targets: Vec<Option<LoopState>>,
    active_loops: Vec<LoopId>,
    upvalues: Vec<UpvalueDescriptor>,
    upvalue_indices: Vec<UpvalueIndex>,
    children: Vec<Prototype>,
    child_indices: Vec<Option<PrototypeIndex>>,
}

impl<'hir> FunctionCompiler<'hir> {
    fn new(
        function: &'hir HirFunction,
        parent: Option<&ParentLayout<'_>>,
    ) -> Result<Self, CompileError> {
        let mut emitter = Emitter::new();

        let hir_labels = function
            .labels
            .iter()
            .map(|_| emitter.new_label())
            .collect();
        let reachable_statements = statement_reachability(function);

        let locals = function
            .locals
            .iter()
            .map(|_| LocalSlot {
                register: None,
                state: LocalState::Unseen,
            })
            .collect();

        let loop_targets = (0..function.loop_count).map(|_| None).collect();

        let upvalue_indices = function
            .upvalues
            .iter()
            .map(|(id, _)| UpvalueIndex::new(id.0))
            .collect::<Vec<_>>();

        let mut upvalues = Vec::with_capacity(function.upvalues.len());

        for (_, upvalue) in function.upvalues.iter() {
            let descriptor = match upvalue.source {
                UpvalueSource::ExternalEnvironment => {
                    assert!(
                        parent.is_none(),
                        "only the entry prototype can capture an external environment"
                    );
                    UpvalueDescriptor::ExternalEnvironment
                }
                UpvalueSource::ParentLocal(local) => {
                    let parent = parent.expect("nested function needs a parent layout");
                    let register = parent.local_registers[local.0 as usize]
                        .expect("captured parent local has no assigned register");
                    UpvalueDescriptor::ParentRegister(register)
                }
                UpvalueSource::ParentUpvalue(upvalue) => {
                    let parent = parent.expect("nested function needs a parent layout");
                    UpvalueDescriptor::ParentUpvalue(parent.upvalue_indices[upvalue.0 as usize])
                }
            };

            upvalues.push(descriptor);
        }

        Ok(Self {
            function,
            emitter,
            constants: ConstantPoolBuilder::new(),
            registers: RegisterStack::new(),
            locals,
            active_locals: vec![],
            scopes: vec![],
            hir_labels,
            reachable_statements,
            loop_targets,
            active_loops: vec![],
            upvalues,
            upvalue_indices,
            children: vec![],
            child_indices: vec![None; function.children.len()],
        })
    }

    fn compile_child(
        &mut self,
        child: ChildFunctionId,
        span: Span,
    ) -> Result<PrototypeIndex, CompileError> {
        let hir_index = child.0 as usize;
        let cached = *self
            .child_indices
            .get(hir_index)
            .expect("closure references a missing child function");

        if let Some(index) = cached {
            return Ok(index);
        }

        let index = u32::try_from(self.children.len())
            .map(PrototypeIndex::new)
            .map_err(|_| CompileError {
                span,
                kind: CompileErrorKind::TooManyChildren,
            })?;

        let local_registers = self
            .locals
            .iter()
            .map(|slot| {
                slot.register
                    .map(|register| register.to_bytecode(span))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;

        let prototype = {
            let parent = ParentLayout {
                local_registers: &local_registers,
                upvalue_indices: &self.upvalue_indices,
            };
            let child = self
                .function
                .children
                .get(hir_index)
                .expect("closure references a missing child function");

            compile_function(child, Some(&parent))?
        };

        self.children.push(prototype);
        self.child_indices[hir_index] = Some(index);

        Ok(index)
    }

    fn enter_scope(&mut self, scope: ScopeId) {
        let expected_parent = self.scopes.last().map(|scope| scope.id);
        let actual_parent = self.function.scopes[scope].parent;

        assert_eq!(
            actual_parent, expected_parent,
            "entered scope does not descend from the active scope"
        );
        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "scope entry requires an empty temporary stack"
        );

        self.scopes.push(ActiveScope {
            id: scope,
            active_local_base: self.active_locals.len(),
            register_base: self.registers.floor(),
        });
    }

    fn enter_root_scope(&mut self) {
        let scope = self.function.blocks[self.function.body].scope;
        self.enter_scope(scope);
    }

    fn scope_requires_close(&self, scope: ScopeId) -> bool {
        let scope = &self.function.scopes[scope];
        scope.has_captured_locals || scope.has_to_be_closed_locals
    }

    fn local_requires_close(&self, local: LocalId) -> bool {
        let local = &self.function.locals[local];
        local.captured || local.attribute == Some(LocalAttribute::Close)
    }

    fn active_scope_requires_close(&self, scope: ActiveScope) -> bool {
        self.active_locals[scope.active_local_base..]
            .iter()
            .copied()
            .any(|local| self.local_requires_close(local))
    }

    fn exit_close_base(&self, exit: &ExitPlan) -> Option<VReg> {
        assert!(
            exit.scopes.len() <= self.scopes.len(),
            "exit plan leaves more scopes than are active"
        );

        let exited_start = self.scopes.len() - exit.scopes.len();
        let active_exited = &self.scopes[exited_start..];
        let mut close_base = None;

        for (&planned, active) in exit.scopes.iter().zip(active_exited.iter().rev()) {
            assert_eq!(
                planned, active.id,
                "exit plan is not the active scope suffix"
            );

            if self.scope_requires_close(planned) {
                close_base = Some(active.register_base);
            }
        }

        close_base
    }

    fn return_close_from(
        &self,
        exit: &ExitPlan,
        span: Span,
    ) -> Result<Option<Register>, CompileError> {
        assert_eq!(
            exit.scopes.len(),
            self.scopes.len(),
            "return exit plan must leave every active scope"
        );

        let mut close_from = self.exit_close_base(exit);

        for &loop_id in &self.active_loops {
            let loop_state =
                self.loop_targets[loop_id.0 as usize].expect("active loop has no target state");

            if let Some(generic_closer) = loop_state.generic_closer {
                close_from = Some(match close_from {
                    Some(base) => base.min(generic_closer),
                    None => generic_closer,
                });
            }
        }

        close_from.map(|base| base.to_bytecode(span)).transpose()
    }

    fn leave_scope(&mut self, span: Span, emit_runtime_close: bool) -> Result<(), CompileError> {
        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "scope exit requires an empty temporary stack"
        );

        let scope = *self.scopes.last().expect("scope stack must not be empty");

        if emit_runtime_close && self.active_scope_requires_close(scope) {
            self.emitter.emit(
                span,
                Instruction::CloseFrom {
                    base: scope.register_base.to_bytecode(span)?,
                },
            )?;
        }

        self.scopes.pop();

        for index in scope.active_local_base..self.active_locals.len() {
            let local = self.active_locals[index];
            let slot = &mut self.locals[local.0 as usize];

            assert_eq!(slot.state, LocalState::Active);
            slot.state = LocalState::Dead;
        }

        self.active_locals.truncate(scope.active_local_base);
        self.registers.release_pinned_to(scope.register_base);

        Ok(())
    }

    fn local_suffix_close_base(&self, retained: &[LocalId]) -> Option<VReg> {
        assert!(
            self.active_locals.starts_with(retained),
            "label locals are not a prefix of the active locals"
        );

        self.active_locals[retained.len()..]
            .iter()
            .copied()
            .filter(|&local| self.local_requires_close(local))
            .map(|local| {
                let slot = &self.locals[local.0 as usize];

                assert_eq!(slot.state, LocalState::Active);
                slot.register
                    .expect("active local requiring cleanup has no register")
            })
            .min()
    }

    fn crossed_generic_close_base(&self, exit: &ExitPlan) -> Option<VReg> {
        self.active_loops
            .iter()
            .copied()
            .filter_map(|loop_id| {
                let state =
                    self.loop_targets[loop_id.0 as usize].expect("active loop has no target state");

                state
                    .generic_closer
                    .filter(|_| exit.scopes.contains(&state.body_scope))
            })
            .min()
    }

    fn emit_goto(
        &mut self,
        target: LabelId,
        exit: &ExitPlan,
        span: Span,
    ) -> Result<(), CompileError> {
        assert!(
            exit.scopes.len() < self.scopes.len(),
            "goto exits the target label's scope"
        );

        let target_scope = self.scopes[self.scopes.len() - exit.scopes.len() - 1];
        let label = &self.function.labels[target];
        let retained = label.active_locals.clone();

        assert_eq!(
            target_scope.id, label.scope,
            "goto exit plan does not stop at the target label's scope"
        );
        assert!(
            self.active_locals.starts_with(&retained),
            "goto enters the scope of an inactive local"
        );

        let mut close_from = self.exit_close_base(exit);

        for candidate in [
            self.local_suffix_close_base(&retained),
            self.crossed_generic_close_base(exit),
        ]
        .into_iter()
        .flatten()
        {
            close_from = Some(match close_from {
                Some(base) => base.min(candidate),
                None => candidate,
            });
        }

        if let Some(base) = close_from {
            self.emitter.emit(
                span,
                Instruction::CloseFrom {
                    base: base.to_bytecode(span)?,
                },
            )?;
        }

        self.emitter.jump(span, self.hir_labels[target.0 as usize])
    }

    fn emit_label(
        &mut self,
        label: LabelId,
        falls_through: bool,
        span: Span,
    ) -> Result<bool, CompileError> {
        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "label reached with live temporaries"
        );

        let hir_label = &self.function.labels[label];
        let retained = hir_label.active_locals.clone();
        let current_scope = self
            .scopes
            .last()
            .expect("label requires an active scope")
            .id;

        assert_eq!(hir_label.scope, current_scope);
        assert!(
            self.active_locals.starts_with(&retained),
            "label locals are not a prefix of the active locals"
        );

        // A natural fallthrough edge must close the locals that cease to exist
        // at a trailing label. Gotos do the same cleanup at their source, so
        // binding after this instruction gives every incoming edge one close.
        if falls_through && let Some(base) = self.local_suffix_close_base(&retained) {
            self.emitter.emit(
                span,
                Instruction::CloseFrom {
                    base: base.to_bytecode(span)?,
                },
            )?;
        }

        if retained.len() < self.active_locals.len() {
            let first_dropped = self.active_locals[retained.len()];
            let release_base = self.locals[first_dropped.0 as usize]
                .register
                .expect("active local dropped at a label has no register");

            for &local in &self.active_locals[retained.len()..] {
                let slot = &mut self.locals[local.0 as usize];

                assert_eq!(slot.state, LocalState::Active);
                slot.state = LocalState::Dead;
            }

            self.active_locals.truncate(retained.len());
            self.registers.release_pinned_to(release_base);
        }

        self.emitter.bind(self.hir_labels[label.0 as usize]);
        Ok(true)
    }

    fn emit_scoped_block(&mut self, block: BlockId) -> Result<bool, CompileError> {
        let span = self.function.blocks[block].span;
        let scope = self.function.blocks[block].scope;

        self.enter_scope(scope);
        let falls_through = self.emit_block_stmts(block)?;
        self.leave_scope(span, falls_through)?;

        Ok(falls_through)
    }

    fn emit_if(
        &mut self,
        branches: &[ConditionalBranchSnapshot],
        else_block: Option<BlockId>,
    ) -> Result<bool, CompileError> {
        assert!(
            !branches.is_empty(),
            "if statement has no conditional branches"
        );

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "if statement began with live temporaries"
        );

        let mut end_label = None;
        let mut any_branch_falls_through = false;

        for (index, branch) in branches.iter().copied().enumerate() {
            let next = self.emitter.new_label();
            let condition_span = self.function.expressions[branch.condition].span;
            let mark = self.registers.temporary_mark();

            let condition = self.registers.reserve_temporaries(1, condition_span)?.base;

            self.emit_one(branch.condition, condition)?;
            self.emitter
                .jump_if_falsy(condition_span, condition, next)?;
            self.registers.release_temporaries_to(mark);

            let body_falls_through = self.emit_scoped_block(branch.body)?;
            any_branch_falls_through |= body_falls_through;

            let has_another_alternative = index + 1 < branches.len() || else_block.is_some();

            if body_falls_through && has_another_alternative {
                let end = match end_label {
                    Some(end) => end,
                    None => {
                        let end = self.emitter.new_label();
                        end_label = Some(end);
                        end
                    }
                };

                self.emitter.jump(branch.span, end)?;
            }

            self.emitter.bind(next);
        }

        let fallback_falls_through = match else_block {
            Some(block) => self.emit_scoped_block(block)?,
            None => true,
        };

        if let Some(end) = end_label {
            self.emitter.bind(end);
        }

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "if statement leaked temporary registers"
        );

        Ok(any_branch_falls_through || fallback_falls_through)
    }

    fn activate_loop(
        &mut self,
        loop_id: LoopId,
        break_label: CodeLabel,
        body_scope: ScopeId,
        generic_closer: Option<VReg>,
    ) {
        let state = LoopState {
            break_label,
            break_active_local_count: self.active_locals.len(),
            body_scope,
            generic_closer,
            break_is_targeted: false,
        };
        let slot = &mut self.loop_targets[loop_id.0 as usize];

        assert!(slot.replace(state).is_none(), "loop activated twice");
        self.active_loops.push(loop_id);
    }

    fn deactivate_loop(&mut self, loop_id: LoopId) -> LoopState {
        assert_eq!(
            self.active_loops.pop(),
            Some(loop_id),
            "loops were deactivated out of nesting order"
        );
        self.loop_targets[loop_id.0 as usize]
            .take()
            .expect("inactive loop was deactivated")
    }

    fn emit_break(
        &mut self,
        target: LoopId,
        exit: &ExitPlan,
        span: Span,
    ) -> Result<(), CompileError> {
        assert_eq!(
            self.active_loops.last().copied(),
            Some(target),
            "break does not target the innermost active loop"
        );

        let loop_state =
            self.loop_targets[target.0 as usize].expect("break targets an inactive loop");
        let mut close_from = self.exit_close_base(exit);

        assert!(!exit.scopes.is_empty(), "break exits no scopes");

        let outermost_exited = self.scopes.len() - exit.scopes.len();
        let body_scope = self.scopes[outermost_exited];

        assert_eq!(body_scope.id, loop_state.body_scope);
        assert_eq!(
            body_scope.active_local_base, loop_state.break_active_local_count,
            "break does not restore the loop's parent local count"
        );

        if let Some(generic_closer) = loop_state.generic_closer {
            close_from = Some(match close_from {
                Some(base) => base.min(generic_closer),
                None => generic_closer,
            });
        }

        if let Some(base) = close_from {
            self.emitter.emit(
                span,
                Instruction::CloseFrom {
                    base: base.to_bytecode(span)?,
                },
            )?;
        }

        self.emitter.jump(span, loop_state.break_label)?;

        self.loop_targets[target.0 as usize]
            .as_mut()
            .expect("break target became inactive")
            .break_is_targeted = true;

        Ok(())
    }

    fn emit_while(
        &mut self,
        loop_id: LoopId,
        condition: ExprId,
        body: BlockId,
        span: Span,
    ) -> Result<bool, CompileError> {
        let body_scope = self.function.blocks[body].scope;

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "while loop began with live temporaries"
        );

        let condition_label = self.emitter.new_label();
        let break_label = self.emitter.new_label();

        self.emitter.bind(condition_label);

        let condition_span = self.function.expressions[condition].span;
        let mark = self.registers.temporary_mark();
        let condition_register = self.registers.reserve_temporaries(1, condition_span)?.base;

        self.emit_one(condition, condition_register)?;
        self.emitter
            .jump_if_falsy(condition_span, condition_register, break_label)?;
        self.registers.release_temporaries_to(mark);

        self.activate_loop(loop_id, break_label, body_scope, None);
        let body_falls_through = self.emit_scoped_block(body)?;
        self.deactivate_loop(loop_id);

        if body_falls_through {
            self.emitter.jump(span, condition_label)?;
        }

        self.emitter.bind(break_label);

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "while loop leaked temporary registers"
        );

        Ok(true)
    }

    fn emit_repeat(
        &mut self,
        loop_id: LoopId,
        body: BlockId,
        condition: ExprId,
        span: Span,
    ) -> Result<bool, CompileError> {
        let body_scope = self.function.blocks[body].scope;
        let body_span = self.function.blocks[body].span;

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "repeat loop began with live temporaries"
        );

        let body_label = self.emitter.new_label();
        let break_label = self.emitter.new_label();

        self.emitter.bind(body_label);
        self.activate_loop(loop_id, break_label, body_scope, None);
        self.enter_scope(body_scope);

        let body_falls_through = self.emit_block_stmts(body)?;

        if body_falls_through {
            let condition_span = self.function.expressions[condition].span;
            let mark = self.registers.temporary_mark();
            let condition_register = self.registers.reserve_temporaries(1, condition_span)?.base;

            self.emit_one(condition, condition_register)?;

            let scope = *self
                .scopes
                .last()
                .expect("repeat body scope must still be active");

            assert_eq!(scope.id, body_scope);

            if self.scope_requires_close(body_scope) {
                let continue_label = self.emitter.new_label();

                self.emitter
                    .jump_if_falsy(condition_span, condition_register, continue_label)?;
                self.registers.release_temporaries_to(mark);

                let close_base = scope.register_base.to_bytecode(span)?;

                self.emitter
                    .emit(span, Instruction::CloseFrom { base: close_base })?;
                self.emitter.jump(span, break_label)?;

                self.emitter.bind(continue_label);
                self.emitter
                    .emit(span, Instruction::CloseFrom { base: close_base })?;
                self.emitter.jump(span, body_label)?;
            } else {
                self.emitter
                    .jump_if_falsy(condition_span, condition_register, body_label)?;
                self.registers.release_temporaries_to(mark);
            }
        }

        self.leave_scope(body_span, false)?;
        let loop_state = self.deactivate_loop(loop_id);
        self.emitter.bind(break_label);

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "repeat loop leaked temporary registers"
        );

        Ok(body_falls_through || loop_state.break_is_targeted)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_numeric_for(
        &mut self,
        loop_id: LoopId,
        variable: LocalId,
        initial: ExprId,
        limit: ExprId,
        step: Option<ExprId>,
        body: BlockId,
        span: Span,
    ) -> Result<bool, CompileError> {
        let body_scope = self.function.blocks[body].scope;
        let body_span = self.function.blocks[body].span;
        let hir_variable = &self.function.locals[variable];
        let variable_span = hir_variable.span;

        assert_eq!(hir_variable.attribute, None);
        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "numeric for began with live temporaries"
        );

        let [initial_register, limit_register, step_register] =
            self.registers.reserve_temporary_array(span)?;
        let control_base = initial_register;

        self.emit_one(initial, initial_register)?;
        self.emit_one(limit, limit_register)?;

        match step {
            Some(step) => self.emit_one(step, step_register)?,
            None => {
                self.emitter.emit(
                    span,
                    Instruction::LoadSmallInt {
                        dst: step_register.to_bytecode(span)?,
                        value: 1,
                    },
                )?;
            }
        }

        let controls = RegRange {
            base: initial_register,
            len: 3,
        };
        self.registers.promote_temporaries_to_pinned(controls);

        let body_label = self.emitter.new_label();
        let exit_label = self.emitter.new_label();

        self.activate_loop(loop_id, exit_label, body_scope, None);
        self.enter_scope(body_scope);

        let variable_register = self.registers.reserve_pinned(1, variable_span)?.base;

        let variable_slot = &mut self.locals[variable.0 as usize];

        assert_eq!(variable_slot.state, LocalState::Unseen);
        assert!(variable_slot.register.is_none());

        variable_slot.register = Some(variable_register);
        variable_slot.state = LocalState::Active;
        self.active_locals.push(variable);

        self.emitter.for_prep(span, control_base, exit_label)?;
        self.emitter.bind(body_label);

        let body_falls_through = self.emit_block_stmts(body)?;

        self.leave_scope(body_span, body_falls_through)?;

        if body_falls_through {
            self.emitter.for_loop(span, control_base, body_label)?;
        }

        let _ = self.deactivate_loop(loop_id);
        self.emitter.bind(exit_label);
        self.registers.release_pinned_to(control_base);

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "numeric for leaked registers"
        );

        Ok(true)
    }

    fn emit_generic_for(
        &mut self,
        loop_id: LoopId,
        variables: &[LocalId],
        expressions: &[ExprId],
        body: BlockId,
        span: Span,
    ) -> Result<bool, CompileError> {
        let variable_count = checked_list_count(variables.len(), span, ListKind::Results)?;
        let body_scope = self.function.blocks[body].scope;
        let body_span = self.function.blocks[body].span;

        assert!(
            !variables.is_empty(),
            "generic for has no visible variables"
        );
        assert!(
            !expressions.is_empty(),
            "generic for has no control expressions"
        );
        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "generic for began with live temporaries"
        );

        let controls = self.emit_exact_expr_list(expressions, 4, span)?;
        let control_base = controls.base;

        self.registers.promote_temporaries_to_pinned(controls);

        let closer = control_base.offset(3);
        self.emitter.emit(
            span,
            Instruction::MarkToClose {
                register: closer.to_bytecode(span)?,
            },
        )?;

        let body_label = self.emitter.new_label();
        let call_label = self.emitter.new_label();
        let exit_label = self.emitter.new_label();

        self.activate_loop(loop_id, exit_label, body_scope, Some(closer));
        self.enter_scope(body_scope);

        let visible = self
            .registers
            .reserve_pinned(u16::from(variable_count), span)?;

        for (&variable, register) in variables.iter().zip(visible.iter()) {
            let hir_variable = &self.function.locals[variable];

            assert_eq!(hir_variable.attribute, None);

            let slot = &mut self.locals[variable.0 as usize];

            assert_eq!(slot.state, LocalState::Unseen);
            assert!(slot.register.is_none());

            slot.register = Some(register);
            slot.state = LocalState::Active;
            self.active_locals.push(variable);
        }

        self.emitter.jump(span, call_label)?;
        self.emitter.bind(body_label);

        let body_falls_through = self.emit_block_stmts(body)?;

        self.leave_scope(body_span, body_falls_through)?;

        self.emitter.bind(call_label);
        self.emitter.emit(
            span,
            Instruction::TForCall {
                base: control_base.to_bytecode(span)?,
                variables: variable_count,
            },
        )?;
        self.emitter.tfor_loop(span, control_base, body_label)?;
        self.emitter.emit(
            span,
            Instruction::CloseFrom {
                base: closer.to_bytecode(span)?,
            },
        )?;

        let _ = self.deactivate_loop(loop_id);
        self.emitter.bind(exit_label);
        self.registers.release_pinned_to(control_base);

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "generic for leaked registers"
        );

        Ok(true)
    }

    fn initialize_parameters(&mut self) -> Result<u8, CompileError> {
        let count = u8::try_from(self.function.parameters.len()).map_err(|_| CompileError {
            span: self.function.span,
            kind: CompileErrorKind::TooManyParameters,
        })?;

        let range = self
            .registers
            .reserve_pinned(count.into(), self.function.span)?;

        for (local, register) in self.function.parameters.iter().copied().zip(range.iter()) {
            let slot = &mut self.locals[local.0 as usize];

            assert_eq!(slot.state, LocalState::Unseen);

            slot.register = Some(register);
            slot.state = LocalState::Active;
            self.active_locals.push(local);
        }

        Ok(count)
    }

    fn emit_block_stmts(&mut self, block: BlockId) -> Result<bool, CompileError> {
        let statements = self.function.blocks[block].statements.clone();
        let mut falls_through = true;

        for statement in statements {
            if !self.reachable_statements[statement.0 as usize] {
                continue;
            }

            let span = self.function.statements[statement].span;
            let label = match self.function.statements[statement].kind {
                HirStmtKind::Label { label } => Some(label),
                _ => None,
            };

            if let Some(label) = label {
                falls_through = self.emit_label(label, falls_through, span)?;
                continue;
            }

            assert!(
                falls_through,
                "reachable non-label statement has no fallthrough predecessor"
            );
            falls_through = self.emit_stmt(statement)?;
        }

        Ok(falls_through)
    }

    fn emit_stmt(&mut self, stmt: StmtId) -> Result<bool, CompileError> {
        let stmt = &self.function.statements[stmt];
        let span = stmt.span;

        match &stmt.kind {
            HirStmtKind::Block(block) => self.emit_scoped_block(*block),
            HirStmtKind::While {
                loop_id,
                condition,
                body,
            } => self.emit_while(*loop_id, *condition, *body, span),
            HirStmtKind::Repeat {
                loop_id,
                condition,
                body,
            } => self.emit_repeat(*loop_id, *body, *condition, span),
            HirStmtKind::NumericFor {
                loop_id,
                variable,
                initial,
                limit,
                step,
                body,
            } => self.emit_numeric_for(*loop_id, *variable, *initial, *limit, *step, *body, span),
            HirStmtKind::GenericFor {
                loop_id,
                variables,
                expressions,
                body,
            } => {
                let loop_id = *loop_id;
                let variables = variables.clone();
                let expressions = expressions.clone();
                let body = *body;

                self.emit_generic_for(loop_id, &variables, &expressions, body, span)
            }
            HirStmtKind::If {
                branches,
                else_block,
            } => {
                let branches = branches
                    .iter()
                    .map(|branch| ConditionalBranchSnapshot {
                        span: branch.span,
                        condition: branch.condition,
                        body: branch.body,
                    })
                    .collect::<Vec<_>>();
                let else_block = *else_block;
                self.emit_if(&branches, else_block)
            }
            HirStmtKind::Local { locals, values } => {
                let locals = locals.clone();
                let values = values.clone();

                self.emit_local_declaration(&locals, &values, span)?;
                Ok(true)
            }
            HirStmtKind::Assign { targets, values } => {
                let targets = self.snapshot_assignment_targets(targets);
                let values = values.clone();

                self.emit_assignment(&targets, &values, span)?;
                Ok(true)
            }
            HirStmtKind::Return { values, exit } => {
                let values = values.clone();
                let exit = exit.clone();
                self.emit_return(&values, &exit, span)?;
                Ok(false)
            }
            HirStmtKind::Break { target, exit } => {
                let target = *target;
                let exit = exit.clone();
                self.emit_break(target, &exit, span)?;
                Ok(false)
            }
            HirStmtKind::Goto { target, exit } => {
                let target = *target;
                let exit = exit.clone();
                self.emit_goto(target, &exit, span)?;
                Ok(false)
            }
            HirStmtKind::Label { .. } => {
                unreachable!("labels are emitted with block reachability context")
            }
            HirStmtKind::Call { call } => {
                self.emit_discard(*call)?;
                Ok(true)
            }
        }
    }

    fn emit_return(
        &mut self,
        values: &[ExprId],
        exit: &ExitPlan,
        span: Span,
    ) -> Result<(), CompileError> {
        if values.is_empty() {
            let close_from = self.return_close_from(exit, span)?;

            self.emitter.emit(
                span,
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    close_from,
                },
            )?;

            return Ok(());
        }

        let mark = self.registers.temporary_mark();
        let value_count = self.emit_expr_list(values, span, ListKind::Results)?;
        let close_from = self.return_close_from(exit, span)?;

        self.emitter.emit(
            span,
            Instruction::Return {
                base: mark.to_bytecode(span)?,
                values: value_count,
                close_from,
            },
        )?;

        self.registers.release_temporaries_to(mark);

        Ok(())
    }

    fn emit_expr_list(
        &mut self,
        exprs: &[ExprId],
        span: Span,
        kind: ListKind,
    ) -> Result<Count, CompileError> {
        if exprs.is_empty() {
            return Ok(Count::Fixed(0));
        }

        let last_idx = exprs.len() - 1;

        for &expr in &exprs[..last_idx] {
            let expr_span = self.function.expressions[expr].span;
            let dst = self.registers.reserve_temporaries(1, expr_span)?.base;
            self.emit_one(expr, dst)?;
        }

        let final_expr = exprs[last_idx];

        match self.emit_expr(final_expr, ResultContext::Open)? {
            Count::Fixed(1) => Ok(Count::Fixed(checked_list_count(exprs.len(), span, kind)?)),
            Count::Open => Ok(Count::Open),
            Count::Fixed(other) => {
                panic!("open expression context produced fixed count {other}")
            }
        }
    }

    fn emit_nil_range(
        &mut self,
        span: Span,
        base: VReg,
        start: u8,
        end: u8,
    ) -> Result<(), CompileError> {
        assert!(start <= end);

        for offset in start..end {
            self.emitter.emit(
                span,
                Instruction::LoadNil {
                    dst: base.offset(u16::from(offset)).to_bytecode(span)?,
                },
            )?;
        }

        Ok(())
    }

    fn emit_exact_expr_list(
        &mut self,
        exprs: &[ExprId],
        width: u8,
        span: Span,
    ) -> Result<RegRange, CompileError> {
        let base = self.registers.top();

        if exprs.is_empty() {
            let results = self.registers.reserve_temporaries(u16::from(width), span)?;
            self.emit_nil_range(span, results.base, 0, width)?;
            return Ok(results);
        }

        let final_index = exprs.len() - 1;
        let mut produced = 0_u8;

        for &expr in &exprs[..final_index] {
            if produced < width {
                self.emit_fixed(expr, ONE_FIXED_RESULT)?;

                produced += 1;
            } else {
                self.emit_discard(expr)?;
            }
        }

        let final_expr = exprs[final_index];
        let remaining = width - produced;

        if let Some(remaining) = NonZeroU8::new(remaining) {
            self.emit_fixed(final_expr, remaining)?;
        } else {
            self.emit_discard(final_expr)?;
        }

        let results = RegRange {
            base,
            len: u16::from(width),
        };

        Ok(results)
    }

    fn emit_local_declaration(
        &mut self,
        locals: &[LocalId],
        values: &[ExprId],
        span: Span,
    ) -> Result<(), CompileError> {
        let count = checked_list_count(locals.len(), span, ListKind::Results)?;

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "local declaration began with live temporaries"
        );

        for &local in locals {
            let slot = &self.locals[local.0 as usize];
            assert_eq!(
                slot.state,
                LocalState::Unseen,
                "local was declared more than once"
            );
            assert!(
                slot.register.is_none(),
                "unseen local already has a register"
            );
        }

        let base = self.registers.top();

        for (offset, &local) in locals.iter().enumerate() {
            let slot = &mut self.locals[local.0 as usize];

            slot.register = Some(base.offset(offset as u16));
            slot.state = LocalState::Reserved;
        }

        let results = self.emit_exact_expr_list(values, count, span)?;

        self.registers.promote_temporaries_to_pinned(results);

        for &local in locals {
            let slot = &mut self.locals[local.0 as usize];

            assert_eq!(slot.state, LocalState::Reserved);
            slot.state = LocalState::Active;
            self.active_locals.push(local);
        }

        for &local in locals {
            let hir_local = &self.function.locals[local];

            if hir_local.attribute == Some(LocalAttribute::Close) {
                let register = self.locals[local.0 as usize]
                    .register
                    .expect("active to-be-closed local has no register");

                self.emitter.emit(
                    hir_local.span,
                    Instruction::MarkToClose {
                        register: register.to_bytecode(hir_local.span)?,
                    },
                )?;
            }
        }

        assert_eq!(
            self.registers.top(),
            self.registers.floor(),
            "local declaration leaked temporary registers"
        );

        Ok(())
    }

    fn snapshot_assignment_targets(&self, targets: &[HirPlace]) -> Vec<AssignmentTargetSnapshot> {
        targets
            .iter()
            .map(|target| match &target.kind {
                HirPlaceKind::Local(local) => {
                    let slot = &self.locals[local.0 as usize];

                    assert_eq!(
                        slot.state,
                        LocalState::Active,
                        "assignment targets an inactive local"
                    );

                    AssignmentTargetSnapshot::Local {
                        span: target.span,
                        register: slot
                            .register
                            .expect("active assignment target has no register"),
                    }
                }
                HirPlaceKind::Upvalue(upvalue) => AssignmentTargetSnapshot::Upvalue {
                    span: target.span,
                    upvalue: self.upvalue_indices[upvalue.0 as usize],
                },
                HirPlaceKind::Index { table, key } => AssignmentTargetSnapshot::Index {
                    span: target.span,
                    table: *table,
                    key: *key,
                },
            })
            .collect()
    }

    fn prepare_assignment_targets(
        &mut self,
        targets: &[AssignmentTargetSnapshot],
    ) -> Result<Vec<PreparedAssignmentTarget>, CompileError> {
        let mut prepared = Vec::with_capacity(targets.len());

        for target in targets.iter().copied() {
            match target {
                AssignmentTargetSnapshot::Local { span, register } => {
                    prepared.push(PreparedAssignmentTarget::Local { span, register });
                }
                AssignmentTargetSnapshot::Upvalue { span, upvalue } => {
                    prepared.push(PreparedAssignmentTarget::Upvalue { span, upvalue });
                }
                AssignmentTargetSnapshot::Index { span, table, key } => {
                    let table_register = self.registers.top();
                    self.emit_fixed(table, ONE_FIXED_RESULT)?;

                    let key_register = self.registers.top();
                    self.emit_fixed(key, ONE_FIXED_RESULT)?;

                    prepared.push(PreparedAssignmentTarget::Index {
                        span,
                        table: table_register,
                        key: key_register,
                    });
                }
            }
        }

        Ok(prepared)
    }

    fn emit_assignment(
        &mut self,
        targets: &[AssignmentTargetSnapshot],
        values: &[ExprId],
        span: Span,
    ) -> Result<(), CompileError> {
        let target_count = checked_list_count(targets.len(), span, ListKind::Results)?;
        let mark = self.registers.temporary_mark();
        let prepared = self.prepare_assignment_targets(targets)?;

        assert_eq!(prepared.len(), targets.len());

        let results = self.emit_exact_expr_list(values, target_count, span)?;

        for (target, src) in prepared.iter().copied().zip(results.iter()).rev() {
            match target {
                PreparedAssignmentTarget::Local { span, register } => {
                    self.emitter.emit(
                        span,
                        Instruction::Move {
                            dst: register.to_bytecode(span)?,
                            src: src.to_bytecode(span)?,
                        },
                    )?;
                }
                PreparedAssignmentTarget::Upvalue { span, upvalue } => {
                    self.emitter.emit(
                        span,
                        Instruction::SetUpvalue {
                            upvalue,
                            src: src.to_bytecode(span)?,
                        },
                    )?;
                }
                PreparedAssignmentTarget::Index { span, table, key } => {
                    self.emitter.emit(
                        span,
                        Instruction::SetTable {
                            table: table.to_bytecode(span)?,
                            key: key.to_bytecode(span)?,
                            value: src.to_bytecode(span)?,
                        },
                    )?;
                }
            }
        }

        self.registers.release_temporaries_to(mark);
        Ok(())
    }

    fn emit_one(&mut self, expr: ExprId, destination: VReg) -> Result<(), CompileError> {
        self.emit_expr(expr, ResultContext::One(destination))?;
        Ok(())
    }

    fn emit_fixed(&mut self, expr: ExprId, count: NonZeroU8) -> Result<(), CompileError> {
        self.emit_expr(expr, ResultContext::Fixed(count))?;
        Ok(())
    }

    fn emit_single_one(
        &mut self,
        span: Span,
        expr: SingleExpr,
        dst: VReg,
    ) -> Result<(), CompileError> {
        match expr {
            SingleExpr::Nil => {
                self.emitter.emit(
                    span,
                    Instruction::LoadNil {
                        dst: dst.to_bytecode(span)?,
                    },
                )?;
            }
            SingleExpr::Bool(value) => {
                self.emitter.emit(
                    span,
                    Instruction::LoadBool {
                        dst: dst.to_bytecode(span)?,
                        value,
                    },
                )?;
            }
            SingleExpr::Integer(value) => {
                if let Ok(value) = i16::try_from(value) {
                    self.emitter.emit(
                        span,
                        Instruction::LoadSmallInt {
                            dst: dst.to_bytecode(span)?,
                            value,
                        },
                    )?;
                } else {
                    let constant = self.constants.intern(ConstantKey::Integer(value), span)?;

                    self.emitter.emit(
                        span,
                        Instruction::LoadConst {
                            dst: dst.to_bytecode(span)?,
                            constant,
                        },
                    )?;
                }
            }
            SingleExpr::FloatBits(bits) => {
                let constant = self.constants.intern(ConstantKey::FloatBits(bits), span)?;

                self.emitter.emit(
                    span,
                    Instruction::LoadConst {
                        dst: dst.to_bytecode(span)?,
                        constant,
                    },
                )?;
            }
            SingleExpr::String(string) => {
                let constant = self.constants.intern(ConstantKey::String(string), span)?;

                self.emitter.emit(
                    span,
                    Instruction::LoadConst {
                        dst: dst.to_bytecode(span)?,
                        constant,
                    },
                )?;
            }
            SingleExpr::Read(binding) => match binding {
                Binding::Local(local) => {
                    let src = {
                        let slot = &self.locals[local.0 as usize];

                        assert_eq!(
                            slot.state,
                            LocalState::Active,
                            "attempted to read a local before activation or after death"
                        );

                        slot.register
                            .expect("active local has no assigned register")
                    };

                    self.emitter.emit(
                        span,
                        Instruction::Move {
                            dst: dst.to_bytecode(span)?,
                            src: src.to_bytecode(span)?,
                        },
                    )?;
                }
                Binding::Upvalue(upvalue) => {
                    let upvalue = self.upvalue_indices[upvalue.0 as usize];

                    self.emitter.emit(
                        span,
                        Instruction::GetUpvalue {
                            dst: dst.to_bytecode(span)?,
                            upvalue,
                        },
                    )?;
                }
            },
            SingleExpr::Unary { operator, operand } => {
                let mark = self.registers.temporary_mark();
                let operand_register = self.registers.reserve_temporaries(1, span)?.base;

                self.emit_one(operand, operand_register)?;

                self.emitter.emit(
                    span,
                    Instruction::Unary {
                        op: bytecode_unary(operator),
                        dst: dst.to_bytecode(span)?,
                        operand: operand_register.to_bytecode(span)?,
                    },
                )?;

                self.registers.release_temporaries_to(mark);
            }
            SingleExpr::Binary {
                left,
                operator,
                right,
            } => match operator {
                BinaryOperator::And => {
                    self.emit_one(left, dst)?;
                    let end = self.emitter.new_label();

                    self.emitter.jump_if_falsy(span, dst, end)?;
                    self.emit_one(right, dst)?;

                    self.emitter.bind(end);
                }
                BinaryOperator::Or => {
                    self.emit_one(left, dst)?;
                    let end = self.emitter.new_label();
                    let right_label = self.emitter.new_label();

                    self.emitter.jump_if_falsy(span, dst, right_label)?;
                    self.emitter.jump(span, end)?;

                    self.emitter.bind(right_label);
                    self.emit_one(right, dst)?;

                    self.emitter.bind(end);
                }
                BinaryOperator::NotEqual => {
                    self.emit_binary_instruction(span, dst, left, right, BytecodeBinaryOp::Equal)?;

                    self.emitter.emit(
                        span,
                        Instruction::Unary {
                            op: BytecodeUnaryOp::Not,
                            dst: dst.to_bytecode(span)?,
                            operand: dst.to_bytecode(span)?,
                        },
                    )?;
                }
                operator => {
                    self.emit_binary_instruction(
                        span,
                        dst,
                        left,
                        right,
                        bytecode_binary(operator),
                    )?;
                }
            },
            SingleExpr::Index { table, key } => {
                let mark = self.registers.temporary_mark();
                let [table_register, key_register] =
                    self.registers.reserve_temporary_array(span)?;

                self.emit_one(table, table_register)?;
                self.emit_one(key, key_register)?;

                self.emitter.emit(
                    span,
                    Instruction::GetTable {
                        dst: dst.to_bytecode(span)?,
                        table: table_register.to_bytecode(span)?,
                        key: key_register.to_bytecode(span)?,
                    },
                )?;

                self.registers.release_temporaries_to(mark);
            }
            SingleExpr::Closure(child) => {
                let child = self.compile_child(child, span)?;
                self.emitter.emit(
                    span,
                    Instruction::Closure {
                        dst: dst.to_bytecode(span)?,
                        child,
                    },
                )?;
            }
            SingleExpr::Table(fields) => {
                self.emit_table_one(span, &fields, dst)?;
            }
        }

        Ok(())
    }

    fn emit_binary_instruction(
        &mut self,
        span: Span,
        dst: VReg,
        source_left: ExprId,
        source_right: ExprId,
        op: BytecodeBinaryOp,
    ) -> Result<(), CompileError> {
        let mark = self.registers.temporary_mark();
        let [left_register, right_register] = self.registers.reserve_temporary_array(span)?;

        self.emit_one(source_left, left_register)?;
        self.emit_one(source_right, right_register)?;

        self.emitter.emit(
            span,
            Instruction::Binary {
                op,
                dst: dst.to_bytecode(span)?,
                left: left_register.to_bytecode(span)?,
                right: right_register.to_bytecode(span)?,
            },
        )?;

        self.registers.release_temporaries_to(mark);

        Ok(())
    }

    fn emit_table_one(
        &mut self,
        span: Span,
        fields: &[TableFieldSnapshot],
        dst: VReg,
    ) -> Result<(), CompileError> {
        let total_field_count = u32::try_from(fields.len()).map_err(|_| CompileError {
            span,
            kind: CompileErrorKind::TooManyTableFields,
        })?;

        let list_field_count = fields
            .iter()
            .filter(|field| matches!(field, TableFieldSnapshot::List { .. }))
            .count();

        let array_hint = u32::try_from(list_field_count).map_err(|_| CompileError {
            span,
            kind: CompileErrorKind::TooManyTableFields,
        })?;

        let hash_hint = total_field_count - array_hint;
        let table = dst.to_bytecode(span)?;

        self.emitter.emit(
            span,
            Instruction::NewTable {
                dst: table,
                array_hint,
                hash_hint,
            },
        )?;

        let final_field_index = fields.len().checked_sub(1);
        let mut list_fields_seen = 0_u32;

        for (field_index, field) in fields.iter().copied().enumerate() {
            match field {
                TableFieldSnapshot::List {
                    span: field_span,
                    value,
                } => {
                    list_fields_seen = list_fields_seen
                        .checked_add(1)
                        .expect("list field count was checked before emission");

                    let mark = self.registers.temporary_mark();
                    let src = self.registers.top();

                    let count = if Some(field_index) == final_field_index {
                        self.emit_expr(value, ResultContext::Open)?
                    } else {
                        let value_span = self.function.expressions[value].span;
                        let output = self.registers.reserve_temporaries(1, value_span)?.base;

                        self.emit_one(value, output)?;
                        Count::Fixed(1)
                    };

                    self.emitter.emit(
                        field_span,
                        Instruction::SetList {
                            table,
                            src: src.to_bytecode(field_span)?,
                            first_index: list_fields_seen,
                            count,
                        },
                    )?;

                    self.registers.release_temporaries_to(mark);
                }
                TableFieldSnapshot::Record {
                    span: field_span,
                    name,
                    value,
                } => {
                    let mark = self.registers.temporary_mark();
                    let [key_register, value_register] =
                        self.registers.reserve_temporary_array(field_span)?;

                    let key_constant = self
                        .constants
                        .intern(ConstantKey::String(name), field_span)?;

                    self.emitter.emit(
                        field_span,
                        Instruction::LoadConst {
                            dst: key_register.to_bytecode(field_span)?,
                            constant: key_constant,
                        },
                    )?;

                    self.emit_one(value, value_register)?;

                    self.emitter.emit(
                        field_span,
                        Instruction::SetTable {
                            table,
                            key: key_register.to_bytecode(field_span)?,
                            value: value_register.to_bytecode(field_span)?,
                        },
                    )?;

                    self.registers.release_temporaries_to(mark);
                }
                TableFieldSnapshot::Computed {
                    span: field_span,
                    key,
                    value,
                } => {
                    let mark = self.registers.temporary_mark();
                    let [key_register, value_register] =
                        self.registers.reserve_temporary_array(field_span)?;

                    self.emit_one(key, key_register)?;
                    self.emit_one(value, value_register)?;

                    self.emitter.emit(
                        field_span,
                        Instruction::SetTable {
                            table,
                            key: key_register.to_bytecode(field_span)?,
                            value: value_register.to_bytecode(field_span)?,
                        },
                    )?;

                    self.registers.release_temporaries_to(mark);
                }
            }
        }

        Ok(())
    }

    fn emit_discard(&mut self, expression: ExprId) -> Result<(), CompileError> {
        self.emit_expr(expression, ResultContext::Discard)?;
        Ok(())
    }

    fn emit_expr(&mut self, expr: ExprId, context: ResultContext) -> Result<Count, CompileError> {
        let (span, expr) = self.result_expr(expr);
        let destination = match context {
            ResultContext::One(destination) => destination,
            ResultContext::Discard | ResultContext::Fixed(_) | ResultContext::Open => {
                self.registers.top()
            }
        };

        match expr {
            ResultExpr::Single(expression) => {
                self.emit_single_for_context(span, expression, destination, context)
            }
            ResultExpr::Vararg => self.emit_vararg(span, destination, context),
            ResultExpr::Call { callee, arguments } => {
                self.emit_call(span, callee, &arguments, destination, context)
            }
            ResultExpr::MethodCall {
                receiver,
                method,
                arguments,
            } => self.emit_method_call(span, receiver, method, &arguments, destination, context),
            ResultExpr::AdjustToOne(expression) => {
                self.emit_adjust_to_one(span, expression, destination, context)
            }
        }
    }

    fn emit_single_for_context(
        &mut self,
        span: Span,
        expr: SingleExpr,
        dst: VReg,
        context: ResultContext,
    ) -> Result<Count, CompileError> {
        match context {
            ResultContext::Discard => {
                let mark = self.registers.temporary_mark();
                let scratch = self.registers.reserve_temporaries(1, span)?.base;

                self.emit_single_one(span, expr, scratch)?;
                self.registers.release_temporaries_to(mark);

                Ok(Count::Fixed(0))
            }
            ResultContext::One(_) => {
                assert!(
                    dst.get() < self.registers.top().get(),
                    "single-result destination must already be reserved"
                );

                self.emit_single_one(span, expr, dst)?;

                Ok(Count::Fixed(1))
            }
            ResultContext::Fixed(count) => {
                let count = count.get();

                self.registers.reserve_temporaries(1, span)?;

                self.emit_single_one(span, expr, dst)?;

                if count > 1 {
                    self.registers
                        .reserve_temporaries(u16::from(count - 1), span)?;
                    self.emit_nil_range(span, dst, 1, count)?;
                }

                Ok(Count::Fixed(count))
            }
            ResultContext::Open => {
                let output = self.registers.reserve_temporaries(1, span)?.base;
                self.emit_single_one(span, expr, output)?;

                Ok(Count::Fixed(1))
            }
        }
    }

    fn emit_adjust_to_one(
        &mut self,
        span: Span,
        expression: ExprId,
        destination: VReg,
        context: ResultContext,
    ) -> Result<Count, CompileError> {
        match context {
            ResultContext::Discard => {
                let mark = self.registers.temporary_mark();
                let scratch = self.registers.reserve_temporaries(1, span)?.base;

                self.emit_one(expression, scratch)?;
                self.registers.release_temporaries_to(mark);

                Ok(Count::Fixed(0))
            }
            ResultContext::One(_) => {
                self.emit_one(expression, destination)?;
                Ok(Count::Fixed(1))
            }
            ResultContext::Fixed(count) => {
                let count = count.get();

                self.emit_fixed(expression, ONE_FIXED_RESULT)?;

                if count > 1 {
                    self.registers
                        .reserve_temporaries(u16::from(count - 1), span)?;
                    self.emit_nil_range(span, destination, 1, count)?;
                }

                Ok(Count::Fixed(count))
            }
            ResultContext::Open => {
                let output = self.registers.reserve_temporaries(1, span)?.base;
                self.emit_one(expression, output)?;
                Ok(Count::Fixed(1))
            }
        }
    }

    fn emit_vararg(
        &mut self,
        span: Span,
        destination: VReg,
        context: ResultContext,
    ) -> Result<Count, CompileError> {
        match context {
            ResultContext::Discard => Ok(Count::Fixed(0)),
            ResultContext::One(_) => {
                assert!(
                    destination.get() < self.registers.top().get(),
                    "single-result destination must already be reserved"
                );

                self.emitter.emit(
                    span,
                    Instruction::Vararg {
                        base: destination.to_bytecode(span)?,
                        results: Count::Fixed(1),
                    },
                )?;

                Ok(Count::Fixed(1))
            }
            ResultContext::Fixed(count) => {
                let count = count.get();

                self.registers.reserve_temporaries(u16::from(count), span)?;

                self.emitter.emit(
                    span,
                    Instruction::Vararg {
                        base: destination.to_bytecode(span)?,
                        results: Count::Fixed(count),
                    },
                )?;

                Ok(Count::Fixed(count))
            }
            ResultContext::Open => {
                let mark = self.registers.temporary_mark();
                let output = self.registers.reserve_temporaries(1, span)?.base;

                self.emitter.emit(
                    span,
                    Instruction::Vararg {
                        base: output.to_bytecode(span)?,
                        results: Count::Open,
                    },
                )?;

                self.registers.release_temporaries_to(mark);

                Ok(Count::Open)
            }
        }
    }

    fn emit_call(
        &mut self,
        span: Span,
        callee: ExprId,
        arguments: &[ExprId],
        destination: VReg,
        context: ResultContext,
    ) -> Result<Count, CompileError> {
        checked_list_count(arguments.len(), span, ListKind::Arguments)?;
        let mark = self.registers.temporary_mark();

        if let ResultContext::One(_) = context {
            assert!(
                destination.get() < mark.get(),
                "single call destination must already be reserved"
            );
        }

        let base = self.registers.reserve_temporaries(1, span)?.base;
        self.emit_one(callee, base)?;
        let argument_count = self.emit_expr_list(arguments, span, ListKind::Arguments)?;
        self.finish_call(span, base, argument_count, context)
    }

    fn emit_method_call(
        &mut self,
        span: Span,
        receiver: ExprId,
        method: StringIndex,
        arguments: &[ExprId],
        destination: VReg,
        context: ResultContext,
    ) -> Result<Count, CompileError> {
        // self consumes one of the 255 fixed argument positions.
        u8::try_from(arguments.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(CompileError {
                span,
                kind: CompileErrorKind::TooManyArguments,
            })?;

        let mark = self.registers.temporary_mark();

        if let ResultContext::One(_) = context {
            assert!(
                destination.get() < mark.get(),
                "single method destination must already be reserved"
            );
        }

        let [base, receiver_register] = self.registers.reserve_temporary_array(span)?;

        self.emit_one(receiver, receiver_register)?;

        let key_mark = self.registers.temporary_mark();
        let key_register = self.registers.reserve_temporaries(1, span)?.base;

        let method_constant = self.constants.intern(ConstantKey::String(method), span)?;

        self.emitter.emit(
            span,
            Instruction::LoadConst {
                dst: key_register.to_bytecode(span)?,
                constant: method_constant,
            },
        )?;

        self.emitter.emit(
            span,
            Instruction::GetTable {
                dst: base.to_bytecode(span)?,
                table: receiver_register.to_bytecode(span)?,
                key: key_register.to_bytecode(span)?,
            },
        )?;

        self.registers.release_temporaries_to(key_mark);

        let explicit_argument_count = self.emit_expr_list(arguments, span, ListKind::Arguments)?;

        let argument_count = match explicit_argument_count {
            Count::Fixed(explicit) => Count::Fixed(
                explicit
                    .checked_add(1)
                    .expect("method argument count was checked"),
            ),

            Count::Open => Count::Open,
        };

        self.finish_call(span, base, argument_count, context)
    }

    fn finish_call(
        &mut self,
        span: Span,
        base: VReg,
        arguments: Count,
        context: ResultContext,
    ) -> Result<Count, CompileError> {
        let results = match context {
            ResultContext::Discard => Count::Fixed(0),
            ResultContext::One(_) => Count::Fixed(1),
            ResultContext::Fixed(count) => Count::Fixed(count.get()),
            ResultContext::Open => Count::Open,
        };

        if let ResultContext::Fixed(count) = context {
            let count = count.get();
            let required_end = u32::from(base.get()) + u32::from(count);
            let current_top = u32::from(self.registers.top().get());

            if required_end > current_top {
                let additional = u16::try_from(required_end - current_top)
                    .expect("call result extension fits in u16");

                self.registers.reserve_temporaries(additional, span)?;
            }
        }

        self.emitter.emit(
            span,
            Instruction::Call {
                base: base.to_bytecode(span)?,
                arguments,
                results,
            },
        )?;

        if let ResultContext::One(dst) = context {
            self.emitter.emit(
                span,
                Instruction::Move {
                    dst: dst.to_bytecode(span)?,
                    src: base.to_bytecode(span)?,
                },
            )?;
        }

        self.registers.release_temporaries_to(base);

        if let ResultContext::Fixed(count) = context {
            let count = count.get();
            self.registers.reserve_temporaries(u16::from(count), span)?;
        }

        Ok(results)
    }

    fn result_expr(&self, expression: ExprId) -> (Span, ResultExpr) {
        let expression = &self.function.expressions[expression];

        let kind = match &expression.kind {
            HirExprKind::Nil => ResultExpr::Single(SingleExpr::Nil),
            HirExprKind::Boolean(value) => ResultExpr::Single(SingleExpr::Bool(*value)),
            HirExprKind::Integer(value) => ResultExpr::Single(SingleExpr::Integer(*value)),
            HirExprKind::Float(value) => ResultExpr::Single(SingleExpr::FloatBits(value.to_bits())),
            HirExprKind::String(string) => {
                ResultExpr::Single(SingleExpr::String(StringIndex::new(string.get())))
            }
            HirExprKind::Vararg => ResultExpr::Vararg,
            HirExprKind::Read(binding) => ResultExpr::Single(SingleExpr::Read(*binding)),
            HirExprKind::Unary { operator, operand } => ResultExpr::Single(SingleExpr::Unary {
                operator: *operator,
                operand: *operand,
            }),
            HirExprKind::Binary {
                left,
                operator,
                right,
            } => ResultExpr::Single(SingleExpr::Binary {
                left: *left,
                operator: *operator,
                right: *right,
            }),
            HirExprKind::Index { table, key } => ResultExpr::Single(SingleExpr::Index {
                table: *table,
                key: *key,
            }),
            HirExprKind::Call { callee, arguments } => ResultExpr::Call {
                callee: *callee,
                arguments: arguments.clone(),
            },
            HirExprKind::MethodCall {
                receiver,
                method,
                arguments,
            } => ResultExpr::MethodCall {
                receiver: *receiver,
                method: StringIndex::new(method.get()),
                arguments: arguments.clone(),
            },
            HirExprKind::Closure(child) => ResultExpr::Single(SingleExpr::Closure(*child)),
            HirExprKind::Table { fields } => {
                let fields = fields
                    .iter()
                    .map(|field| match field {
                        HirTableField::List { span, value } => TableFieldSnapshot::List {
                            span: *span,
                            value: *value,
                        },
                        HirTableField::Record { span, name, value } => TableFieldSnapshot::Record {
                            span: *span,
                            name: StringIndex::new(name.get()),
                            value: *value,
                        },
                        HirTableField::Computed { span, key, value } => {
                            TableFieldSnapshot::Computed {
                                span: *span,
                                key: *key,
                                value: *value,
                            }
                        }
                    })
                    .collect();

                ResultExpr::Single(SingleExpr::Table(fields))
            }
            HirExprKind::AdjustToOne { expression } => ResultExpr::AdjustToOne(*expression),
        };

        (expression.span, kind)
    }
}

fn compile_function(
    function: &HirFunction,
    parent: Option<&ParentLayout<'_>>,
) -> Result<Prototype, CompileError> {
    let mut compiler = FunctionCompiler::new(function, parent)?;

    compiler.enter_root_scope();
    let parameter_count = compiler.initialize_parameters()?;

    let falls_through = compiler.emit_block_stmts(function.body)?;

    let implicit_close_from = if falls_through {
        let root = *compiler
            .scopes
            .last()
            .expect("root scope must still be active");

        compiler
            .active_scope_requires_close(root)
            .then(|| root.register_base.to_bytecode(function.span))
            .transpose()?
    } else {
        None
    };

    compiler.leave_scope(function.span, false)?;

    if falls_through {
        compiler.emitter.emit(
            function.span,
            Instruction::Return {
                base: Register(0),
                values: Count::Fixed(0),
                close_from: implicit_close_from,
            },
        )?;
    }

    let max_registers = compiler.registers.max_registers().max(1);
    let constants = compiler.constants.finish();
    let emitted = compiler.emitter.finish()?;

    Ok(Prototype {
        span: function.span,
        parameter_count,
        is_vararg: function.is_vararg,
        max_registers,
        constants,
        upvalues: compiler.upvalues.into_boxed_slice(),
        children: compiler.children.into_boxed_slice(),
        code: emitted.instructions,
        source_map: emitted.source_map,
    })
}

pub(crate) fn compile_entry(function: &HirFunction) -> Result<Prototype, CompileError> {
    compile_function(function, None)
}

fn checked_list_count(length: usize, span: Span, kind: ListKind) -> Result<u8, CompileError> {
    u8::try_from(length).map_err(|_| CompileError {
        span,
        kind: match kind {
            ListKind::Arguments => CompileErrorKind::TooManyArguments,
            ListKind::Results => CompileErrorKind::TooManyResults,
        },
    })
}

fn bytecode_unary(operator: UnaryOperator) -> BytecodeUnaryOp {
    match operator {
        UnaryOperator::Negate => BytecodeUnaryOp::Negate,
        UnaryOperator::Not => BytecodeUnaryOp::Not,
        UnaryOperator::Length => BytecodeUnaryOp::Length,
        UnaryOperator::BitwiseNot => BytecodeUnaryOp::BitwiseNot,
    }
}

fn bytecode_binary(operator: BinaryOperator) -> BytecodeBinaryOp {
    match operator {
        BinaryOperator::Add => BytecodeBinaryOp::Add,
        BinaryOperator::Subtract => BytecodeBinaryOp::Subtract,
        BinaryOperator::Multiply => BytecodeBinaryOp::Multiply,
        BinaryOperator::Divide => BytecodeBinaryOp::Divide,
        BinaryOperator::FloorDivide => BytecodeBinaryOp::FloorDivide,
        BinaryOperator::Modulo => BytecodeBinaryOp::Modulo,
        BinaryOperator::Power => BytecodeBinaryOp::Power,

        BinaryOperator::BitwiseAnd => BytecodeBinaryOp::BitwiseAnd,
        BinaryOperator::BitwiseOr => BytecodeBinaryOp::BitwiseOr,
        BinaryOperator::BitwiseXor => BytecodeBinaryOp::BitwiseXor,
        BinaryOperator::ShiftLeft => BytecodeBinaryOp::ShiftLeft,
        BinaryOperator::ShiftRight => BytecodeBinaryOp::ShiftRight,

        BinaryOperator::Concat => BytecodeBinaryOp::Concat,

        BinaryOperator::Equal => BytecodeBinaryOp::Equal,
        BinaryOperator::LessThan => BytecodeBinaryOp::LessThan,
        BinaryOperator::LessEqual => BytecodeBinaryOp::LessEqual,
        BinaryOperator::GreaterThan => BytecodeBinaryOp::GreaterThan,
        BinaryOperator::GreaterEqual => BytecodeBinaryOp::GreaterEqual,

        BinaryOperator::And | BinaryOperator::Or | BinaryOperator::NotEqual => {
            unreachable!("operator is lowered specially")
        }
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::SourceId;
    use orbit_parser::{lexer::lex, parser::parse_chunk};

    use crate::{
        CompileError, CompileErrorKind,
        bytecode::{Chunk, Constant, ConstantIndex, StringIndex, UpvalueDescriptor, UpvalueIndex},
        compile,
    };

    use super::*;

    fn compile_source_result(source: &str) -> Result<Chunk, CompileError> {
        let source_id = SourceId::new(0);
        let tokens = lex(source_id, source).unwrap();
        let ast = parse_chunk(source_id, &tokens).unwrap();
        let hir = orbit_resolver::resolve(&ast).unwrap();
        compile(hir)
    }

    fn compile_source(source: &str) -> Chunk {
        compile_source_result(source).unwrap()
    }

    fn assert_unary_operator(source: &str, expected: BytecodeUnaryOp) {
        let chunk = compile_source(source);

        assert_eq!(chunk.entry.max_registers, 2, "source: {source}");

        let [
            _,
            Instruction::Unary { op, dst, operand },
            Instruction::Return { base, values, .. },
        ] = chunk.entry.code.as_ref()
        else {
            panic!("unexpected bytecode for {source}: {:#?}", chunk.entry.code);
        };

        assert_eq!(*op, expected, "source: {source}");
        assert_eq!(*dst, Register(0), "source: {source}");
        assert_eq!(*operand, Register(1), "source: {source}");
        assert_eq!(*base, Register(0), "source: {source}");
        assert_eq!(*values, Count::Fixed(1), "source: {source}");
    }

    fn assert_binary_operator(operator: &str, expected: BytecodeBinaryOp) {
        let source = format!("return 1 {operator} 2");
        let chunk = compile_source(&source);

        assert_eq!(chunk.entry.max_registers, 3, "source: {source}");

        let [
            Instruction::LoadSmallInt {
                dst: left_dst,
                value: left_value,
            },
            Instruction::LoadSmallInt {
                dst: right_dst,
                value: right_value,
            },
            Instruction::Binary {
                op,
                dst,
                left,
                right,
            },
            Instruction::Return { base, values, .. },
        ] = chunk.entry.code.as_ref()
        else {
            panic!("unexpected bytecode for {source}: {:#?}", chunk.entry.code);
        };

        assert_eq!(*left_dst, Register(1), "source: {source}");
        assert_eq!(*left_value, 1, "source: {source}");
        assert_eq!(*right_dst, Register(2), "source: {source}");
        assert_eq!(*right_value, 2, "source: {source}");
        assert_eq!(*op, expected, "source: {source}");
        assert_eq!(*dst, Register(0), "source: {source}");
        assert_eq!(*left, Register(1), "source: {source}");
        assert_eq!(*right, Register(2), "source: {source}");
        assert_eq!(*base, Register(0), "source: {source}");
        assert_eq!(*values, Count::Fixed(1), "source: {source}");
    }

    fn calls(chunk: &Chunk) -> Vec<(Register, Count, Count)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Call {
                    base,
                    arguments,
                    results,
                } => Some((*base, *arguments, *results)),

                _ => None,
            })
            .collect()
    }

    fn varargs(chunk: &Chunk) -> Vec<(Register, Count)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Vararg { base, results } => Some((*base, *results)),

                _ => None,
            })
            .collect()
    }

    fn returns(chunk: &Chunk) -> Vec<(Register, Count)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Return { base, values, .. } => Some((*base, *values)),
                _ => None,
            })
            .collect()
    }

    fn return_details(chunk: &Chunk) -> Vec<(Register, Count, Option<Register>)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Return {
                    base,
                    values,
                    close_from,
                } => Some((*base, *values, *close_from)),
                _ => None,
            })
            .collect()
    }

    fn marked_to_close(chunk: &Chunk) -> Vec<Register> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::MarkToClose { register } => Some(*register),
                _ => None,
            })
            .collect()
    }

    fn standalone_closes(chunk: &Chunk) -> Vec<Register> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::CloseFrom { base } => Some(*base),
                _ => None,
            })
            .collect()
    }

    fn jump_offsets(chunk: &Chunk) -> Vec<i32> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Jump { offset } => Some(*offset),
                _ => None,
            })
            .collect()
    }

    fn for_preps(chunk: &Chunk) -> Vec<(Register, i32)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::ForPrep { base, exit_offset } => Some((*base, *exit_offset)),
                _ => None,
            })
            .collect()
    }

    fn for_loops(chunk: &Chunk) -> Vec<(Register, i32)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::ForLoop { base, body_offset } => Some((*base, *body_offset)),
                _ => None,
            })
            .collect()
    }

    fn tfor_calls(chunk: &Chunk) -> Vec<(Register, u8)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::TForCall { base, variables } => Some((*base, *variables)),
                _ => None,
            })
            .collect()
    }

    fn tfor_loops(chunk: &Chunk) -> Vec<(Register, i32)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::TForLoop { base, body_offset } => Some((*base, *body_offset)),
                _ => None,
            })
            .collect()
    }

    fn table_allocations(chunk: &Chunk) -> Vec<(Register, u32, u32)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::NewTable {
                    dst,
                    array_hint,
                    hash_hint,
                } => Some((*dst, *array_hint, *hash_hint)),
                _ => None,
            })
            .collect()
    }

    fn set_lists(chunk: &Chunk) -> Vec<(Register, Register, u32, Count)> {
        chunk
            .entry
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::SetList {
                    table,
                    src,
                    first_index,
                    count,
                } => Some((*table, *src, *first_index, *count)),
                _ => None,
            })
            .collect()
    }

    fn closures(prototype: &Prototype) -> Vec<(Register, PrototypeIndex)> {
        prototype
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Closure { dst, child } => Some((*dst, *child)),
                _ => None,
            })
            .collect()
    }

    fn prototype_returns(prototype: &Prototype) -> Vec<(Register, Count)> {
        prototype
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Return { base, values, .. } => Some((*base, *values)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn compiles_an_empty_chunk() {
        let chunk = compile_source("");

        assert_eq!(chunk.entry.parameter_count, 0);
        assert_eq!(chunk.entry.max_registers, 1);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [Instruction::Return {
                base: Register(0),
                values: Count::Fixed(0),
                ..
            }]
        ));
    }

    #[test]
    fn compiles_literal_returns() {
        let chunk = compile_source(r#"return nil, true, false, 12, 100000, 1.5, "hello""#);

        assert_eq!(chunk.entry.max_registers, 7);

        assert!(matches!(
            chunk.entry.constants.as_ref(),
            [
                Constant::Integer(100000),
                Constant::FloatBits(bits),
                Constant::String(string),
            ] if *bits == 1.5_f64.to_bits()
                && *string == StringIndex::new(0)
        ));

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadNil {
                    dst: Register(0)
                },
                Instruction::LoadBool {
                    dst: Register(1),
                    value: true
                },
                Instruction::LoadBool {
                    dst: Register(2),
                    value: false
                },
                Instruction::LoadSmallInt {
                    dst: Register(3),
                    value: 12
                },
                Instruction::LoadConst {
                    dst: Register(4),
                    constant: integer
                },
                Instruction::LoadConst {
                    dst: Register(5),
                    constant: float
                },
                Instruction::LoadConst {
                    dst: Register(6),
                    constant: string
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(7),
                    ..
                },
            ] if *integer == ConstantIndex::new(0)
                && *float == ConstantIndex::new(1)
                && *string == ConstantIndex::new(2)
        ));
    }

    #[test]
    fn compiles_an_explicit_empty_return_without_a_fallback_return() {
        let chunk = compile_source("return");

        assert_eq!(chunk.entry.max_registers, 1);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [Instruction::Return {
                base: Register(0),
                values: Count::Fixed(0),
                ..
            }]
        ));
    }

    #[test]
    fn compiles_all_unary_operators() {
        assert_unary_operator("return -1", BytecodeUnaryOp::Negate);
        assert_unary_operator("return not false", BytecodeUnaryOp::Not);
        assert_unary_operator(r#"return #"hello""#, BytecodeUnaryOp::Length);
        assert_unary_operator("return ~1", BytecodeUnaryOp::BitwiseNot);
    }

    #[test]
    fn compiles_all_non_short_circuit_binary_operators() {
        let cases = [
            ("+", BytecodeBinaryOp::Add),
            ("-", BytecodeBinaryOp::Subtract),
            ("*", BytecodeBinaryOp::Multiply),
            ("/", BytecodeBinaryOp::Divide),
            ("//", BytecodeBinaryOp::FloorDivide),
            ("%", BytecodeBinaryOp::Modulo),
            ("^", BytecodeBinaryOp::Power),
            ("&", BytecodeBinaryOp::BitwiseAnd),
            ("|", BytecodeBinaryOp::BitwiseOr),
            ("~", BytecodeBinaryOp::BitwiseXor),
            ("<<", BytecodeBinaryOp::ShiftLeft),
            (">>", BytecodeBinaryOp::ShiftRight),
            ("..", BytecodeBinaryOp::Concat),
            ("==", BytecodeBinaryOp::Equal),
            ("<", BytecodeBinaryOp::LessThan),
            ("<=", BytecodeBinaryOp::LessEqual),
            (">", BytecodeBinaryOp::GreaterThan),
            (">=", BytecodeBinaryOp::GreaterEqual),
        ];

        for (operator, expected) in cases {
            assert_binary_operator(operator, expected);
        }
    }

    #[test]
    fn compiles_not_equal() {
        let source = "return 1 ~= 2";
        let chunk = compile_source(source);

        assert_eq!(chunk.entry.max_registers, 3);

        let [
            Instruction::LoadSmallInt {
                dst: Register(1),
                value: 1,
            },
            Instruction::LoadSmallInt {
                dst: Register(2),
                value: 2,
            },
            Instruction::Binary {
                op: BytecodeBinaryOp::Equal,
                dst: Register(0),
                left: Register(1),
                right: Register(2),
            },
            Instruction::Unary {
                op: BytecodeUnaryOp::Not,
                dst: Register(0),
                operand: Register(0),
            },
            Instruction::Return {
                base: Register(0),
                values: Count::Fixed(1),
                ..
            },
        ] = chunk.entry.code.as_ref()
        else {
            panic!("unexpected bytecode for {source}: {:#?}", chunk.entry.code);
        };
    }

    #[test]
    fn compiles_a_global_read_through_the_environment_upvalue() {
        let chunk = compile_source("return global_name");

        assert_eq!(chunk.strings.len(), 1);
        assert_eq!(chunk.strings[0].as_ref(), b"global_name");
        assert_eq!(chunk.entry.max_registers, 3);
        assert!(matches!(
            chunk.entry.upvalues.as_ref(),
            [UpvalueDescriptor::ExternalEnvironment]
        ));
        assert!(matches!(
            chunk.entry.constants.as_ref(),
            [Constant::String(string)] if *string == StringIndex::new(0)
        ));
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::GetUpvalue {
                    dst: Register(1),
                    upvalue,
                },
                Instruction::LoadConst {
                    dst: Register(2),
                    constant,
                },
                Instruction::GetTable {
                    dst: Register(0),
                    table: Register(1),
                    key: Register(2),
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    ..
                },
            ] if *upvalue == UpvalueIndex::new(0)
                && *constant == ConstantIndex::new(0)
        ));
    }

    #[test]
    fn reuses_nested_expression_temporaries() {
        let chunk = compile_source("return global_name[1 + 2]");

        assert_eq!(chunk.entry.max_registers, 5);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::GetUpvalue {
                    dst: Register(3),
                    upvalue,
                },
                Instruction::LoadConst {
                    dst: Register(4),
                    constant,
                },
                Instruction::GetTable {
                    dst: Register(1),
                    table: Register(3),
                    key: Register(4),
                },
                Instruction::LoadSmallInt {
                    dst: Register(3),
                    value: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(4),
                    value: 2,
                },
                Instruction::Binary {
                    op: BytecodeBinaryOp::Add,
                    dst: Register(2),
                    left: Register(3),
                    right: Register(4),
                },
                Instruction::GetTable {
                    dst: Register(0),
                    table: Register(1),
                    key: Register(2),
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    ..
                },
            ] if *upvalue == UpvalueIndex::new(0)
                && *constant == ConstantIndex::new(0)
        ));
    }

    #[test]
    fn deduplicates_constants_within_a_prototype() {
        let chunk = compile_source(r#"return 100000, 100000, "same", "same""#);

        assert!(matches!(
            chunk.entry.constants.as_ref(),
            [Constant::Integer(100000), Constant::String(string)]
                if *string == StringIndex::new(0)
        ));
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadConst {
                    dst: Register(0),
                    constant: integer_a,
                },
                Instruction::LoadConst {
                    dst: Register(1),
                    constant: integer_b,
                },
                Instruction::LoadConst {
                    dst: Register(2),
                    constant: string_a,
                },
                Instruction::LoadConst {
                    dst: Register(3),
                    constant: string_b,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(4),
                    ..
                },
            ] if *integer_a == ConstantIndex::new(0)
                && *integer_b == ConstantIndex::new(0)
                && *string_a == ConstantIndex::new(1)
                && *string_b == ConstantIndex::new(1)
        ));
    }

    #[test]
    fn rejects_a_return_count_that_does_not_fit_in_bytecode() {
        let source = format!("return {}", vec!["nil"; 256].join(", "));
        let error = compile_source_result(&source).unwrap_err();

        assert!(matches!(error.kind, CompileErrorKind::TooManyResults));
    }

    #[test]
    fn compiles_short_circuit_and() {
        let chunk = compile_source("return false and 7");

        assert_eq!(chunk.entry.max_registers, 1);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadBool {
                    dst: Register(0),
                    value: false,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(0),
                    offset: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 7,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    ..
                },
            ]
        ));
    }

    #[test]
    fn compiles_short_circuit_or() {
        let chunk = compile_source("return true or 7");

        assert_eq!(chunk.entry.max_registers, 1);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadBool {
                    dst: Register(0),
                    value: true,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(0),
                    offset: 1,
                },
                Instruction::Jump { offset: 1 },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 7,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    ..
                },
            ]
        ));
    }

    #[test]
    fn adjusts_return_tail_cardinality() {
        let chunk = compile_source("return (nil)()");

        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(0), Count::Open)]
        );
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Open)]);

        let chunk = compile_source("return ((nil)())");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Fixed(1))]
        );
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(1))]);

        let chunk = compile_source("return (nil)(), 7");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Fixed(1))]
        );
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(2))]);

        let chunk = compile_source("return 7, (nil)()");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Open)]
        );
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Open)]);
    }

    #[test]
    fn adjusts_vararg_cardinality() {
        let chunk = compile_source("return ...");

        assert_eq!(varargs(&chunk), vec![(Register(0), Count::Open)]);
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Open)]);

        let chunk = compile_source("return (...)");

        assert_eq!(varargs(&chunk), vec![(Register(0), Count::Fixed(1))]);
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(1))]);

        let chunk = compile_source("return ..., 7");

        assert_eq!(varargs(&chunk), vec![(Register(0), Count::Fixed(1))]);
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(2))]);

        let chunk = compile_source("return 7, ...");

        assert_eq!(varargs(&chunk), vec![(Register(1), Count::Open)]);
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Open)]);
    }

    #[test]
    fn adjusts_final_call_argument_only() {
        let chunk = compile_source("return (nil)(1, 2)");

        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(2), Count::Open)]
        );

        let chunk = compile_source("return (nil)((nil)())");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(1), Count::Fixed(0), Count::Open),
                (Register(0), Count::Open, Count::Open),
            ]
        );

        let chunk = compile_source("return (nil)((nil)(), 7)");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(2), Count::Fixed(0), Count::Fixed(1)),
                (Register(0), Count::Fixed(2), Count::Open),
            ]
        );

        let chunk = compile_source("return (nil)(((nil)()))");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(2), Count::Fixed(0), Count::Fixed(1)),
                (Register(0), Count::Fixed(1), Count::Open),
            ]
        );
    }

    #[test]
    fn lowers_call_statements_and_methods() {
        let chunk = compile_source("(nil)()");

        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(0), Count::Fixed(0))]
        );
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(0))]);

        let chunk = compile_source("return (nil):m(7)");

        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(2), Count::Open)]
        );

        let chunk = compile_source("return (nil):m((nil)())");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(2), Count::Fixed(0), Count::Open),
                (Register(0), Count::Open, Count::Open),
            ]
        );
    }

    #[test]
    fn allows_an_open_return_tail_in_register_255() {
        let mut values = vec!["nil"; 255];
        values.push("...");
        let source = format!("return {}", values.join(", "));
        let chunk = compile_source(&source);

        assert_eq!(chunk.entry.max_registers, 256);
        assert!(matches!(
            &chunk.entry.code[chunk.entry.code.len() - 2..],
            [
                Instruction::Vararg {
                    base: Register(255),
                    results: Count::Open,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Open,
                    ..
                },
            ]
        ));
    }

    #[test]
    fn lowers_empty_and_mixed_table_constructors() {
        let empty = compile_source("return {}");

        assert_eq!(empty.entry.max_registers, 1);
        assert!(matches!(
            empty.entry.code.as_ref(),
            [
                Instruction::NewTable {
                    dst: Register(0),
                    array_hint: 0,
                    hash_hint: 0,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    ..
                },
            ]
        ));

        let chunk = compile_source("return {10, x = 20, [30] = 40, 50}");

        assert_eq!(chunk.entry.max_registers, 3);
        assert!(matches!(
            chunk.entry.constants.as_ref(),
            [Constant::String(name)] if *name == StringIndex::new(0)
        ));
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::NewTable {
                    dst: Register(0),
                    array_hint: 2,
                    hash_hint: 2,
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 10,
                },
                Instruction::SetList {
                    table: Register(0),
                    src: Register(1),
                    first_index: 1,
                    count: Count::Fixed(1),
                },
                Instruction::LoadConst {
                    dst: Register(1),
                    constant,
                },
                Instruction::LoadSmallInt {
                    dst: Register(2),
                    value: 20,
                },
                Instruction::SetTable {
                    table: Register(0),
                    key: Register(1),
                    value: Register(2),
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 30,
                },
                Instruction::LoadSmallInt {
                    dst: Register(2),
                    value: 40,
                },
                Instruction::SetTable {
                    table: Register(0),
                    key: Register(1),
                    value: Register(2),
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 50,
                },
                Instruction::SetList {
                    table: Register(0),
                    src: Register(1),
                    first_index: 2,
                    count: Count::Fixed(1),
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    ..
                },
            ] if *constant == ConstantIndex::new(0)
        ));
    }

    #[test]
    fn adjusts_table_list_call_cardinality() {
        let chunk = compile_source("return {(nil)()}");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Open)]
        );
        assert_eq!(
            set_lists(&chunk),
            vec![(Register(0), Register(1), 1, Count::Open)]
        );
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(1))]);
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::Call {
                    results: Count::Open,
                    ..
                },
                Instruction::SetList {
                    count: Count::Open,
                    ..
                },
            ]
        )));

        let chunk = compile_source("return {((nil)())}");

        assert_eq!(
            calls(&chunk),
            vec![(Register(2), Count::Fixed(0), Count::Fixed(1))]
        );
        assert_eq!(
            set_lists(&chunk),
            vec![(Register(0), Register(1), 1, Count::Fixed(1))]
        );

        let chunk = compile_source("return {(nil)(), x = 1}");

        assert_eq!(
            calls(&chunk),
            vec![(Register(2), Count::Fixed(0), Count::Fixed(1))]
        );
        assert_eq!(
            set_lists(&chunk),
            vec![(Register(0), Register(1), 1, Count::Fixed(1))]
        );

        let chunk = compile_source("return {x = 1, (nil)()}");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Open)]
        );
        assert_eq!(
            set_lists(&chunk),
            vec![(Register(0), Register(1), 1, Count::Open)]
        );

        let chunk = compile_source("return {x = (nil)()}");

        assert_eq!(
            calls(&chunk),
            vec![(Register(3), Count::Fixed(0), Count::Fixed(1))]
        );
        assert!(set_lists(&chunk).is_empty());

        let chunk = compile_source("return {(nil):m()}");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(1), Count::Open)]
        );
        assert_eq!(
            set_lists(&chunk),
            vec![(Register(0), Register(1), 1, Count::Open)]
        );
    }

    #[test]
    fn adjusts_table_list_vararg_cardinality() {
        let chunk = compile_source("return {1, ..., 3}");

        assert_eq!(varargs(&chunk), vec![(Register(1), Count::Fixed(1))]);
        assert_eq!(
            set_lists(&chunk),
            vec![
                (Register(0), Register(1), 1, Count::Fixed(1)),
                (Register(0), Register(1), 2, Count::Fixed(1)),
                (Register(0), Register(1), 3, Count::Fixed(1)),
            ]
        );

        let chunk = compile_source("return {1, ...}");

        assert_eq!(varargs(&chunk), vec![(Register(1), Count::Open)]);
        assert_eq!(
            set_lists(&chunk),
            vec![
                (Register(0), Register(1), 1, Count::Fixed(1)),
                (Register(0), Register(1), 2, Count::Open),
            ]
        );
        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(1))]);
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::Vararg {
                    results: Count::Open,
                    ..
                },
                Instruction::SetList {
                    count: Count::Open,
                    ..
                },
            ]
        )));
    }

    #[test]
    fn evaluates_computed_table_keys_before_values() {
        let chunk = compile_source("return {[(true)()] = (false)()}");

        assert_eq!(chunk.entry.max_registers, 4);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::NewTable {
                    dst: Register(0),
                    array_hint: 0,
                    hash_hint: 1,
                },
                Instruction::LoadBool {
                    dst: Register(3),
                    value: true,
                },
                Instruction::Call {
                    base: Register(3),
                    arguments: Count::Fixed(0),
                    results: Count::Fixed(1),
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(3),
                },
                Instruction::LoadBool {
                    dst: Register(3),
                    value: false,
                },
                Instruction::Call {
                    base: Register(3),
                    arguments: Count::Fixed(0),
                    results: Count::Fixed(1),
                },
                Instruction::Move {
                    dst: Register(2),
                    src: Register(3),
                },
                Instruction::SetTable {
                    table: Register(0),
                    key: Register(1),
                    value: Register(2),
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    ..
                },
            ]
        ));
    }

    #[test]
    fn lowers_nested_tables_and_deduplicates_record_keys() {
        let chunk = compile_source("return {{}, [1] = 2}");

        assert_eq!(
            table_allocations(&chunk),
            vec![(Register(0), 1, 1), (Register(1), 0, 0)]
        );
        assert_eq!(
            set_lists(&chunk),
            vec![(Register(0), Register(1), 1, Count::Fixed(1))]
        );

        let chunk = compile_source("return {x = 1, x = 2}");

        assert_eq!(table_allocations(&chunk), vec![(Register(0), 0, 2)]);
        assert!(matches!(
            chunk.entry.constants.as_ref(),
            [Constant::String(name)] if *name == StringIndex::new(0)
        ));
    }

    #[test]
    fn handles_more_than_255_table_list_fields() {
        let mut fields = vec!["nil"; 300];
        fields.push("...");
        let source = format!("return {{{}}}", fields.join(", "));
        let chunk = compile_source(&source);
        let writes = set_lists(&chunk);

        assert_eq!(chunk.entry.max_registers, 2);
        assert_eq!(table_allocations(&chunk), vec![(Register(0), 301, 0)]);
        assert_eq!(writes.len(), 301);
        assert_eq!(
            writes.last(),
            Some(&(Register(0), Register(1), 301, Count::Open))
        );
        assert_eq!(varargs(&chunk), vec![(Register(1), Count::Open)]);
    }

    #[test]
    fn lowers_local_declarations_with_exact_rhs_adjustment() {
        let chunk = compile_source("local a, b\nreturn a, b");

        assert_eq!(chunk.entry.max_registers, 4);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadNil { dst: Register(0) },
                Instruction::LoadNil { dst: Register(1) },
                Instruction::Move {
                    dst: Register(2),
                    src: Register(0),
                },
                Instruction::Move {
                    dst: Register(3),
                    src: Register(1),
                },
                Instruction::Return {
                    base: Register(2),
                    values: Count::Fixed(2),
                    ..
                },
            ]
        ));

        let chunk = compile_source("local a, b = 7\nreturn a, b");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 7,
                },
                Instruction::LoadNil { dst: Register(1) },
                ..
            ]
        ));

        let chunk = compile_source("local a, b, c = (nil)()\nreturn a, b, c");

        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(0), Count::Fixed(3))]
        );
        assert_eq!(chunk.entry.max_registers, 6);

        let chunk = compile_source("local a, b = ((nil)())\nreturn a, b");

        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(0), Count::Fixed(1))]
        );
        assert!(
            chunk.entry.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::LoadNil { dst: Register(1) }
            ))
        );

        let chunk = compile_source("local a, b, c = (nil)(), (false)()\nreturn a, b, c");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(0), Count::Fixed(0), Count::Fixed(1)),
                (Register(1), Count::Fixed(0), Count::Fixed(2)),
            ]
        );

        let chunk = compile_source("local a = 1, (nil)()\nreturn a");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Fixed(0))]
        );
    }

    #[test]
    fn activates_local_bindings_only_after_all_initializers() {
        let chunk = compile_source("local value = 7\nlocal value = value\nreturn value");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 7,
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Move {
                    dst: Register(2),
                    src: Register(1),
                },
                Instruction::Return {
                    base: Register(2),
                    values: Count::Fixed(1),
                    ..
                },
            ]
        ));

        let chunk = compile_source("local value <const> = 7\nreturn value");
        assert_eq!(returns(&chunk), vec![(Register(1), Count::Fixed(1))]);
    }

    #[test]
    fn adjusts_varargs_to_local_declaration_width() {
        let chunk = compile_source("local a, b = ...\nlocal c, d = (...)\nreturn a, b, c, d");

        assert_eq!(
            varargs(&chunk),
            vec![
                (Register(0), Count::Fixed(2)),
                (Register(2), Count::Fixed(1)),
            ]
        );
        assert!(
            chunk.entry.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::LoadNil { dst: Register(3) }
            ))
        );
    }

    #[test]
    fn keeps_open_call_arguments_adjacent_to_a_fixed_result_call() {
        let chunk = compile_source("local a, b = (nil)((false)())\nreturn a, b");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(1), Count::Fixed(0), Count::Open),
                (Register(0), Count::Open, Count::Fixed(2)),
            ]
        );
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::Call {
                    base: Register(1),
                    results: Count::Open,
                    ..
                },
                Instruction::Call {
                    base: Register(0),
                    arguments: Count::Open,
                    results: Count::Fixed(2),
                },
            ]
        )));
    }

    #[test]
    fn lowers_parallel_assignment_through_an_isolated_rhs_window() {
        let chunk = compile_source("local a, b = 1, 2\na, b = b, a\nreturn a, b");

        assert_eq!(chunk.entry.max_registers, 4);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 2,
                },
                Instruction::Move {
                    dst: Register(2),
                    src: Register(1),
                },
                Instruction::Move {
                    dst: Register(3),
                    src: Register(0),
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(3),
                },
                Instruction::Move {
                    dst: Register(0),
                    src: Register(2),
                },
                ..
            ]
        ));

        let chunk = compile_source("local a, b = 1, 2\na, b = 3\nreturn a, b");

        assert!(chunk.entry.code.windows(4).any(|instructions| matches!(
            instructions,
            [
                Instruction::LoadSmallInt {
                    dst: Register(2),
                    value: 3,
                },
                Instruction::LoadNil { dst: Register(3) },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(3),
                },
                Instruction::Move {
                    dst: Register(0),
                    src: Register(2),
                },
            ]
        )));

        let chunk = compile_source("local a = 0\na = 1, (nil)()\nreturn a");

        assert_eq!(
            calls(&chunk),
            vec![(Register(2), Count::Fixed(0), Count::Fixed(0))]
        );
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::Call {
                    results: Count::Fixed(0),
                    ..
                },
                Instruction::Move {
                    dst: Register(0),
                    src: Register(1),
                },
            ]
        )));

        let chunk = compile_source("local a = 0\na, a = 1, 2\nreturn a");

        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::Move {
                    dst: Register(0),
                    src: Register(2),
                },
                Instruction::Move {
                    dst: Register(0),
                    src: Register(1),
                },
            ]
        )));
    }

    #[test]
    fn prepares_indexed_targets_before_rhs_and_writes_afterward() {
        let chunk = compile_source("local i, t = 3, {}\ni, t[i] = i + 1, 20\nreturn i, t");

        let code = chunk.entry.code.as_ref();

        let table_capture = code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::Move {
                        dst: Register(2),
                        src: Register(1),
                    }
                )
            })
            .unwrap();

        let key_capture = code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::Move {
                        dst: Register(3),
                        src: Register(0),
                    }
                )
            })
            .unwrap();

        let rhs_add = code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Binary { .. }))
            .unwrap();

        let local_write = code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::Move {
                        dst: Register(0),
                        src: Register(4),
                    }
                )
            })
            .unwrap();

        let indexed_write = code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::SetTable {
                        table: Register(2),
                        key: Register(3),
                        value: Register(5),
                    }
                )
            })
            .unwrap();

        assert!(table_capture < key_capture);
        assert!(key_capture < rhs_add);
        assert!(rhs_add < indexed_write);
        assert!(indexed_write < local_write);
    }

    #[test]
    fn lowers_global_and_upvalue_assignment() {
        let chunk = compile_source("global_name = 7");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::GetUpvalue {
                    dst: Register(0),
                    upvalue,
                },
                Instruction::LoadConst {
                    dst: Register(1),
                    constant,
                },
                Instruction::LoadSmallInt {
                    dst: Register(2),
                    value: 7,
                },
                Instruction::SetTable {
                    table: Register(0),
                    key: Register(1),
                    value: Register(2),
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    ..
                },
            ] if *upvalue == UpvalueIndex::new(0)
                && *constant == ConstantIndex::new(0)
        ));

        let chunk = compile_source("_ENV = nil");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadNil { dst: Register(0) },
                Instruction::SetUpvalue {
                    upvalue,
                    src: Register(0),
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    ..
                },
            ] if *upvalue == UpvalueIndex::new(0)
        ));
    }

    #[test]
    fn reuses_registers_and_restores_outer_bindings_after_do_blocks() {
        let chunk = compile_source("do local inner = 1 end\nlocal after = 2\nreturn after");

        assert_eq!(chunk.entry.max_registers, 2);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 2,
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));

        let chunk = compile_source("local value = 1\ndo local value = 2 end\nreturn value");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 2,
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));

        let names = (0..255)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>();
        let source = format!(
            "local {}\ndo local first = 1 end\ndo local second = 2 end",
            names.join(", ")
        );
        let chunk = compile_source(&source);

        assert_eq!(chunk.entry.max_registers, 256);
    }

    #[test]
    fn a_return_inside_a_do_block_terminates_the_enclosing_function() {
        let chunk = compile_source("do return 1 end\nlocal unreachable = 2");

        assert_eq!(chunk.entry.max_registers, 1);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn marks_close_locals_and_closes_normal_block_fallthrough() {
        let chunk =
            compile_source("local outer = 1\ndo local resource <close> = nil end\nreturn outer");

        assert_eq!(marked_to_close(&chunk), vec![Register(1)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(1)]);
        assert_eq!(
            return_details(&chunk),
            vec![(Register(1), Count::Fixed(1), None)]
        );
        assert_eq!(chunk.entry.max_registers, 2);

        let chunk = compile_source("local plain, resource <close> = (nil)(), nil");
        let call = chunk
            .entry
            .code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Call { .. }))
            .unwrap();
        let second_initializer = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(instruction, Instruction::LoadNil { dst: Register(1) })
            })
            .unwrap();
        let mark = chunk
            .entry
            .code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::MarkToClose { .. }))
            .unwrap();

        assert!(call < second_initializer);
        assert!(second_initializer < mark);

        let chunk =
            compile_source("do local outer <close> = nil do local inner <close> = nil end end");

        assert_eq!(marked_to_close(&chunk), vec![Register(0), Register(1)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(1), Register(0)]);
        assert_eq!(
            return_details(&chunk),
            vec![(Register(0), Count::Fixed(0), None)]
        );
    }

    #[test]
    fn fuses_root_and_explicit_return_cleanup_into_return() {
        let chunk = compile_source("local resource <close> = nil");

        assert_eq!(marked_to_close(&chunk), vec![Register(0)]);
        assert!(standalone_closes(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![(Register(0), Count::Fixed(0), Some(Register(0)),)]
        );

        let chunk = compile_source("local resource <close> = nil\nreturn 7");

        assert!(standalone_closes(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![(Register(1), Count::Fixed(1), Some(Register(0)),)]
        );

        let mark = chunk
            .entry
            .code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::MarkToClose { .. }))
            .unwrap();
        let result = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::LoadSmallInt {
                        dst: Register(1),
                        value: 7,
                    }
                )
            })
            .unwrap();
        let return_instruction = chunk.entry.code.len() - 1;

        assert!(mark < result);
        assert!(result < return_instruction);
    }

    #[test]
    fn keeps_open_returns_adjacent_and_coalesces_nested_cleanup() {
        let chunk =
            compile_source("local outer = 1\ndo local resource <close> = nil return (nil)() end");

        assert!(standalone_closes(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![(Register(2), Count::Open, Some(Register(1)),)]
        );
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::Call {
                    base: Register(2),
                    results: Count::Open,
                    ..
                },
                Instruction::Return {
                    base: Register(2),
                    values: Count::Open,
                    close_from: Some(Register(1)),
                },
            ]
        )));

        let chunk =
            compile_source("local outer <close> = nil\ndo local inner <close> = nil return end");

        assert_eq!(marked_to_close(&chunk), vec![Register(0), Register(1)]);
        assert!(standalone_closes(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![(Register(0), Count::Fixed(0), Some(Register(0)),)]
        );
    }

    #[test]
    fn enforces_exact_width_limits() {
        let names = (0..255)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>();

        let source = format!("local {} = (nil)()", names.join(", "));
        let chunk = compile_source(&source);

        assert_eq!(chunk.entry.max_registers, 255);
        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(0), Count::Fixed(255))]
        );

        let source = format!("local {} = 1 + 2", names.join(", "));
        let chunk = compile_source(&source);
        assert_eq!(chunk.entry.max_registers, 255);

        let source = format!("local prefix = 0\nlocal {} = (nil)()", names.join(", "));
        let chunk = compile_source(&source);
        assert_eq!(chunk.entry.max_registers, 256);
        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Fixed(255))]
        );

        let source = format!(
            "local prefix_a, prefix_b = 0, 0\nlocal {} = (nil)()",
            names.join(", ")
        );
        let error = compile_source_result(&source).unwrap_err();
        assert!(matches!(
            error.kind,
            CompileErrorKind::TooManyRegisters { required: 257 }
        ));

        let names = (0..256)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>();
        let source = format!("local {}", names.join(", "));
        let error = compile_source_result(&source).unwrap_err();
        assert!(matches!(error.kind, CompileErrorKind::TooManyResults));

        let targets = vec!["global_name"; 256].join(", ");
        let source = format!("{targets} = nil");
        let error = compile_source_result(&source).unwrap_err();
        assert!(matches!(error.kind, CompileErrorKind::TooManyResults));
    }

    #[test]
    fn lowers_closures_to_indexed_child_prototypes() {
        let chunk = compile_source(
            "return function() return 7 end, function(value, ...) return value, ... end",
        );

        assert_eq!(
            closures(&chunk.entry),
            vec![
                (Register(0), PrototypeIndex::new(0)),
                (Register(1), PrototypeIndex::new(1)),
            ]
        );
        assert_eq!(
            return_details(&chunk),
            vec![(Register(0), Count::Fixed(2), None)]
        );
        assert_eq!(chunk.entry.children.len(), 2);

        let first = &chunk.entry.children[0];
        assert_eq!(first.parameter_count, 0);
        assert!(!first.is_vararg);
        assert!(first.upvalues.is_empty());
        assert_eq!(
            prototype_returns(first),
            vec![(Register(0), Count::Fixed(1))]
        );

        let second = &chunk.entry.children[1];
        assert_eq!(second.parameter_count, 1);
        assert!(second.is_vararg);
        assert!(second.upvalues.is_empty());
        assert!(matches!(
            second.code.as_ref(),
            [
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Vararg {
                    base: Register(2),
                    results: Count::Open,
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Open,
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn captures_parent_registers_and_forwards_parent_upvalue_cells() {
        let chunk = compile_source(
            "local value = 1\nreturn function() return function() return value end end",
        );

        assert_eq!(chunk.entry.children.len(), 1);
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(0))]
        ));
        assert_eq!(
            return_details(&chunk),
            vec![(Register(1), Count::Fixed(1), Some(Register(0)))]
        );

        let middle = &chunk.entry.children[0];
        assert_eq!(
            closures(middle),
            vec![(Register(0), PrototypeIndex::new(0))]
        );
        assert_eq!(middle.children.len(), 1);
        assert!(matches!(
            middle.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentUpvalue(upvalue)]
                if *upvalue == UpvalueIndex::new(0)
        ));

        let inner = &middle.children[0];
        assert!(matches!(
            inner.code.as_ref(),
            [
                Instruction::GetUpvalue {
                    dst: Register(0),
                    upvalue,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ] if *upvalue == UpvalueIndex::new(0)
        ));
    }

    #[test]
    fn preserves_first_capture_order_and_deduplicates_upvalues() {
        let chunk = compile_source(
            "local first, second = 1, 2\nreturn function() return second, first, second end",
        );

        let child = &chunk.entry.children[0];
        assert!(matches!(
            child.upvalues.as_ref(),
            [
                UpvalueDescriptor::ParentRegister(Register(1)),
                UpvalueDescriptor::ParentRegister(Register(0)),
            ]
        ));
        assert!(matches!(
            child.code.as_ref(),
            [
                Instruction::GetUpvalue {
                    dst: Register(0),
                    upvalue: first_read,
                },
                Instruction::GetUpvalue {
                    dst: Register(1),
                    upvalue: second_read,
                },
                Instruction::GetUpvalue {
                    dst: Register(2),
                    upvalue: repeated_read,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(3),
                    close_from: None,
                },
            ] if *first_read == UpvalueIndex::new(0)
                && *second_read == UpvalueIndex::new(1)
                && *repeated_read == UpvalueIndex::new(0)
        ));
    }

    #[test]
    fn compiles_recursive_local_function_captures_from_reserved_registers() {
        let chunk = compile_source("local function recurse() return recurse end\nreturn recurse");

        assert_eq!(chunk.entry.children.len(), 1);
        assert_eq!(
            closures(&chunk.entry),
            vec![(Register(0), PrototypeIndex::new(0))]
        );
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(0))]
        ));
        assert!(matches!(
            chunk.entry.children[0].code.as_ref(),
            [
                Instruction::GetUpvalue {
                    dst: Register(0),
                    upvalue,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ] if *upvalue == UpvalueIndex::new(0)
        ));
    }

    #[test]
    fn closes_reused_parent_registers_between_sibling_captures() {
        let chunk = compile_source(
            "local first, second\n\
             do local value = 1 first = function() return value end end\n\
             do local value = 2 second = function() return value end end\n\
             return first, second",
        );

        assert_eq!(chunk.entry.children.len(), 2);
        assert_eq!(
            closures(&chunk.entry),
            vec![
                (Register(3), PrototypeIndex::new(0)),
                (Register(3), PrototypeIndex::new(1)),
            ]
        );
        assert_eq!(standalone_closes(&chunk), vec![Register(2), Register(2)]);

        for child in &chunk.entry.children {
            assert!(matches!(
                child.upvalues.as_ref(),
                [UpvalueDescriptor::ParentRegister(Register(2))]
            ));
        }
    }

    #[test]
    fn lowers_closures_in_fixed_and_discarded_result_contexts() {
        let chunk = compile_source("local first, second = function() return 1 end");

        assert_eq!(chunk.entry.children.len(), 1);
        assert_eq!(
            closures(&chunk.entry),
            vec![(Register(0), PrototypeIndex::new(0))]
        );
        assert!(
            chunk.entry.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::LoadNil { dst: Register(1) }
            ))
        );

        let chunk = compile_source("local kept = 1, function() return 2 end");

        assert_eq!(chunk.entry.children.len(), 1);
        assert_eq!(
            closures(&chunk.entry),
            vec![(Register(1), PrototypeIndex::new(0))]
        );
    }

    #[test]
    fn omits_unreachable_child_prototypes_with_unassigned_captures() {
        let chunk = compile_source(
            "do return end\nlocal value = 1\nlocal closure = function() return value end",
        );

        assert!(chunk.entry.children.is_empty());
        assert!(closures(&chunk.entry).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![(Register(0), Count::Fixed(0), Some(Register(0)))]
        );
    }

    #[test]
    fn lowers_if_elseif_else_with_patched_offsets() {
        let chunk = compile_source(
            "local result\n\
             if false then result = 1\n\
             elseif true then result = 2\n\
             else result = 3 end\n\
             return result",
        );

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadNil { dst: Register(0) },
                Instruction::LoadBool {
                    dst: Register(1),
                    value: false,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(1),
                    offset: 3,
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 1,
                },
                Instruction::Move {
                    dst: Register(0),
                    src: Register(1),
                },
                Instruction::Jump { offset: 7 },
                Instruction::LoadBool {
                    dst: Register(1),
                    value: true,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(1),
                    offset: 3,
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 2,
                },
                Instruction::Move {
                    dst: Register(0),
                    src: Register(1),
                },
                Instruction::Jump { offset: 2 },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 3,
                },
                Instruction::Move {
                    dst: Register(0),
                    src: Register(1),
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn lowers_while_with_a_backward_jump_and_reuses_the_condition_register() {
        let chunk = compile_source("while false do local body = 1 end\nreturn 2");

        assert_eq!(chunk.entry.max_registers, 1);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadBool {
                    dst: Register(0),
                    value: false,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(0),
                    offset: 2,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::Jump { offset: -4 },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 2,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn break_jumps_to_the_loop_exit_and_suppresses_the_backedge() {
        let chunk = compile_source("while true do break end");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadBool {
                    dst: Register(0),
                    value: true,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(0),
                    offset: 1,
                },
                Instruction::Jump { offset: 0 },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    close_from: None,
                },
            ]
        ));

        let chunk = compile_source("while (nil)() do break end");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Fixed(1))]
        );
        assert!(chunk.entry.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::JumpIfFalsy {
                condition: Register(0),
                ..
            }
        )));
        assert!(jump_offsets(&chunk).iter().all(|offset| *offset >= 0));
    }

    #[test]
    fn while_falls_through_even_when_its_body_returns() {
        let chunk = compile_source("while true do return 1 end\nreturn 2");

        assert_eq!(
            returns(&chunk),
            vec![
                (Register(0), Count::Fixed(1)),
                (Register(0), Count::Fixed(1)),
            ]
        );
        assert!(jump_offsets(&chunk).is_empty());
    }

    #[test]
    fn closes_while_scopes_on_backedge_break_and_return() {
        let chunk = compile_source("while true do local resource <close> = nil end");

        assert_eq!(marked_to_close(&chunk), vec![Register(0)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(0)]);
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::CloseFrom { base: Register(0) },
                Instruction::Jump { offset },
            ] if *offset < 0
        )));

        let chunk = compile_source(
            "while true do\n\
                 local outer <close> = nil\n\
                 do local inner <close> = nil break end\n\
             end\n\
             return 1",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(0), Register(1)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(0)]);
        assert!(jump_offsets(&chunk).iter().all(|offset| *offset >= 0));

        let chunk = compile_source("while true do local resource <close> = nil return 1 end");

        assert!(standalone_closes(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![
                (Register(1), Count::Fixed(1), Some(Register(0))),
                (Register(0), Count::Fixed(0), None),
            ]
        );
    }

    #[test]
    fn nested_breaks_target_the_innermost_active_loop() {
        let chunk = compile_source(
            "while true do\n\
                 local outer <close> = nil\n\
                 while true do\n\
                     local inner <close> = nil\n\
                     break\n\
                 end\n\
                 break\n\
             end",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(0), Register(1)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(1), Register(0)]);
        assert!(jump_offsets(&chunk).iter().all(|offset| *offset >= 0));
    }

    #[test]
    fn a_conditional_break_keeps_the_remaining_body_reachable() {
        let chunk = compile_source(
            "while true do\n\
                 if false then break end\n\
                 local after = 1\n\
             end\n\
             return 2",
        );

        assert!(
            chunk.entry.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::LoadSmallInt { value: 1, .. }
            ))
        );
        assert!(jump_offsets(&chunk).iter().any(|offset| *offset < 0));
    }

    #[test]
    fn closes_captured_iteration_locals_before_backedge_and_register_reuse() {
        let chunk = compile_source(
            "local escaped\n\
             while false do\n\
                 local captured = 1\n\
                 escaped = function() return captured end\n\
             end\n\
             local reused = 2\n\
             return escaped, reused",
        );

        assert_eq!(chunk.entry.children.len(), 1);
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(1))]
        ));
        assert_eq!(standalone_closes(&chunk), vec![Register(1)]);

        let close = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(instruction, Instruction::CloseFrom { base: Register(1) })
            })
            .unwrap();

        let backedge = chunk
            .entry
            .code
            .iter()
            .position(
                |instruction| matches!(instruction, Instruction::Jump { offset } if *offset < 0),
            )
            .unwrap();

        let reuse = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::LoadSmallInt {
                        dst: Register(1),
                        value: 2,
                    }
                )
            })
            .unwrap();

        assert!(close < backedge);
        assert!(backedge < reuse);
    }

    #[test]
    fn lowers_repeat_body_before_condition_and_reuses_the_body_register_after_exit() {
        let chunk = compile_source("repeat local body = 1 until false\nreturn 2");

        assert_eq!(chunk.entry.max_registers, 2);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadBool {
                    dst: Register(1),
                    value: false,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(1),
                    offset: -3,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 2,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn repeat_condition_reads_body_locals_before_scope_exit() {
        let chunk = compile_source("repeat local visible = 1 until visible == 1");

        assert_eq!(chunk.entry.max_registers, 4);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::Move {
                    dst: Register(2),
                    src: Register(0),
                },
                Instruction::LoadSmallInt {
                    dst: Register(3),
                    value: 1,
                },
                Instruction::Binary {
                    op: BytecodeBinaryOp::Equal,
                    dst: Register(1),
                    left: Register(2),
                    right: Register(3),
                },
                Instruction::JumpIfFalsy {
                    condition: Register(1),
                    offset: -5,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn repeat_break_skips_the_condition_and_reaches_the_continuation() {
        let chunk = compile_source("repeat break until (nil)()\nreturn 2");

        assert!(calls(&chunk).is_empty());
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::Jump { offset: 0 },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 2,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));

        let chunk = compile_source("repeat until (nil)()");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Fixed(1))]
        );
        assert!(chunk.entry.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::JumpIfFalsy { offset, .. } if *offset < 0
        )));
    }

    #[test]
    fn repeat_distinguishes_terminal_return_from_break_fallthrough() {
        let chunk = compile_source("repeat return 1 until true\nreturn 2");

        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(1))]);
        assert!(chunk.entry.code.iter().all(|instruction| !matches!(
            instruction,
            Instruction::LoadBool { .. }
                | Instruction::LoadSmallInt { value: 2, .. }
                | Instruction::JumpIfFalsy { .. }
        )));

        let chunk = compile_source(
            "repeat\n\
                 if true then break else return 1 end\n\
             until 99\n\
             return 2",
        );

        assert_eq!(
            returns(&chunk),
            vec![
                (Register(0), Count::Fixed(1)),
                (Register(0), Count::Fixed(1)),
            ]
        );
        assert!(chunk.entry.code.iter().all(|instruction| !matches!(
            instruction,
            Instruction::LoadSmallInt { value: 99, .. }
        )));
        assert!(
            chunk.entry.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::LoadSmallInt { value: 2, .. }
            ))
        );
    }

    #[test]
    fn closes_repeat_scopes_on_condition_break_and_return() {
        let chunk = compile_source("repeat local resource <close> = nil until false");

        assert_eq!(marked_to_close(&chunk), vec![Register(0)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(0), Register(0)]);
        assert!(chunk.entry.code.windows(3).any(|instructions| matches!(
            instructions,
            [
                Instruction::JumpIfFalsy {
                    condition: Register(1),
                    offset: 2,
                },
                Instruction::CloseFrom { base: Register(0) },
                Instruction::Jump { offset },
            ] if *offset > 0
        )));
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::CloseFrom { base: Register(0) },
                Instruction::Jump { offset },
            ] if *offset < 0
        )));

        let chunk = compile_source("repeat local resource <close> = nil break until false");

        assert_eq!(marked_to_close(&chunk), vec![Register(0)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(0)]);
        assert!(
            chunk
                .entry
                .code
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::JumpIfFalsy { .. }))
        );
        assert!(jump_offsets(&chunk).iter().all(|offset| *offset >= 0));

        let chunk = compile_source("repeat local resource <close> = nil return 1 until false");

        assert!(standalone_closes(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![(Register(1), Count::Fixed(1), Some(Register(0)))]
        );
    }

    #[test]
    fn a_conditional_repeat_break_has_separate_cleanup_paths() {
        let chunk = compile_source(
            "repeat\n\
                 local resource <close> = nil\n\
                 if false then break end\n\
             until false",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(0)]);
        assert_eq!(
            standalone_closes(&chunk),
            vec![Register(0), Register(0), Register(0)]
        );
        assert!(jump_offsets(&chunk).iter().any(|offset| *offset < 0));
    }

    #[test]
    fn nested_repeat_breaks_target_the_innermost_active_loop() {
        let chunk = compile_source(
            "repeat\n\
                 local outer <close> = nil\n\
                 repeat\n\
                     local inner <close> = nil\n\
                     break\n\
                 until false\n\
                 break\n\
             until false",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(0), Register(1)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(1), Register(0)]);
        assert!(
            chunk
                .entry
                .code
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::JumpIfFalsy { .. }))
        );
        assert!(jump_offsets(&chunk).iter().all(|offset| *offset >= 0));

        let chunk = compile_source(
            "repeat\n\
                 while true do break end\n\
                 return\n\
             until false\n\
             local unreachable = 1",
        );

        assert_eq!(returns(&chunk), vec![(Register(0), Count::Fixed(0))]);
        assert!(
            chunk.entry.code.iter().all(|instruction| !matches!(
                instruction,
                Instruction::LoadSmallInt { value: 1, .. }
            ))
        );
    }

    #[test]
    fn closes_captured_repeat_locals_before_backedge_and_register_reuse() {
        let chunk = compile_source(
            "local escaped\n\
             repeat\n\
                 local captured = 1\n\
                 escaped = function() return captured end\n\
             until false\n\
             local reused = 2\n\
             return escaped, reused",
        );

        assert_eq!(chunk.entry.children.len(), 1);
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(1))]
        ));
        assert_eq!(standalone_closes(&chunk), vec![Register(1), Register(1)]);

        let close = chunk
            .entry
            .code
            .iter()
            .rposition(|instruction| {
                matches!(instruction, Instruction::CloseFrom { base: Register(1) })
            })
            .unwrap();
        let backedge = chunk
            .entry
            .code
            .iter()
            .position(
                |instruction| matches!(instruction, Instruction::Jump { offset } if *offset < 0),
            )
            .unwrap();
        let reuse = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::LoadSmallInt {
                        dst: Register(1),
                        value: 2,
                    }
                )
            })
            .unwrap();

        assert!(close < backedge);
        assert!(backedge < reuse);
    }

    #[test]
    fn terminal_repeat_omits_its_unreachable_condition_closure() {
        let chunk = compile_source("repeat return until function() end");

        assert!(chunk.entry.children.is_empty());
        assert!(closures(&chunk.entry).is_empty());
    }

    #[test]
    fn lowers_numeric_for_frame_default_step_assignment_and_backedge() {
        let chunk = compile_source("for i = 1, 2 do local seen = i end\nreturn 3");

        assert_eq!(chunk.entry.max_registers, 5);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 2,
                },
                Instruction::LoadSmallInt {
                    dst: Register(2),
                    value: 1,
                },
                Instruction::ForPrep {
                    base: Register(0),
                    exit_offset: 2,
                },
                Instruction::Move {
                    dst: Register(4),
                    src: Register(3),
                },
                Instruction::ForLoop {
                    base: Register(0),
                    body_offset: -2,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 3,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));

        let chunk = compile_source("for i = 1, 5, 2 do break end");

        assert!(chunk.entry.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::LoadSmallInt {
                dst: Register(2),
                value: 2,
            }
        )));
        assert!(for_loops(&chunk).is_empty());

        let chunk = compile_source("for i = 1, 2 do i = 99 end");

        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::LoadSmallInt {
                    dst: Register(4),
                    value: 99,
                },
                Instruction::Move {
                    dst: Register(3),
                    src: Register(4),
                },
            ]
        )));
        assert_eq!(for_loops(&chunk), vec![(Register(0), -3)]);
    }

    #[test]
    fn numeric_for_controls_are_evaluated_once_in_the_parent_scope() {
        let chunk = compile_source("for i = (nil)(1), (nil)(2), (nil)(3) do break end");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(3), Count::Fixed(1), Count::Fixed(1)),
                (Register(3), Count::Fixed(1), Count::Fixed(1)),
                (Register(3), Count::Fixed(1), Count::Fixed(1)),
            ]
        );
        assert_eq!(
            chunk
                .entry
                .code
                .iter()
                .filter_map(|instruction| match instruction {
                    Instruction::LoadSmallInt {
                        dst: Register(4),
                        value,
                    } => Some(*value),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let last_call = chunk
            .entry
            .code
            .iter()
            .rposition(|instruction| matches!(instruction, Instruction::Call { .. }))
            .unwrap();
        let prep = chunk
            .entry
            .code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::ForPrep { .. }))
            .unwrap();

        assert!(last_call < prep);

        let chunk = compile_source("local i = 9\nfor i = i, i do return i end\nreturn i");

        assert_eq!(chunk.entry.max_registers, 6);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 9,
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Move {
                    dst: Register(2),
                    src: Register(0),
                },
                Instruction::LoadSmallInt {
                    dst: Register(3),
                    value: 1,
                },
                Instruction::ForPrep {
                    base: Register(1),
                    exit_offset: 2,
                },
                Instruction::Move {
                    dst: Register(5),
                    src: Register(4),
                },
                Instruction::Return {
                    base: Register(5),
                    values: Count::Fixed(1),
                    close_from: None,
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));
        assert!(for_loops(&chunk).is_empty());
    }

    #[test]
    fn closes_numeric_for_scopes_on_backedge_break_and_return() {
        let chunk = compile_source("for i = 1, 2 do local resource <close> = nil end");

        assert_eq!(marked_to_close(&chunk), vec![Register(4)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(3)]);
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::CloseFrom { base: Register(3) },
                Instruction::ForLoop {
                    base: Register(0),
                    body_offset,
                },
            ] if *body_offset < 0
        )));

        let chunk = compile_source("for i = 1, 2 do local resource <close> = nil break end");

        assert_eq!(marked_to_close(&chunk), vec![Register(4)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(3)]);
        assert!(for_loops(&chunk).is_empty());
        assert!(jump_offsets(&chunk).iter().all(|offset| *offset >= 0));

        let chunk =
            compile_source("for i = 1, 2 do local resource <close> = nil return 1 end\nreturn 2");

        assert!(standalone_closes(&chunk).is_empty());
        assert!(for_loops(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![
                (Register(5), Count::Fixed(1), Some(Register(3))),
                (Register(0), Count::Fixed(1), None),
            ]
        );
    }

    #[test]
    fn closes_captured_numeric_for_variables_before_backedge_and_register_reuse() {
        let chunk = compile_source(
            "local escaped\n\
             for i = 1, 2 do\n\
                 escaped = function() return i end\n\
             end\n\
             local reused = 3\n\
             return escaped, reused",
        );

        assert_eq!(chunk.entry.children.len(), 1);
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(4))]
        ));
        assert_eq!(standalone_closes(&chunk), vec![Register(4)]);
        assert_eq!(for_loops(&chunk).len(), 1);

        let close = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(instruction, Instruction::CloseFrom { base: Register(4) })
            })
            .unwrap();
        let backedge = chunk
            .entry
            .code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::ForLoop { .. }))
            .unwrap();
        let reuse = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::LoadSmallInt {
                        dst: Register(1),
                        value: 3,
                    }
                )
            })
            .unwrap();

        assert!(close < backedge);
        assert!(backedge < reuse);
    }

    #[test]
    fn nested_numeric_breaks_preserve_outer_loop_state() {
        let chunk = compile_source(
            "for outer = 1, 2 do\n\
                 local outer_resource <close> = nil\n\
                 for inner = 1, 2 do\n\
                     local inner_resource <close> = nil\n\
                     break\n\
                 end\n\
                 break\n\
             end",
        );

        assert_eq!(for_preps(&chunk), vec![(Register(0), 12), (Register(5), 4)]);
        assert!(for_loops(&chunk).is_empty());
        assert_eq!(marked_to_close(&chunk), vec![Register(4), Register(9)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(8), Register(3)]);
    }

    #[test]
    fn numeric_for_frame_respects_the_register_limit() {
        let names = (0..252)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>();
        let source = format!("local {}\nfor i = 1, 1 do end", names.join(", "));
        let chunk = compile_source(&source);

        assert_eq!(chunk.entry.max_registers, 256);
        assert_eq!(for_preps(&chunk), vec![(Register(252), 1)]);
        assert_eq!(for_loops(&chunk), vec![(Register(252), -1)]);

        let names = (0..253)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>();
        let source = format!("local {}\nfor i = 1, 1 do end", names.join(", "));
        let error = compile_source_result(&source).unwrap_err();

        assert!(matches!(
            error.kind,
            CompileErrorKind::TooManyRegisters { required: 257 }
        ));
    }

    #[test]
    fn lowers_generic_for_frame_initial_call_and_backedge() {
        let chunk = compile_source("for key, value in nil do local seen = key end\nreturn 1");

        assert_eq!(chunk.entry.max_registers, 7);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadNil { dst: Register(0) },
                Instruction::LoadNil { dst: Register(1) },
                Instruction::LoadNil { dst: Register(2) },
                Instruction::LoadNil { dst: Register(3) },
                Instruction::MarkToClose {
                    register: Register(3),
                },
                Instruction::Jump { offset: 1 },
                Instruction::Move {
                    dst: Register(6),
                    src: Register(4),
                },
                Instruction::TForCall {
                    base: Register(0),
                    variables: 2,
                },
                Instruction::TForLoop {
                    base: Register(0),
                    body_offset: -3,
                },
                Instruction::CloseFrom { base: Register(3) },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn generic_for_adjusts_controls_to_four_values_and_keeps_variables_separate() {
        let chunk = compile_source("for first, second in (nil)() do break end");

        assert_eq!(
            calls(&chunk),
            vec![(Register(0), Count::Fixed(0), Count::Fixed(4))]
        );
        assert_eq!(marked_to_close(&chunk), vec![Register(3)]);
        assert_eq!(tfor_calls(&chunk), vec![(Register(0), 2)]);
        assert_eq!(tfor_loops(&chunk), vec![(Register(0), -4)]);

        let chunk = compile_source("for item in (nil)(), (nil)() do break end");

        assert_eq!(
            calls(&chunk),
            vec![
                (Register(0), Count::Fixed(0), Count::Fixed(1)),
                (Register(1), Count::Fixed(0), Count::Fixed(3)),
            ]
        );
        assert_eq!(marked_to_close(&chunk), vec![Register(3)]);

        let chunk = compile_source("for item in ... do break end");

        assert_eq!(varargs(&chunk), vec![(Register(0), Count::Fixed(4))]);
        assert_eq!(marked_to_close(&chunk), vec![Register(3)]);

        let chunk = compile_source("for item in 1, 2, 3, 4, (nil)() do break end");

        assert_eq!(
            calls(&chunk),
            vec![(Register(4), Count::Fixed(0), Count::Fixed(0))]
        );
        assert!(chunk.entry.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::LoadSmallInt {
                dst: Register(3),
                value: 4,
            }
        )));
        assert_eq!(marked_to_close(&chunk), vec![Register(3)]);

        let chunk = compile_source(
            "local first = 9\n\
             for first, second in nil do\n\
                 first = 1\n\
                 second = 2\n\
             end\n\
             return first",
        );

        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::LoadSmallInt {
                    dst: Register(7),
                    value: 1,
                },
                Instruction::Move {
                    dst: Register(5),
                    src: Register(7),
                },
            ]
        )));
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::LoadSmallInt {
                    dst: Register(7),
                    value: 2,
                },
                Instruction::Move {
                    dst: Register(6),
                    src: Register(7),
                },
            ]
        )));
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                ..,
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0)
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Fixed(1),
                    close_from: None
                }
            ]
        ));
    }

    #[test]
    fn closes_generic_for_values_on_exhaustion_break_and_return() {
        let chunk = compile_source("for value in nil do local resource <close> = nil end");

        assert_eq!(marked_to_close(&chunk), vec![Register(3), Register(5)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(4), Register(3)]);
        assert!(chunk.entry.code.windows(3).any(|instructions| matches!(
            instructions,
            [
                Instruction::CloseFrom { base: Register(4) },
                Instruction::TForCall {
                    base: Register(0),
                    variables: 1,
                },
                Instruction::TForLoop {
                    base: Register(0),
                    body_offset,
                },
            ] if *body_offset < 0
        )));

        let chunk = compile_source("for value in nil do local resource <close> = nil break end");

        assert_eq!(marked_to_close(&chunk), vec![Register(3), Register(5)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(3), Register(3)]);
        assert_eq!(tfor_calls(&chunk), vec![(Register(0), 1)]);
        assert_eq!(tfor_loops(&chunk).len(), 1);

        let chunk = compile_source(
            "for value in nil do\n\
                 local resource <close> = nil\n\
                 if true then break end\n\
             end",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(3), Register(5)]);
        assert_eq!(
            standalone_closes(&chunk),
            vec![Register(3), Register(4), Register(3)]
        );

        let chunk = compile_source(
            "for value in nil do local resource <close> = nil return 1 end\nreturn 2",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(3), Register(5)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(3)]);
        assert_eq!(
            return_details(&chunk),
            vec![
                (Register(6), Count::Fixed(1), Some(Register(3))),
                (Register(0), Count::Fixed(1), None),
            ]
        );

        let chunk = compile_source("for outer in nil do while true do break end end");

        assert_eq!(standalone_closes(&chunk), vec![Register(3)]);
        assert_eq!(tfor_calls(&chunk), vec![(Register(0), 1)]);
    }

    #[test]
    fn closes_captured_generic_for_variables_before_backedge_and_reuses_the_frame() {
        let chunk = compile_source(
            "local escaped\n\
             for key, value in nil do\n\
                 escaped = function() return key, value end\n\
             end\n\
             local reused = 3\n\
             return escaped, reused",
        );

        assert_eq!(chunk.entry.children.len(), 1);
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [
                UpvalueDescriptor::ParentRegister(Register(5)),
                UpvalueDescriptor::ParentRegister(Register(6)),
            ]
        ));
        assert_eq!(standalone_closes(&chunk), vec![Register(5), Register(4)]);
        assert_eq!(tfor_calls(&chunk), vec![(Register(1), 2)]);
        assert_eq!(tfor_loops(&chunk).len(), 1);

        let close = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(instruction, Instruction::CloseFrom { base: Register(5) })
            })
            .unwrap();
        let backedge = chunk
            .entry
            .code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::TForLoop { .. }))
            .unwrap();
        let reuse = chunk
            .entry
            .code
            .iter()
            .position(|instruction| {
                matches!(
                    instruction,
                    Instruction::LoadSmallInt {
                        dst: Register(1),
                        value: 3,
                    }
                )
            })
            .unwrap();

        assert!(close < backedge);
        assert!(backedge < reuse);
    }

    #[test]
    fn nested_generic_return_closes_from_the_outermost_hidden_closer() {
        let chunk = compile_source(
            "for outer in nil do\n\
                 for inner in nil do\n\
                     return 1\n\
                 end\n\
             end\n\
             return 2",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(3), Register(8)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(8), Register(3)]);
        assert_eq!(
            return_details(&chunk),
            vec![
                (Register(10), Count::Fixed(1), Some(Register(3))),
                (Register(0), Count::Fixed(1), None),
            ]
        );
        assert_eq!(tfor_calls(&chunk), vec![(Register(5), 1), (Register(0), 1)]);
    }

    #[test]
    fn generic_for_frame_and_variable_count_respect_bytecode_limits() {
        let names = (0..251)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>();
        let source = format!("local {}\nfor item in (nil)() do end", names.join(", "));
        let chunk = compile_source(&source);

        assert_eq!(chunk.entry.max_registers, 256);
        assert_eq!(
            calls(&chunk),
            vec![(Register(251), Count::Fixed(0), Count::Fixed(4))]
        );
        assert_eq!(tfor_calls(&chunk), vec![(Register(251), 1)]);
        assert_eq!(tfor_loops(&chunk), vec![(Register(251), -2)]);

        let names = (0..252)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>();
        let source = format!("local {}\nfor item in nil do end", names.join(", "));
        let error = compile_source_result(&source).unwrap_err();

        assert!(matches!(
            error.kind,
            CompileErrorKind::TooManyRegisters { required: 257 }
        ));

        let variables = (0..256)
            .map(|index| format!("item{index}"))
            .collect::<Vec<_>>();
        let source = format!("for {} in nil do end", variables.join(", "));
        let error = compile_source_result(&source).unwrap_err();

        assert!(matches!(error.kind, CompileErrorKind::TooManyResults));
    }

    #[test]
    fn lowers_forward_backward_shadowed_and_cyclic_gotos() {
        let chunk = compile_source(
            "goto target\n\
             local skipped = function() return 1 end\n\
             ::target::",
        );

        assert!(chunk.entry.children.is_empty());
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::Jump { offset: 0 },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    close_from: None,
                },
            ]
        ));

        let chunk = compile_source("::again:: local value = 1 goto again");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::Jump { offset: -2 },
            ]
        ));

        let chunk = compile_source("::same:: do goto same; ::same:: end");

        assert_eq!(jump_offsets(&chunk), vec![0]);

        let chunk = compile_source(
            "goto second\n\
             ::first::\n\
             (nil)()\n\
             goto first\n\
             ::second::\n\
             goto first",
        );

        assert_eq!(calls(&chunk).len(), 1);
        assert_eq!(jump_offsets(&chunk), vec![3, -3, -4]);
        assert!(returns(&chunk).is_empty());

        let chunk = compile_source("::outside:: repeat goto outside until (nil)()");

        assert!(calls(&chunk).is_empty());
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [Instruction::Jump { offset: -1 }]
        ));
    }

    #[test]
    fn closes_trailing_label_locals_once_on_each_incoming_edge() {
        let chunk = compile_source("local resource <close> = nil ::done::");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadNil { dst: Register(0) },
                Instruction::MarkToClose {
                    register: Register(0),
                },
                Instruction::CloseFrom { base: Register(0) },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    close_from: None,
                },
            ]
        ));

        let chunk = compile_source("local resource <close> = nil goto done ::done::");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadNil { dst: Register(0) },
                Instruction::MarkToClose {
                    register: Register(0),
                },
                Instruction::CloseFrom { base: Register(0) },
                Instruction::Jump { offset: 0 },
                Instruction::Return {
                    base: Register(0),
                    values: Count::Fixed(0),
                    close_from: None,
                },
            ]
        ));

        let chunk = compile_source(
            "local resource <close> = nil\n\
             if true then goto done end\n\
             ::done::",
        );

        assert_eq!(standalone_closes(&chunk), vec![Register(0), Register(0)]);
        assert!(jump_offsets(&chunk).contains(&1));
        assert!(matches!(
            chunk.entry.code.last(),
            Some(Instruction::Return {
                values: Count::Fixed(0),
                close_from: None,
                ..
            })
        ));

        let chunk = compile_source(
            "do local before = 1 ::done:: end\n\
             local after = 2\n\
             return after",
        );

        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 2,
                },
            ]
        )));
    }

    #[test]
    fn goto_closes_captured_local_suffixes_and_exited_scopes() {
        let chunk = compile_source(
            "::again::\n\
             local captured = 1\n\
             local closure = function() return captured end\n\
             goto again",
        );

        assert_eq!(chunk.entry.children.len(), 1);
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(0))]
        ));
        assert_eq!(standalone_closes(&chunk), vec![Register(0)]);
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::CloseFrom { base: Register(0) },
                Instruction::Jump { offset },
            ] if *offset < 0
        )));

        let chunk = compile_source(
            "::outside::\n\
             do\n\
                 local resource <close> = nil\n\
                 goto outside\n\
             end",
        );

        assert_eq!(marked_to_close(&chunk), vec![Register(0)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(0)]);
        assert!(jump_offsets(&chunk).iter().all(|offset| *offset < 0));
    }

    #[test]
    fn goto_preserves_or_closes_generic_for_frames_at_their_scope_boundary() {
        let chunk = compile_source("for value in nil do goto continue ::continue:: end");

        assert_eq!(standalone_closes(&chunk), vec![Register(3)]);
        assert_eq!(tfor_calls(&chunk), vec![(Register(0), 1)]);
        assert_eq!(tfor_loops(&chunk).len(), 1);

        let chunk = compile_source(
            "local escaped\n\
             for value in nil do\n\
                 escaped = function() return value end\n\
                 goto continue\n\
                 ::continue::\n\
             end",
        );

        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(5))]
        ));
        assert_eq!(standalone_closes(&chunk), vec![Register(5), Register(4)]);

        let chunk = compile_source(
            "for outer in nil do\n\
                 ::again::\n\
                 for inner in nil do\n\
                     goto again\n\
                 end\n\
             end",
        );

        assert_eq!(
            standalone_closes(&chunk),
            vec![Register(8), Register(8), Register(3)]
        );
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::CloseFrom { base: Register(8) },
                Instruction::Jump { offset },
            ] if *offset < 0
        )));

        let chunk = compile_source(
            "::outside::\n\
             for outer in nil do\n\
                 for inner in nil do\n\
                     goto outside\n\
                 end\n\
             end",
        );

        assert_eq!(
            standalone_closes(&chunk),
            vec![Register(3), Register(8), Register(3)]
        );
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::CloseFrom { base: Register(3) },
                Instruction::Jump { offset },
            ] if *offset < 0
        )));
    }

    #[test]
    fn reuses_condition_and_branch_registers_and_restores_outer_bindings() {
        let chunk =
            compile_source("if true then local branch = 1 end\nlocal after = 2\nreturn after");

        assert_eq!(chunk.entry.max_registers, 2);
        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadBool {
                    dst: Register(0),
                    value: true,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(0),
                    offset: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 2,
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));

        let chunk =
            compile_source("local value = 1\nif true then local value = 2 end\nreturn value");

        assert!(matches!(
            chunk.entry.code.as_ref(),
            [
                Instruction::LoadSmallInt {
                    dst: Register(0),
                    value: 1,
                },
                Instruction::LoadBool {
                    dst: Register(1),
                    value: true,
                },
                Instruction::JumpIfFalsy {
                    condition: Register(1),
                    offset: 1,
                },
                Instruction::LoadSmallInt {
                    dst: Register(1),
                    value: 2,
                },
                Instruction::Move {
                    dst: Register(1),
                    src: Register(0),
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Fixed(1),
                    close_from: None,
                },
            ]
        ));
    }

    #[test]
    fn propagates_if_termination_only_when_every_path_returns() {
        let chunk = compile_source(
            "if true then return 1 elseif false then return 2 else return 3 end\n\
             local unreachable = 4",
        );

        assert_eq!(
            returns(&chunk),
            vec![
                (Register(0), Count::Fixed(1)),
                (Register(0), Count::Fixed(1)),
                (Register(0), Count::Fixed(1)),
            ]
        );
        assert!(
            !chunk.entry.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::LoadSmallInt { value: 4, .. }
            ))
        );

        let chunk = compile_source("if true then return 1 end");

        assert_eq!(
            returns(&chunk),
            vec![
                (Register(0), Count::Fixed(1)),
                (Register(0), Count::Fixed(0)),
            ]
        );
    }

    #[test]
    fn terminal_if_omits_unreachable_child_prototypes() {
        let chunk = compile_source(
            "if true then return else return end\n\
             local captured\n\
             local function skipped() return captured end",
        );

        assert!(chunk.entry.children.is_empty());
        assert!(closures(&chunk.entry).is_empty());
        assert_eq!(
            returns(&chunk),
            vec![
                (Register(0), Count::Fixed(0)),
                (Register(0), Count::Fixed(0)),
            ]
        );
    }

    #[test]
    fn closes_if_branch_scopes_on_fallthrough_and_return() {
        let chunk = compile_source("if true then local resource <close> = nil end");

        assert_eq!(marked_to_close(&chunk), vec![Register(0)]);
        assert_eq!(standalone_closes(&chunk), vec![Register(0)]);
        assert_eq!(
            return_details(&chunk),
            vec![(Register(0), Count::Fixed(0), None)]
        );

        let chunk = compile_source(
            "if true then local resource <close> = nil return (nil)() else return 1 end",
        );

        assert!(standalone_closes(&chunk).is_empty());
        assert_eq!(
            return_details(&chunk),
            vec![
                (Register(1), Count::Open, Some(Register(0))),
                (Register(0), Count::Fixed(1), None),
            ]
        );
        assert!(chunk.entry.code.windows(2).any(|instructions| matches!(
            instructions,
            [
                Instruction::Call {
                    base: Register(1),
                    results: Count::Open,
                    ..
                },
                Instruction::Return {
                    base: Register(1),
                    values: Count::Open,
                    close_from: Some(Register(0)),
                },
            ]
        )));
    }

    #[test]
    fn closes_captured_branch_locals_before_reusing_their_registers() {
        let chunk = compile_source(
            "local stored\n\
             if true then\n\
                 local value = 1\n\
                 stored = function() return value end\n\
             end\n\
             return stored",
        );

        assert_eq!(chunk.entry.children.len(), 1);
        assert!(matches!(
            chunk.entry.children[0].upvalues.as_ref(),
            [UpvalueDescriptor::ParentRegister(Register(1))]
        ));
        assert_eq!(standalone_closes(&chunk), vec![Register(1)]);
        assert_eq!(
            closures(&chunk.entry),
            vec![(Register(2), PrototypeIndex::new(0))]
        );
        assert_eq!(
            return_details(&chunk),
            vec![(Register(1), Count::Fixed(1), None)]
        );
    }

    #[test]
    fn adjusts_if_conditions_to_exactly_one_result() {
        let chunk = compile_source("if (nil)() then return 1 end");

        assert_eq!(
            calls(&chunk),
            vec![(Register(1), Count::Fixed(0), Count::Fixed(1))]
        );
        assert!(chunk.entry.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::JumpIfFalsy {
                condition: Register(0),
                ..
            }
        )));
    }
}
