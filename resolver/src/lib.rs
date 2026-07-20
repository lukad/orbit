use std::collections::HashMap;

use orbit_common::{Span, Spanned};
use orbit_parser::{
    ast::{
        AssignmentTarget, AssignmentTargetKind, BinaryOperator as AstBinaryOperator, Block, Call,
        Chunk, Expr, ExprKind, FunctionBody, FunctionName, LocalAttribute as AstLocalAttribute,
        LocalDecl, ReturnStmt, Stmt, StmtKind, TableFieldKind, UnaryOperator as AstUnaryOperator,
    },
    lexer::{ByteString, Symbol},
};

use crate::{
    arena::Arena,
    hir::{
        BinaryOperator, Binding, BlockId, ChildFunctionId, ExitPlan, ExprId, HirBlock, HirChunk,
        HirConditionalBranch, HirExpr, HirExprKind, HirFunction, HirLabel, HirLocal, HirPlace,
        HirPlaceKind, HirScope, HirStmt, HirStmtKind, HirTableField, HirUpvalue, LabelId,
        LocalAttribute, LocalId, LoopId, ScopeId, StmtId, StringId, UnaryOperator, UpvalueId,
        UpvalueSource,
    },
};

mod arena;
pub mod hir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}

impl Diagnostic {
    fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}

pub fn resolve(chunk: &Chunk) -> Result<HirChunk, Vec<Diagnostic>> {
    Resolver::new(chunk).resolve()
}

struct Resolver<'ast> {
    chunk: &'ast Chunk,
    environment_name: Symbol,
    diagnostics: Vec<Diagnostic>,
    strings: StringPoolBuilder,
    functions: Vec<FunctionBuilder>,
}

impl<'ast> Resolver<'ast> {
    fn new(chunk: &'ast Chunk) -> Self {
        Self {
            chunk,
            environment_name: Symbol::from("_ENV"),
            diagnostics: Vec::new(),
            strings: StringPoolBuilder::new(),
            functions: Vec::new(),
        }
    }

    fn resolve(mut self) -> Result<HirChunk, Vec<Diagnostic>> {
        self.push_root_function(self.chunk.span);
        let body = self.resolve_function_body(&[], self.chunk);
        self.finalize_current_gotos();
        let entry = self.pop_function(body);

        if self.diagnostics.is_empty() {
            Ok(HirChunk {
                strings: self.strings.finish(),
                entry,
            })
        } else {
            self.diagnostics.sort_by_key(|diagnostic| {
                (
                    diagnostic.span.source.get(),
                    diagnostic.span.start,
                    diagnostic.span.end,
                )
            });
            Err(self.diagnostics)
        }
    }

    fn current_function(&self) -> &FunctionBuilder {
        self.functions
            .last()
            .expect("resolution always has an active function")
    }

    fn current_function_mut(&mut self) -> &mut FunctionBuilder {
        self.functions
            .last_mut()
            .expect("resolution always has an active function")
    }

    fn push_root_function(&mut self, span: Span) {
        let mut function = FunctionBuilder::new(span, true);
        let environment = function.upvalues.push(HirUpvalue {
            name: self.environment_name.clone(),
            span,
            source: UpvalueSource::ExternalEnvironment,
        });
        function
            .upvalue_map
            .insert(UpvalueSource::ExternalEnvironment, environment);
        self.functions.push(function);
    }

    fn push_nested_function(&mut self, span: Span, is_vararg: bool) {
        self.functions.push(FunctionBuilder::new(span, is_vararg));
    }

    fn pop_function(&mut self, body: BlockId) -> HirFunction {
        let function = self
            .functions
            .pop()
            .expect("function stack must not be empty");

        debug_assert!(function.scope_stack.is_empty());
        debug_assert!(function.loop_stack.is_empty());
        debug_assert!(function.pending_gotos.is_empty());

        HirFunction {
            span: function.span,
            parameters: function.parameters,
            is_vararg: function.is_vararg,
            locals: function.locals.map(|local| local.hir),
            upvalues: function.upvalues,
            scopes: function.scopes,
            blocks: function.blocks,
            statements: function.statements,
            expressions: function.expressions,
            loop_count: function.loops.len(),
            labels: function.labels,
            children: function.children,
            body,
        }
    }

    fn resolve_nested_function(
        &mut self,
        body: &FunctionBody,
        implicit_self: Option<Spanned<Symbol>>,
    ) -> ChildFunctionId {
        self.push_nested_function(body.span, body.is_variadic);

        let mut parameters =
            Vec::with_capacity(body.parameters.len() + usize::from(implicit_self.is_some()));
        if let Some(self_parameter) = implicit_self {
            parameters.push(self_parameter);
        }
        parameters.extend(body.parameters.iter().cloned());

        let body_id = self.resolve_function_body(&parameters, &body.body);
        self.finalize_current_gotos();
        let function = self.pop_function(body_id);

        let parent = self.current_function_mut();
        let id = ChildFunctionId(
            u32::try_from(parent.children.len())
                .expect("too many nested functions in one function"),
        );
        parent.children.push(function);
        id
    }

    fn resolve_function_body(&mut self, parameters: &[Spanned<Symbol>], body: &Block) -> BlockId {
        let scope = self.enter_scope();
        self.predeclare_labels(body);

        let parameter_ids = parameters
            .iter()
            .map(|parameter| self.declare_local(parameter.clone(), None))
            .collect::<Vec<_>>();
        self.current_function_mut().parameters = parameter_ids;
        self.current_function_mut()
            .scope_stack
            .last_mut()
            .unwrap()
            .trailing_label_local_count = self.current_function().active_locals.len();

        let statements = self.resolve_block_contents(body, true);
        let block = self.push_block(body.span, scope, statements);
        self.finish_scope();
        block
    }

    fn resolve_scoped_block(&mut self, body: &Block) -> BlockId {
        let scope = self.enter_scope();
        self.predeclare_labels(body);
        let statements = self.resolve_block_contents(body, true);
        let block = self.push_block(body.span, scope, statements);
        self.finish_scope();
        block
    }

    fn resolve_block_contents(&mut self, body: &Block, allow_trailing_labels: bool) -> Vec<StmtId> {
        let mut statements = Vec::new();
        for (index, statement) in body.statements.iter().enumerate() {
            let trailing_label = allow_trailing_labels
                && body.return_statement.is_none()
                && body.statements[index + 1..].iter().all(|statement| {
                    matches!(statement.kind, StmtKind::Empty | StmtKind::Label(_))
                });
            if let Some(statement) = self.resolve_statement(statement, trailing_label) {
                statements.push(statement);
            }
        }

        if let Some(return_statement) = &body.return_statement {
            statements.push(self.resolve_return(return_statement));
        }
        statements
    }

    fn push_block(&mut self, span: Span, scope: ScopeId, statements: Vec<StmtId>) -> BlockId {
        self.current_function_mut().blocks.push(HirBlock {
            span,
            scope,
            statements,
        })
    }

    fn enter_scope(&mut self) -> ScopeId {
        let parent = self
            .current_function()
            .scope_stack
            .last()
            .map(|scope| scope.id);
        let scope = self.current_function_mut().scopes.push(HirScope {
            parent,
            has_captured_locals: false,
            has_to_be_closed_locals: false,
        });
        let active_local_base = self.current_function().active_locals.len();
        self.current_function_mut().scope_stack.push(ScopeFrame {
            id: scope,
            bindings: HashMap::new(),
            active_local_base,
            trailing_label_local_count: active_local_base,
            labels: HashMap::new(),
        });
        scope
    }

    fn finish_scope(&mut self) {
        let frame = self
            .current_function_mut()
            .scope_stack
            .pop()
            .expect("scope stack must not be empty");
        self.current_function_mut()
            .active_locals
            .truncate(frame.active_local_base);
    }

    fn current_scope(&self) -> ScopeId {
        self.current_function()
            .scope_stack
            .last()
            .expect("function must have an active scope")
            .id
    }

    fn predeclare_labels(&mut self, body: &Block) {
        for statement in &body.statements {
            let StmtKind::Label(name) = &statement.kind else {
                continue;
            };

            let duplicate = self
                .current_function()
                .scope_stack
                .last()
                .unwrap()
                .labels
                .get(&name.value)
                .copied();
            if let Some(previous) = duplicate {
                let previous_span = self.current_function().labels[previous].span;
                self.diagnostics.push(Diagnostic::error(
                    name.span,
                    format!(
                        "duplicate label `{}` (first declared at {}..{})",
                        name.value, previous_span.start, previous_span.end
                    ),
                ));
                continue;
            }

            let scope = self.current_scope();
            let label = self.current_function_mut().labels.push(HirLabel {
                name: name.value.clone(),
                span: name.span,
                scope,
                active_locals: Vec::new(),
            });
            self.current_function_mut()
                .scope_stack
                .last_mut()
                .unwrap()
                .labels
                .insert(name.value.clone(), label);
        }
    }

    fn declare_local(
        &mut self,
        name: Spanned<Symbol>,
        attribute: Option<LocalAttribute>,
    ) -> LocalId {
        let scope = self.current_scope();
        let local = self.current_function_mut().locals.push(LocalInfo {
            hir: HirLocal {
                name: name.value.clone(),
                span: name.span,
                attribute,
                captured: false,
            },
            scope,
        });
        if attribute == Some(LocalAttribute::Close) {
            self.current_function_mut().scopes[scope].has_to_be_closed_locals = true;
        }
        self.current_function_mut()
            .scope_stack
            .last_mut()
            .unwrap()
            .bindings
            .insert(name.value, local);
        self.current_function_mut().active_locals.push(local);
        local
    }

    fn find_local(&self, function_index: usize, name: &Symbol) -> Option<LocalId> {
        self.functions[function_index]
            .scope_stack
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(name).copied())
    }

    fn find_upvalue(&self, function_index: usize, name: &Symbol) -> Option<UpvalueId> {
        self.functions[function_index]
            .upvalues
            .iter()
            .find_map(|(id, upvalue)| (&upvalue.name == name).then_some(id))
    }

    fn resolve_lexical_binding(&mut self, name: &Spanned<Symbol>) -> Option<Binding> {
        let current = self.functions.len() - 1;
        if let Some(local) = self.find_local(current, &name.value) {
            return Some(Binding::Local(local));
        }
        if let Some(upvalue) = self.find_upvalue(current, &name.value) {
            return Some(Binding::Upvalue(upvalue));
        }
        self.resolve_capture(current, name).map(Binding::Upvalue)
    }

    fn resolve_capture(
        &mut self,
        function_index: usize,
        name: &Spanned<Symbol>,
    ) -> Option<UpvalueId> {
        let parent_index = function_index.checked_sub(1)?;
        if let Some(local) = self.find_local(parent_index, &name.value) {
            self.mark_captured(parent_index, local);
            return Some(self.intern_upvalue(
                function_index,
                name,
                UpvalueSource::ParentLocal(local),
            ));
        }
        if let Some(parent_upvalue) = self.find_upvalue(parent_index, &name.value) {
            return Some(self.intern_upvalue(
                function_index,
                name,
                UpvalueSource::ParentUpvalue(parent_upvalue),
            ));
        }
        if let Some(parent_upvalue) = self.resolve_capture(parent_index, name) {
            return Some(self.intern_upvalue(
                function_index,
                name,
                UpvalueSource::ParentUpvalue(parent_upvalue),
            ));
        }
        None
    }

    fn intern_upvalue(
        &mut self,
        function_index: usize,
        name: &Spanned<Symbol>,
        source: UpvalueSource,
    ) -> UpvalueId {
        if let Some(existing) = self.functions[function_index].upvalue_map.get(&source) {
            return *existing;
        }
        let upvalue = self.functions[function_index].upvalues.push(HirUpvalue {
            name: name.value.clone(),
            span: name.span,
            source,
        });
        self.functions[function_index]
            .upvalue_map
            .insert(source, upvalue);
        upvalue
    }

    fn mark_captured(&mut self, function_index: usize, local: LocalId) {
        let function = &mut self.functions[function_index];
        function.locals[local].hir.captured = true;
        let scope = function.locals[local].scope;
        function.scopes[scope].has_captured_locals = true;
    }

    fn resolve_statement(&mut self, statement: &Stmt, trailing_label: bool) -> Option<StmtId> {
        let kind = match &statement.kind {
            StmtKind::Empty => return None,
            StmtKind::Local { names, values } => {
                return Some(self.resolve_local_declaration(statement.span, names, values));
            }
            StmtKind::Assign { targets, values } => {
                let targets = targets
                    .iter()
                    .filter_map(|target| match self.resolve_assignment_target(target) {
                        Ok(target) => Some(target),
                        Err(diagnostic) => {
                            self.diagnostics.push(diagnostic);
                            None
                        }
                    })
                    .collect();
                let values = values
                    .iter()
                    .map(|value| self.resolve_expression(value))
                    .collect();
                HirStmtKind::Assign { targets, values }
            }
            StmtKind::Call(call) => HirStmtKind::Call {
                call: self.resolve_call(call, statement.span),
            },
            StmtKind::Label(name) => {
                return self.resolve_label(statement.span, name, trailing_label);
            }
            StmtKind::Break => {
                let Some(target) = self.current_function().loop_stack.last().copied() else {
                    self.diagnostics.push(Diagnostic::error(
                        statement.span,
                        "`break` is only valid inside a loop",
                    ));
                    return None;
                };
                let parent_scope = self.current_function().loops[target].parent_scope;
                HirStmtKind::Break {
                    target,
                    exit: self.exit_plan_to(Some(parent_scope)),
                }
            }
            StmtKind::Goto(name) => return self.resolve_goto(statement.span, name),
            StmtKind::Do(body) => HirStmtKind::Block(self.resolve_scoped_block(body)),
            StmtKind::While { condition, body } => {
                return Some(self.resolve_while(statement.span, condition, body));
            }
            StmtKind::Repeat { body, condition } => {
                return Some(self.resolve_repeat(statement.span, body, condition));
            }
            StmtKind::If {
                branches,
                else_block,
            } => {
                let branches = branches
                    .iter()
                    .map(|branch| HirConditionalBranch {
                        span: branch.condition.span.join(&branch.body.span),
                        condition: self.resolve_expression(&branch.condition),
                        body: self.resolve_scoped_block(&branch.body),
                    })
                    .collect();
                let else_block = else_block
                    .as_ref()
                    .map(|body| self.resolve_scoped_block(body));
                HirStmtKind::If {
                    branches,
                    else_block,
                }
            }
            StmtKind::NumericFor {
                name,
                initial,
                limit,
                step,
                body,
            } => {
                return Some(self.resolve_numeric_for(
                    statement.span,
                    name,
                    initial,
                    limit,
                    step.as_ref(),
                    body,
                ));
            }
            StmtKind::GenericFor {
                names,
                values,
                body,
            } => {
                return Some(self.resolve_generic_for(statement.span, names, values, body));
            }
            StmtKind::Function { name, body } => {
                return Some(self.resolve_named_function(statement.span, name, body));
            }
            StmtKind::LocalFunction { name, body } => {
                return Some(self.resolve_local_function(statement.span, name, body));
            }
        };
        Some(self.push_statement(statement.span, kind))
    }

    fn resolve_local_declaration(
        &mut self,
        span: Span,
        names: &[LocalDecl],
        expressions: &[Expr],
    ) -> StmtId {
        let values = expressions
            .iter()
            .map(|value| self.resolve_expression(value))
            .collect();

        let close_attributes = names
            .iter()
            .filter_map(|name| name.attribute.as_ref())
            .filter(|attribute| attribute.value == AstLocalAttribute::Close)
            .collect::<Vec<_>>();
        for attribute in close_attributes.iter().skip(1) {
            self.diagnostics.push(Diagnostic::error(
                attribute.span,
                "a local declaration can contain at most one `<close>` variable",
            ));
        }

        let locals = names
            .iter()
            .map(|name| self.declare_local(name.name.clone(), Self::resolve_attribute(name)))
            .collect();
        self.push_statement(span, HirStmtKind::Local { locals, values })
    }

    fn resolve_attribute(local: &LocalDecl) -> Option<LocalAttribute> {
        local
            .attribute
            .as_ref()
            .map(|attribute| match attribute.value {
                AstLocalAttribute::Const => LocalAttribute::Const,
                AstLocalAttribute::Close => LocalAttribute::Close,
            })
    }

    fn resolve_local_function(
        &mut self,
        span: Span,
        name: &Spanned<Symbol>,
        body: &FunctionBody,
    ) -> StmtId {
        let local = self.declare_local(name.clone(), None);
        let child = self.resolve_nested_function(body, None);
        let closure = self.push_expression(body.span, HirExprKind::Closure(child));
        self.push_statement(
            span,
            HirStmtKind::Local {
                locals: vec![local],
                values: vec![closure],
            },
        )
    }

    fn resolve_named_function(
        &mut self,
        span: Span,
        name: &FunctionName,
        body: &FunctionBody,
    ) -> StmtId {
        let target = self.resolve_function_name_place(name);
        let implicit_self = name.method.as_ref().map(|method| Spanned {
            value: Symbol::from("self"),
            span: method.span,
        });
        let child = self.resolve_nested_function(body, implicit_self);
        let closure = self.push_expression(body.span, HirExprKind::Closure(child));
        self.push_statement(
            span,
            HirStmtKind::Assign {
                targets: target.into_iter().collect(),
                values: vec![closure],
            },
        )
    }

    fn resolve_function_name_place(&mut self, name: &FunctionName) -> Option<HirPlace> {
        if name.fields.is_empty() && name.method.is_none() {
            return match self.resolve_name_place(name.name.clone()) {
                Ok(kind) => Some(HirPlace {
                    span: name.name.span,
                    kind,
                }),
                Err(diagnostic) => {
                    self.diagnostics.push(diagnostic);
                    None
                }
            };
        }

        let mut table_span = name.name.span;
        let mut table = self.resolve_name_expression(name.name.clone());
        let (intermediate, final_field) = if let Some(method) = &name.method {
            (name.fields.as_slice(), method)
        } else {
            let (final_field, intermediate) = name.fields.split_last().unwrap();
            (intermediate, final_field)
        };
        for field in intermediate {
            table_span = table_span.join(&field.span);
            table = self.resolve_field_expression(table, field, table_span);
        }
        let key = self.identifier_string_expression(final_field);
        Some(HirPlace {
            span: name.name.span.join(&final_field.span),
            kind: HirPlaceKind::Index { table, key },
        })
    }

    fn resolve_while(&mut self, span: Span, condition: &Expr, body: &Block) -> StmtId {
        let condition = self.resolve_expression(condition);
        let parent_scope = self.current_scope();
        let body_scope = self.enter_scope();
        self.predeclare_labels(body);
        let loop_id = self.push_loop(parent_scope);
        self.current_function_mut().loop_stack.push(loop_id);
        let statements = self.resolve_block_contents(body, true);
        let body = self.push_block(body.span, body_scope, statements);
        self.current_function_mut().loop_stack.pop();
        self.finish_scope();
        self.push_statement(
            span,
            HirStmtKind::While {
                loop_id,
                condition,
                body,
            },
        )
    }

    fn resolve_repeat(&mut self, span: Span, body: &Block, condition: &Expr) -> StmtId {
        let parent_scope = self.current_scope();
        let body_scope = self.enter_scope();
        self.predeclare_labels(body);
        let loop_id = self.push_loop(parent_scope);
        self.current_function_mut().loop_stack.push(loop_id);
        // Repeat-body locals remain in scope through the condition, so a label
        // immediately before `until` is not outside those locals' scopes.
        let statements = self.resolve_block_contents(body, false);
        // Unlike other loops, locals declared in a repeat body are visible in its condition.
        let condition = self.resolve_expression(condition);
        let body = self.push_block(body.span, body_scope, statements);
        self.current_function_mut().loop_stack.pop();
        self.finish_scope();
        self.push_statement(
            span,
            HirStmtKind::Repeat {
                loop_id,
                body,
                condition,
            },
        )
    }

    fn resolve_numeric_for(
        &mut self,
        span: Span,
        name: &Spanned<Symbol>,
        initial: &Expr,
        limit: &Expr,
        step: Option<&Expr>,
        body: &Block,
    ) -> StmtId {
        let initial = self.resolve_expression(initial);
        let limit = self.resolve_expression(limit);
        let step = step.map(|step| self.resolve_expression(step));
        let parent_scope = self.current_scope();
        let body_scope = self.enter_scope();
        self.predeclare_labels(body);
        let variable = self.declare_local(name.clone(), None);
        let loop_id = self.push_loop(parent_scope);
        self.current_function_mut().loop_stack.push(loop_id);
        let statements = self.resolve_block_contents(body, true);
        let body = self.push_block(body.span, body_scope, statements);
        self.current_function_mut().loop_stack.pop();
        self.finish_scope();
        self.push_statement(
            span,
            HirStmtKind::NumericFor {
                loop_id,
                variable,
                initial,
                limit,
                step,
                body,
            },
        )
    }

    fn resolve_generic_for(
        &mut self,
        span: Span,
        names: &[Spanned<Symbol>],
        values: &[Expr],
        body: &Block,
    ) -> StmtId {
        let expressions = values
            .iter()
            .map(|value| self.resolve_expression(value))
            .collect();
        let parent_scope = self.current_scope();
        let body_scope = self.enter_scope();
        self.predeclare_labels(body);
        let variables = names
            .iter()
            .map(|name| self.declare_local(name.clone(), None))
            .collect();
        let loop_id = self.push_loop(parent_scope);
        self.current_function_mut().loop_stack.push(loop_id);
        let statements = self.resolve_block_contents(body, true);
        let body = self.push_block(body.span, body_scope, statements);
        self.current_function_mut().loop_stack.pop();
        self.finish_scope();
        self.push_statement(
            span,
            HirStmtKind::GenericFor {
                loop_id,
                variables,
                expressions,
                body,
            },
        )
    }

    fn push_loop(&mut self, parent_scope: ScopeId) -> LoopId {
        self.current_function_mut()
            .loops
            .push(LoopInfo { parent_scope })
    }

    fn resolve_label(
        &mut self,
        statement_span: Span,
        name: &Spanned<Symbol>,
        trailing_label: bool,
    ) -> Option<StmtId> {
        let label = self
            .current_function()
            .scope_stack
            .last()
            .unwrap()
            .labels
            .get(&name.value)
            .copied()
            .expect("labels are predeclared before their block is resolved");

        // A duplicate points at the first label's ID and was already diagnosed.
        if self.current_function().labels[label].span != name.span {
            return None;
        }

        let mut active_locals = self.current_function().active_locals.clone();
        if trailing_label {
            let local_count = self
                .current_function()
                .scope_stack
                .last()
                .unwrap()
                .trailing_label_local_count;
            active_locals.truncate(local_count);
        }
        self.current_function_mut().labels[label].active_locals = active_locals;
        Some(self.push_statement(statement_span, HirStmtKind::Label { label }))
    }

    fn resolve_goto(&mut self, statement_span: Span, name: &Spanned<Symbol>) -> Option<StmtId> {
        let target = self
            .current_function()
            .scope_stack
            .iter()
            .rev()
            .find_map(|scope| scope.labels.get(&name.value).copied());
        let Some(target) = target else {
            self.diagnostics.push(Diagnostic::error(
                name.span,
                format!("no visible label named `{}`", name.value),
            ));
            return None;
        };

        let target_scope = self.current_function().labels[target].scope;
        let exit = self.exit_plan_to(Some(target_scope));
        let statement = self.push_statement(statement_span, HirStmtKind::Goto { target, exit });
        let active_locals = self.current_function().active_locals.clone();
        self.current_function_mut().pending_gotos.push(PendingGoto {
            span: name.span,
            target,
            active_locals,
        });
        Some(statement)
    }

    fn finalize_current_gotos(&mut self) {
        let pending = std::mem::take(&mut self.current_function_mut().pending_gotos);
        for goto in pending {
            let label = &self.current_function().labels[goto.target];
            if let Some(local) = label
                .active_locals
                .iter()
                .find(|local| !goto.active_locals.contains(local))
                .copied()
            {
                let local = &self.current_function().locals[local].hir;
                self.diagnostics.push(Diagnostic::error(
                    goto.span,
                    format!(
                        "goto jumps into the scope of local `{}` declared at {}..{}",
                        local.name, local.span.start, local.span.end
                    ),
                ));
            }
        }
    }

    fn exit_plan_to(&self, target_scope: Option<ScopeId>) -> ExitPlan {
        let scopes = self
            .current_function()
            .scope_stack
            .iter()
            .rev()
            .map(|frame| frame.id)
            .take_while(|scope| Some(*scope) != target_scope)
            .collect();
        ExitPlan { scopes }
    }

    fn resolve_return(&mut self, statement: &ReturnStmt) -> StmtId {
        let values = statement
            .values
            .iter()
            .map(|value| self.resolve_expression(value))
            .collect();
        let exit = self.exit_plan_to(None);
        self.push_statement(statement.span, HirStmtKind::Return { values, exit })
    }

    fn resolve_assignment_target(
        &mut self,
        target: &AssignmentTarget,
    ) -> Result<HirPlace, Diagnostic> {
        let kind = match &target.kind {
            AssignmentTargetKind::Name(name) => self.resolve_name_place(Spanned {
                value: name.clone(),
                span: target.span,
            })?,
            AssignmentTargetKind::Index { table, key } => HirPlaceKind::Index {
                table: self.resolve_expression(table),
                key: self.resolve_expression(key),
            },
            AssignmentTargetKind::Field { table, field } => {
                let table = self.resolve_expression(table);
                let key = self.identifier_string_expression(field);
                HirPlaceKind::Index { table, key }
            }
        };
        Ok(HirPlace {
            span: target.span,
            kind,
        })
    }

    fn resolve_name_place(&mut self, name: Spanned<Symbol>) -> Result<HirPlaceKind, Diagnostic> {
        match self.resolve_lexical_binding(&name) {
            Some(Binding::Local(local)) => {
                self.check_local_mutable(local, name.span)?;
                Ok(HirPlaceKind::Local(local))
            }
            Some(Binding::Upvalue(upvalue)) => {
                self.check_upvalue_mutable(upvalue, name.span)?;
                Ok(HirPlaceKind::Upvalue(upvalue))
            }
            None => Ok(HirPlaceKind::Index {
                table: self.environment_expression(name.span),
                key: self.identifier_string_expression(&name),
            }),
        }
    }

    fn check_local_mutable(&self, local: LocalId, assignment_span: Span) -> Result<(), Diagnostic> {
        let local = &self.current_function().locals[local].hir;
        if local.attribute.is_some() {
            return Err(Diagnostic::error(
                assignment_span,
                format!(
                    "cannot assign to immutable local `{}` declared at {}..{}",
                    local.name, local.span.start, local.span.end
                ),
            ));
        }
        Ok(())
    }

    fn check_upvalue_mutable(
        &self,
        upvalue: UpvalueId,
        assignment_span: Span,
    ) -> Result<(), Diagnostic> {
        let current = self.functions.len() - 1;
        if let Some((name, declaration_span, attribute)) = self.captured_local(current, upvalue)
            && attribute.is_some()
        {
            return Err(Diagnostic::error(
                assignment_span,
                format!(
                    "cannot assign to immutable captured local `{name}` declared at {}..{}",
                    declaration_span.start, declaration_span.end
                ),
            ));
        }
        Ok(())
    }

    fn captured_local(
        &self,
        function_index: usize,
        upvalue: UpvalueId,
    ) -> Option<(Symbol, Span, Option<LocalAttribute>)> {
        let parent = function_index.checked_sub(1)?;
        match self.functions[function_index].upvalues[upvalue].source {
            UpvalueSource::ExternalEnvironment => None,
            UpvalueSource::ParentLocal(local) => {
                let local = &self.functions[parent].locals[local].hir;
                Some((local.name.clone(), local.span, local.attribute))
            }
            UpvalueSource::ParentUpvalue(upvalue) => self.captured_local(parent, upvalue),
        }
    }

    fn resolve_expression(&mut self, expression: &Expr) -> ExprId {
        let kind = match &expression.kind {
            ExprKind::Nil => HirExprKind::Nil,
            ExprKind::Boolean(value) => HirExprKind::Boolean(*value),
            ExprKind::Integer(value) => HirExprKind::Integer(*value),
            ExprKind::Float(value) => HirExprKind::Float(*value),
            ExprKind::String(value) => HirExprKind::String(self.intern_string(value)),
            ExprKind::Vararg => {
                if !self.current_function().is_vararg {
                    self.diagnostics.push(Diagnostic::error(
                        expression.span,
                        "cannot use `...` outside a variadic function",
                    ));
                }
                HirExprKind::Vararg
            }
            ExprKind::Name(name) => {
                return self.resolve_name_expression(Spanned {
                    value: name.clone(),
                    span: expression.span,
                });
            }
            ExprKind::Parenthesized(inner) => {
                let inner_id = self.resolve_expression(inner);
                if matches!(inner.kind, ExprKind::Call(_) | ExprKind::Vararg) {
                    HirExprKind::AdjustToOne {
                        expression: inner_id,
                    }
                } else {
                    return inner_id;
                }
            }
            ExprKind::Unary {
                operator,
                expression: operand,
            } => HirExprKind::Unary {
                operator: Self::resolve_unary_operator(*operator),
                operand: self.resolve_expression(operand),
            },
            ExprKind::Binary {
                left,
                operator,
                right,
            } => HirExprKind::Binary {
                left: self.resolve_expression(left),
                operator: Self::resolve_binary_operator(*operator),
                right: self.resolve_expression(right),
            },
            ExprKind::Index { table, key } => HirExprKind::Index {
                table: self.resolve_expression(table),
                key: self.resolve_expression(key),
            },
            ExprKind::Field { table, field } => {
                let table = self.resolve_expression(table);
                return self.resolve_field_expression(table, field, expression.span);
            }
            ExprKind::Call(call) => return self.resolve_call(call, expression.span),
            ExprKind::Function(body) => {
                HirExprKind::Closure(self.resolve_nested_function(body, None))
            }
            ExprKind::Table(fields) => HirExprKind::Table {
                fields: fields
                    .iter()
                    .map(|field| match &field.kind {
                        TableFieldKind::Indexed { key, value } => HirTableField::Computed {
                            span: field.span,
                            key: self.resolve_expression(key),
                            value: self.resolve_expression(value),
                        },
                        TableFieldKind::Named { name, value } => HirTableField::Record {
                            span: field.span,
                            name: self.intern_identifier(&name.value),
                            value: self.resolve_expression(value),
                        },
                        TableFieldKind::Value(value) => HirTableField::List {
                            span: field.span,
                            value: self.resolve_expression(value),
                        },
                    })
                    .collect(),
            },
        };
        self.push_expression(expression.span, kind)
    }

    fn resolve_call(&mut self, call: &Call, span: Span) -> ExprId {
        let kind = match call {
            Call::Function { callee, arguments } => HirExprKind::Call {
                callee: self.resolve_expression(callee),
                arguments: arguments
                    .iter()
                    .map(|argument| self.resolve_expression(argument))
                    .collect(),
            },
            Call::Method {
                receiver,
                method,
                arguments,
            } => HirExprKind::MethodCall {
                receiver: self.resolve_expression(receiver),
                method: self.intern_identifier(&method.value),
                arguments: arguments
                    .iter()
                    .map(|argument| self.resolve_expression(argument))
                    .collect(),
            },
        };
        self.push_expression(span, kind)
    }

    fn resolve_name_expression(&mut self, name: Spanned<Symbol>) -> ExprId {
        if let Some(binding) = self.resolve_lexical_binding(&name) {
            return self.push_expression(name.span, HirExprKind::Read(binding));
        }
        self.resolve_environment_index(&name)
    }

    fn resolve_environment_binding(&mut self, span: Span) -> Binding {
        let environment = Spanned {
            value: self.environment_name.clone(),
            span,
        };
        self.resolve_lexical_binding(&environment)
            .expect("the root function always provides the external environment")
    }

    fn environment_expression(&mut self, span: Span) -> ExprId {
        let environment = self.resolve_environment_binding(span);
        self.push_expression(span, HirExprKind::Read(environment))
    }

    fn resolve_environment_index(&mut self, name: &Spanned<Symbol>) -> ExprId {
        let table = self.environment_expression(name.span);
        let key = self.identifier_string_expression(name);
        self.push_expression(name.span, HirExprKind::Index { table, key })
    }

    fn resolve_field_expression(
        &mut self,
        table: ExprId,
        field: &Spanned<Symbol>,
        span: Span,
    ) -> ExprId {
        let key = self.identifier_string_expression(field);
        self.push_expression(span, HirExprKind::Index { table, key })
    }

    fn identifier_string_expression(&mut self, name: &Spanned<Symbol>) -> ExprId {
        let string = self.intern_identifier(&name.value);
        self.push_expression(name.span, HirExprKind::String(string))
    }

    fn intern_identifier(&mut self, name: &Symbol) -> StringId {
        self.strings.intern(name.as_str().as_bytes())
    }

    fn intern_string(&mut self, value: &ByteString) -> StringId {
        self.strings.intern(value.as_bytes())
    }

    fn push_statement(&mut self, span: Span, kind: HirStmtKind) -> StmtId {
        self.current_function_mut()
            .statements
            .push(HirStmt { span, kind })
    }

    fn push_expression(&mut self, span: Span, kind: HirExprKind) -> ExprId {
        self.current_function_mut()
            .expressions
            .push(HirExpr { span, kind })
    }

    fn resolve_unary_operator(operator: AstUnaryOperator) -> UnaryOperator {
        match operator {
            AstUnaryOperator::Negate => UnaryOperator::Negate,
            AstUnaryOperator::Not => UnaryOperator::Not,
            AstUnaryOperator::Length => UnaryOperator::Length,
            AstUnaryOperator::BitwiseNot => UnaryOperator::BitwiseNot,
        }
    }

    fn resolve_binary_operator(operator: AstBinaryOperator) -> BinaryOperator {
        match operator {
            AstBinaryOperator::Add => BinaryOperator::Add,
            AstBinaryOperator::Subtract => BinaryOperator::Subtract,
            AstBinaryOperator::Multiply => BinaryOperator::Multiply,
            AstBinaryOperator::Divide => BinaryOperator::Divide,
            AstBinaryOperator::FloorDivide => BinaryOperator::FloorDivide,
            AstBinaryOperator::Modulo => BinaryOperator::Modulo,
            AstBinaryOperator::Power => BinaryOperator::Power,
            AstBinaryOperator::BitwiseAnd => BinaryOperator::BitwiseAnd,
            AstBinaryOperator::BitwiseOr => BinaryOperator::BitwiseOr,
            AstBinaryOperator::BitwiseXor => BinaryOperator::BitwiseXor,
            AstBinaryOperator::ShiftLeft => BinaryOperator::ShiftLeft,
            AstBinaryOperator::ShiftRight => BinaryOperator::ShiftRight,
            AstBinaryOperator::Concat => BinaryOperator::Concat,
            AstBinaryOperator::Equal => BinaryOperator::Equal,
            AstBinaryOperator::NotEqual => BinaryOperator::NotEqual,
            AstBinaryOperator::LessThan => BinaryOperator::LessThan,
            AstBinaryOperator::LessThanOrEqual => BinaryOperator::LessEqual,
            AstBinaryOperator::GreaterThan => BinaryOperator::GreaterThan,
            AstBinaryOperator::GreaterThanOrEqual => BinaryOperator::GreaterEqual,
            AstBinaryOperator::And => BinaryOperator::And,
            AstBinaryOperator::Or => BinaryOperator::Or,
        }
    }
}

struct FunctionBuilder {
    span: Span,
    is_vararg: bool,
    parameters: Vec<LocalId>,
    locals: Arena<LocalId, LocalInfo>,
    upvalues: Arena<UpvalueId, HirUpvalue>,
    scopes: Arena<ScopeId, HirScope>,
    blocks: Arena<BlockId, HirBlock>,
    statements: Arena<StmtId, HirStmt>,
    expressions: Arena<ExprId, HirExpr>,
    loops: Arena<LoopId, LoopInfo>,
    labels: Arena<LabelId, HirLabel>,
    children: Vec<HirFunction>,
    scope_stack: Vec<ScopeFrame>,
    active_locals: Vec<LocalId>,
    upvalue_map: HashMap<UpvalueSource, UpvalueId>,
    loop_stack: Vec<LoopId>,
    pending_gotos: Vec<PendingGoto>,
}

impl FunctionBuilder {
    fn new(span: Span, is_vararg: bool) -> Self {
        Self {
            span,
            is_vararg,
            parameters: Vec::new(),
            locals: Arena::new(),
            upvalues: Arena::new(),
            scopes: Arena::new(),
            blocks: Arena::new(),
            statements: Arena::new(),
            expressions: Arena::new(),
            loops: Arena::new(),
            labels: Arena::new(),
            children: Vec::new(),
            scope_stack: Vec::new(),
            active_locals: Vec::new(),
            upvalue_map: HashMap::new(),
            loop_stack: Vec::new(),
            pending_gotos: Vec::new(),
        }
    }
}

struct ScopeFrame {
    id: ScopeId,
    bindings: HashMap<Symbol, LocalId>,
    active_local_base: usize,
    trailing_label_local_count: usize,
    labels: HashMap<Symbol, LabelId>,
}

struct LocalInfo {
    // Resolution-only ownership metadata. The emitted HIR has a single source
    // of truth for local order in parameters and statements.
    hir: HirLocal,
    scope: ScopeId,
}

struct LoopInfo {
    // Needed while resolving `break`, but redundant once the structured loop
    // statement and its exit plan have been produced.
    parent_scope: ScopeId,
}

struct PendingGoto {
    span: Span,
    target: LabelId,
    active_locals: Vec<LocalId>,
}

struct StringPoolBuilder {
    strings: Vec<Box<[u8]>>,
    ids: HashMap<Box<[u8]>, StringId>,
}

impl StringPoolBuilder {
    fn new() -> Self {
        Self {
            strings: Vec::new(),
            ids: HashMap::new(),
        }
    }

    fn intern(&mut self, value: &[u8]) -> StringId {
        if let Some(id) = self.ids.get(value) {
            return *id;
        }
        let id =
            StringId::new(u32::try_from(self.strings.len()).expect("too many interned strings"));
        let value: Box<[u8]> = value.into();
        self.ids.insert(value.clone(), id);
        self.strings.push(value);
        id
    }

    fn finish(self) -> Vec<Box<[u8]>> {
        self.strings
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::SourceId;
    use orbit_parser::{lexer::lex, parser::parse_chunk};

    use super::*;

    fn resolve_source(source: &str) -> Result<HirChunk, Vec<Diagnostic>> {
        let source_id = SourceId::new(0);
        let tokens = lex(source_id, source).expect("test source should lex");
        let chunk = parse_chunk(source_id, &tokens).expect("test source should parse");
        resolve(&chunk)
    }

    fn resolved(source: &str) -> HirChunk {
        resolve_source(source)
            .unwrap_or_else(|diagnostics| panic!("resolution failed: {diagnostics:#?}"))
    }

    fn root_statements(chunk: &HirChunk) -> &[StmtId] {
        &chunk.entry.blocks[chunk.entry.body].statements
    }

    #[test]
    fn lowers_the_complete_parser_surface() {
        let chunk = resolved(
            r#"
                local seed <const> = 1
                local t = { seed, named = "value", [seed] = 3 }
                do
                    t.x = -seed + #t * (~seed // 2) ^ 2
                end
                if seed < 2 and true then
                    print("yes")
                elseif false or nil then
                    print("no")
                else
                    print("else")
                end
                while seed < 2 do break end
                repeat local visible = 1 until visible == 1
                for i = 1, 10, 2 do print(i) end
                for key, value in pairs(t) do print(key, value) end
                local f = function(a, ...) return a, ... end
                function t.child:method(parameter) return self, parameter end
                f((seed), ...)
            "#,
        );

        let statements = root_statements(&chunk);
        assert_eq!(statements.len(), 11);
        assert!(matches!(
            chunk.entry.statements[statements[0]].kind,
            HirStmtKind::Local { .. }
        ));
        assert!(matches!(
            chunk.entry.statements[statements[2]].kind,
            HirStmtKind::Block(_)
        ));
        assert!(matches!(
            chunk.entry.statements[statements[3]].kind,
            HirStmtKind::If { .. }
        ));
        assert!(matches!(
            chunk.entry.statements[statements[4]].kind,
            HirStmtKind::While { .. }
        ));
        assert!(matches!(
            chunk.entry.statements[statements[5]].kind,
            HirStmtKind::Repeat { .. }
        ));
        assert!(matches!(
            chunk.entry.statements[statements[6]].kind,
            HirStmtKind::NumericFor { .. }
        ));
        assert!(matches!(
            chunk.entry.statements[statements[7]].kind,
            HirStmtKind::GenericFor { .. }
        ));
        assert!(matches!(
            chunk.entry.statements[statements[10]].kind,
            HirStmtKind::Call { .. }
        ));
        assert_eq!(chunk.entry.loop_count, 4);
        assert_eq!(chunk.entry.children.len(), 2);
    }

    #[test]
    fn local_initializers_resolve_before_new_bindings() {
        let chunk = resolved("local x = x; x = y");
        let statements = root_statements(&chunk);

        let HirStmtKind::Local { locals, values } = &chunk.entry.statements[statements[0]].kind
        else {
            panic!("expected local declaration")
        };
        let local = locals[0];
        let HirExprKind::Index { table, .. } = chunk.entry.expressions[values[0]].kind else {
            panic!("initializer should read the global x")
        };
        assert!(matches!(
            chunk.entry.expressions[table].kind,
            HirExprKind::Read(Binding::Upvalue(UpvalueId(0)))
        ));

        let HirStmtKind::Assign { targets, values } = &chunk.entry.statements[statements[1]].kind
        else {
            panic!("expected assignment")
        };
        assert!(matches!(targets[0].kind, HirPlaceKind::Local(id) if id == local));
        assert!(matches!(
            chunk.entry.expressions[values[0]].kind,
            HirExprKind::Index { .. }
        ));
    }

    #[test]
    fn environment_is_lexical_and_can_be_shadowed_or_captured() {
        let chunk = resolved("local sandbox = {}; local _ENV = sandbox; return global_name");
        let statements = root_statements(&chunk);
        let HirStmtKind::Local { locals, .. } = &chunk.entry.statements[statements[1]].kind else {
            panic!("expected local environment")
        };
        let local_environment = locals[0];
        let HirStmtKind::Return { values, .. } = &chunk.entry.statements[statements[2]].kind else {
            panic!("expected return")
        };
        let HirExprKind::Index { table, .. } = chunk.entry.expressions[values[0]].kind else {
            panic!("global should lower through the environment")
        };
        assert!(matches!(
            chunk.entry.expressions[table].kind,
            HirExprKind::Read(Binding::Local(id)) if id == local_environment
        ));

        let chunk = resolved("return function() return global_name end");
        assert!(matches!(
            chunk.entry.children[0].upvalues[UpvalueId(0)].source,
            UpvalueSource::ParentUpvalue(UpvalueId(0))
        ));
    }

    #[test]
    fn propagates_captures_across_multiple_function_levels() {
        let chunk =
            resolved("local x = 1; local f = function() return function() return x end end");
        let root = &chunk.entry;
        assert!(root.locals[LocalId(0)].captured);
        assert!(
            root.scopes
                .iter()
                .any(|(_, scope)| scope.has_captured_locals)
        );

        let middle = &root.children[0];
        assert!(matches!(
            middle.upvalues[UpvalueId(0)].source,
            UpvalueSource::ParentLocal(LocalId(0))
        ));
        let inner = &middle.children[0];
        assert!(matches!(
            inner.upvalues[UpvalueId(0)].source,
            UpvalueSource::ParentUpvalue(UpvalueId(0))
        ));
    }

    #[test]
    fn local_function_is_visible_to_its_own_body() {
        let chunk = resolved("local function recurse() return recurse end");
        let root = &chunk.entry;
        assert!(root.locals[LocalId(0)].captured);
        assert!(matches!(
            root.children[0].upvalues[UpvalueId(0)].source,
            UpvalueSource::ParentLocal(LocalId(0))
        ));
    }

    #[test]
    fn named_methods_lower_to_index_assignment_and_implicit_self() {
        let chunk = resolved("local t = {}; function t.child:run(value) return self, value end");
        let statements = root_statements(&chunk);
        let HirStmtKind::Assign { targets, values } = &chunk.entry.statements[statements[1]].kind
        else {
            panic!("function declaration should lower to assignment")
        };
        assert!(matches!(targets[0].kind, HirPlaceKind::Index { .. }));
        assert!(matches!(
            chunk.entry.expressions[values[0]].kind,
            HirExprKind::Closure(ChildFunctionId(0))
        ));

        let method = &chunk.entry.children[0];
        assert_eq!(method.parameters.len(), 2);
        assert_eq!(method.locals[method.parameters[0]].name.as_str(), "self");
        assert_eq!(method.locals[method.parameters[1]].name.as_str(), "value");
    }

    #[test]
    fn repeat_condition_can_read_body_locals() {
        let chunk = resolved("repeat local visible = 1 until visible == 1");
        let statement = root_statements(&chunk)[0];
        let HirStmtKind::Repeat {
            body, condition, ..
        } = chunk.entry.statements[statement].kind
        else {
            panic!("expected repeat loop")
        };
        let variable_statement = chunk.entry.blocks[body].statements[0];
        let HirStmtKind::Local { locals, .. } = &chunk.entry.statements[variable_statement].kind
        else {
            panic!("expected local declaration")
        };
        let variable = locals[0];
        let HirExprKind::Binary { left, .. } = chunk.entry.expressions[condition].kind else {
            panic!("expected binary condition")
        };
        assert!(matches!(
            chunk.entry.expressions[left].kind,
            HirExprKind::Read(Binding::Local(id)) if id == variable
        ));
    }

    #[test]
    fn reports_independent_semantic_errors_together() {
        let diagnostics = resolve_source(
            r#"
                break
                local constant <const> = 1
                constant = 2
                local resource <close> = {}
                resource = {}
                local function invalid_vararg() return ... end
                goto missing
                ::duplicate::
                ::duplicate::
            "#,
        )
        .expect_err("source should fail resolution");
        let messages = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();

        assert_eq!(diagnostics.len(), 6);
        assert!(messages.iter().any(|message| message.contains("`break`")));
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.contains("immutable local"))
                .count(),
            2
        );
        assert!(messages.iter().any(|message| message.contains("variadic")));
        assert!(
            messages
                .iter()
                .any(|message| message.contains("no visible label"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("duplicate label"))
        );
    }

    #[test]
    fn rejects_writes_to_const_captures() {
        let diagnostics = resolve_source(
            "local immutable <const> = 1; local function change() immutable = 2 end",
        )
        .expect_err("captured const assignment should fail");
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("immutable captured local"));
    }

    #[test]
    fn goto_cannot_enter_a_local_scope_but_can_target_a_trailing_label() {
        let diagnostics =
            resolve_source("goto target; local crossed = 1; ::target::; print(crossed)")
                .expect_err("goto should not cross a local declaration");
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("crossed"));

        resolved("goto target; local skipped = 1; ::target::");
    }

    #[test]
    fn repeat_body_labels_do_not_get_the_trailing_label_exemption() {
        for source in [
            "repeat goto done; local crossed = true; ::done:: until true",
            "repeat goto done; local crossed = true; ::done:: until crossed",
        ] {
            let diagnostics =
                resolve_source(source).expect_err("goto should not skip a repeat-body local");

            assert_eq!(diagnostics.len(), 1);
            assert!(diagnostics[0].message.contains("crossed"));
        }

        resolved("repeat do goto done; local skipped = true; ::done:: end until true");
    }

    #[test]
    fn inner_forward_labels_shadow_outer_labels() {
        let chunk = resolved("::same:: do goto same; ::same:: end");
        let root_statements = root_statements(&chunk);
        let HirStmtKind::Block(block) = chunk.entry.statements[root_statements[1]].kind else {
            panic!("expected do block")
        };
        let inner_statements = &chunk.entry.blocks[block].statements;
        let HirStmtKind::Goto { target, .. } = chunk.entry.statements[inner_statements[0]].kind
        else {
            panic!("expected goto")
        };
        assert_eq!(target, LabelId(1));
    }

    #[test]
    fn exit_plans_list_each_scope_being_left() {
        let chunk = resolved(
            r#"
                ::again::
                do
                    local resource <close> = {}
                    goto again
                end
                while true do
                    do break end
                end
            "#,
        );
        let root_ids = root_statements(&chunk);

        let HirStmtKind::Block(block) = chunk.entry.statements[root_ids[1]].kind else {
            panic!("expected do block")
        };
        let inner_scope = chunk.entry.blocks[block].scope;
        let goto_statement = chunk.entry.blocks[block].statements[1];
        let HirStmtKind::Goto { exit, .. } = &chunk.entry.statements[goto_statement].kind else {
            panic!("expected goto")
        };
        assert_eq!(exit.scopes, vec![inner_scope]);
        assert!(chunk.entry.scopes[inner_scope].has_to_be_closed_locals);

        let HirStmtKind::While { body, .. } = chunk.entry.statements[root_ids[2]].kind else {
            panic!("expected while")
        };
        let loop_scope = chunk.entry.blocks[body].scope;
        let nested_block_statement = chunk.entry.blocks[body].statements[0];
        let HirStmtKind::Block(nested_block) = chunk.entry.statements[nested_block_statement].kind
        else {
            panic!("expected nested do block")
        };
        let nested_scope = chunk.entry.blocks[nested_block].scope;
        let break_statement = chunk.entry.blocks[nested_block].statements[0];
        let HirStmtKind::Break { exit, .. } = &chunk.entry.statements[break_statement].kind else {
            panic!("expected break")
        };
        assert_eq!(exit.scopes, vec![nested_scope, loop_scope]);

        let chunk = resolved("do return end");
        let root_scope = chunk.entry.blocks[chunk.entry.body].scope;
        let HirStmtKind::Block(block) = chunk.entry.statements[root_statements(&chunk)[0]].kind
        else {
            panic!("expected do block")
        };
        let inner_scope = chunk.entry.blocks[block].scope;
        let return_statement = chunk.entry.blocks[block].statements[0];
        let HirStmtKind::Return { exit, .. } = &chunk.entry.statements[return_statement].kind
        else {
            panic!("expected return")
        };
        assert_eq!(exit.scopes, vec![inner_scope, root_scope]);
    }

    #[test]
    fn parenthesized_calls_are_adjusted_to_one_result() {
        let chunk = resolved("return f(), (f()), ((f()))");
        let adjust_count = chunk
            .entry
            .expressions
            .iter()
            .filter(|(_, expression)| matches!(expression.kind, HirExprKind::AdjustToOne { .. }))
            .count();
        assert_eq!(adjust_count, 2);
    }

    #[test]
    fn string_pool_deduplicates_literals_and_identifier_keys() {
        let chunk =
            resolved(r#"local first = "same"; local second = { same = "same" }; print("same")"#);
        assert_eq!(
            chunk
                .strings
                .iter()
                .filter(|string| string.as_ref() == b"same")
                .count(),
            1
        );
    }
}
