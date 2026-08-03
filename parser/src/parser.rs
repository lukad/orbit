use std::collections::VecDeque;

use orbit_common::{SourceId, Span, Spanned, number::Number};
use strum::IntoDiscriminant;

use crate::ast::{
    AssignmentTarget, AssignmentTargetKind, BinaryOperator, Block, Call, Chunk, Expr, ExprKind,
    FunctionBody, FunctionName, IfBranch, LocalAttribute, LocalDecl, ReturnStmt, Stmt, StmtKind,
    TableField, TableFieldKind, UnaryOperator,
};
use crate::lexer::{Symbol, Token, TokenKind};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("{kind}")]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    #[error("expected {expected:?}, but found {actual:?}")]
    ExpectedToken {
        expected: TokenKind,
        actual: Option<TokenKind>,
    },
    #[error("expected an expression, but found {actual:?}")]
    ExpectedExpression { actual: Option<TokenKind> },
    #[error("expected a statement, but found {actual:?}")]
    ExpectedStatement { actual: Option<TokenKind> },
    #[error("expected a function call or assignment")]
    ExpectedCallOrAssignment,
    #[error("invalid assignment target")]
    InvalidAssignmentTarget,
    #[error("expected function arguments")]
    ExpectedArguments,
    #[error("unknown attribute '{attribute}'")]
    InvalidLocalAttribute { attribute: Symbol },
    #[error("a return statement must be the final statement in its block")]
    StatementAfterReturn,
    #[error("expected EOF, but found {actual:?}")]
    ExpectedEof { actual: TokenKind },
}

pub type ParseResult<T> = Result<T, ParseError>;

pub fn parse_chunk(source_id: SourceId, tokens: Vec<Spanned<Token>>) -> ParseResult<Chunk> {
    Parser::new(source_id, tokens).parse_chunk()
}

struct Parser {
    tokens: VecDeque<Spanned<Token>>,
    source_id: SourceId,
    source_end: u32,
}

impl Parser {
    fn new(source_id: SourceId, tokens: Vec<Spanned<Token>>) -> Self {
        let source_end = tokens.last().map_or(0, |token| token.span.end);
        Self {
            tokens: tokens.into(),
            source_id,
            source_end,
        }
    }

    fn parse_chunk(mut self) -> ParseResult<Chunk> {
        let mut chunk = self.parse_block_until(&[TokenKind::Eof])?;
        let eof_span = if self.at(TokenKind::Eof) {
            self.advance().unwrap().span
        } else {
            self.eof_span()
        };

        if let Some(token) = self.current() {
            return Err(self.error(ParseErrorKind::ExpectedEof {
                actual: token.value.discriminant(),
            }));
        }

        chunk.span = Span::new(self.source_id, 0, eof_span.end);
        Ok(chunk)
    }

    fn parse_block_until(&mut self, terminators: &[TokenKind]) -> ParseResult<Block> {
        let start = self.span().start;
        let mut end = start;
        let mut statements = Vec::new();
        let mut return_statement = None;

        while !self.at_any(terminators) && !self.at_eof() {
            if self.at(TokenKind::Return) {
                let return_stmt = self.parse_return_stmt(terminators)?;
                end = return_stmt.span.end;
                return_statement = Some(return_stmt);

                if !self.at_any(terminators) && !self.at_eof() {
                    return Err(self.error(ParseErrorKind::StatementAfterReturn));
                }
                break;
            }

            let statement = self.parse_stmt()?;
            end = statement.span.end;
            statements.push(statement);
        }

        Ok(Block::new(
            statements,
            return_statement,
            Span::new(self.source_id, start, end),
        ))
    }

    fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        match self.kind() {
            Some(TokenKind::Semicolon) => {
                let token = self.advance().unwrap();
                Ok(Stmt::new(StmtKind::Empty, token.span))
            }
            Some(TokenKind::Local) => self.parse_local_stmt(),
            Some(TokenKind::Function) => self.parse_function_stmt(),
            Some(TokenKind::Do) => self.parse_do_stmt(),
            Some(TokenKind::While) => self.parse_while_stmt(),
            Some(TokenKind::Repeat) => self.parse_repeat_stmt(),
            Some(TokenKind::If) => self.parse_if_stmt(),
            Some(TokenKind::For) => self.parse_for_stmt(),
            Some(TokenKind::Break) => {
                let token = self.advance().unwrap();
                Ok(Stmt::new(StmtKind::Break, token.span))
            }
            Some(TokenKind::Goto) => self.parse_goto_stmt(),
            Some(TokenKind::DoubleColon) => self.parse_label_stmt(),
            Some(TokenKind::Name | TokenKind::LeftParen) => self.parse_assignment_or_call_stmt(),
            actual => Err(self.error(ParseErrorKind::ExpectedStatement { actual })),
        }
    }

    fn parse_local_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::Local)?.span;

        if self.eat(TokenKind::Function).is_some() {
            let name = self.expect_name()?;
            let body = self.parse_function_body()?;
            let span = start.join(&body.span);
            return Ok(Stmt::new(StmtKind::LocalFunction { name, body }, span));
        }

        let mut names = vec![self.parse_local_decl()?];
        while self.eat(TokenKind::Comma).is_some() {
            names.push(self.parse_local_decl()?);
        }

        let values = if self.eat(TokenKind::Equal).is_some() {
            self.parse_expr_list()?
        } else {
            Vec::new()
        };
        let end = values.last().map_or_else(
            || names.last().unwrap().name.span,
            |expression| expression.span,
        );

        Ok(Stmt::new(
            StmtKind::Local { names, values },
            start.join(&end),
        ))
    }

    fn parse_local_decl(&mut self) -> ParseResult<LocalDecl> {
        let name = self.expect_name()?;
        let attribute = if let Some(open) = self.eat(TokenKind::Less) {
            let attribute_name = self.expect_name()?;
            let value = match attribute_name.value.as_str() {
                "const" => LocalAttribute::Const,
                "close" => LocalAttribute::Close,
                _ => {
                    let span = attribute_name.span;
                    return Err(self.error_at(
                        ParseErrorKind::InvalidLocalAttribute {
                            attribute: attribute_name.value,
                        },
                        span,
                    ));
                }
            };
            let close = self.expect(TokenKind::Greater)?;
            Some(Spanned {
                value,
                span: open.span.join(&close.span),
            })
        } else {
            None
        };

        Ok(LocalDecl { name, attribute })
    }

    fn parse_function_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::Function)?.span;
        let name = self.parse_function_name()?;
        let body = self.parse_function_body()?;
        let span = start.join(&body.span);

        Ok(Stmt::new(StmtKind::Function { name, body }, span))
    }

    fn parse_function_name(&mut self) -> ParseResult<FunctionName> {
        let name = self.expect_name()?;
        let mut fields = Vec::new();

        while self.eat(TokenKind::Dot).is_some() {
            fields.push(self.expect_name()?);
        }

        let method = if self.eat(TokenKind::Colon).is_some() {
            Some(self.expect_name()?)
        } else {
            None
        };

        Ok(FunctionName {
            name,
            fields,
            method,
        })
    }

    fn parse_function_body(&mut self) -> ParseResult<FunctionBody> {
        let open = self.expect(TokenKind::LeftParen)?;
        let mut parameters = Vec::new();
        let mut is_variadic = false;

        if !self.at(TokenKind::RightParen) {
            if self.eat(TokenKind::Ellipsis).is_some() {
                is_variadic = true;
            } else {
                parameters.push(self.expect_name()?);
                while self.eat(TokenKind::Comma).is_some() {
                    if self.eat(TokenKind::Ellipsis).is_some() {
                        is_variadic = true;
                        break;
                    }
                    parameters.push(self.expect_name()?);
                }
            }
        }

        self.expect(TokenKind::RightParen)?;
        let body = self.parse_block_until(&[TokenKind::End])?;
        let end = self.expect(TokenKind::End)?;

        Ok(FunctionBody {
            parameters,
            is_variadic,
            body,
            span: open.span.join(&end.span),
        })
    }

    fn parse_do_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::Do)?.span;
        let body = self.parse_block_until(&[TokenKind::End])?;
        let end = self.expect(TokenKind::End)?.span;

        Ok(Stmt::new(StmtKind::Do(body), start.join(&end)))
    }

    fn parse_while_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::While)?.span;
        let condition = self.parse_expr()?;
        self.expect(TokenKind::Do)?;
        let body = self.parse_block_until(&[TokenKind::End])?;
        let end = self.expect(TokenKind::End)?.span;

        Ok(Stmt::new(
            StmtKind::While { condition, body },
            start.join(&end),
        ))
    }

    fn parse_repeat_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::Repeat)?.span;
        let body = self.parse_block_until(&[TokenKind::Until])?;
        self.expect(TokenKind::Until)?;
        let condition = self.parse_expr()?;
        let span = start.join(&condition.span);

        Ok(Stmt::new(StmtKind::Repeat { body, condition }, span))
    }

    fn parse_if_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::If)?.span;
        let mut branches = Vec::new();

        loop {
            let condition = self.parse_expr()?;
            self.expect(TokenKind::Then)?;
            let body =
                self.parse_block_until(&[TokenKind::ElseIf, TokenKind::Else, TokenKind::End])?;
            branches.push(IfBranch { condition, body });

            if self.eat(TokenKind::ElseIf).is_none() {
                break;
            }
        }

        let else_block = if self.eat(TokenKind::Else).is_some() {
            Some(self.parse_block_until(&[TokenKind::End])?)
        } else {
            None
        };
        let end = self.expect(TokenKind::End)?.span;

        Ok(Stmt::new(
            StmtKind::If {
                branches,
                else_block,
            },
            start.join(&end),
        ))
    }

    fn parse_for_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::For)?.span;
        let first_name = self.expect_name()?;

        let kind = if self.eat(TokenKind::Equal).is_some() {
            let initial = self.parse_expr()?;
            self.expect(TokenKind::Comma)?;
            let limit = self.parse_expr()?;
            let step = if self.eat(TokenKind::Comma).is_some() {
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect(TokenKind::Do)?;
            let body = self.parse_block_until(&[TokenKind::End])?;

            StmtKind::NumericFor {
                name: first_name,
                initial,
                limit,
                step,
                body,
            }
        } else {
            let mut names = vec![first_name];
            while self.eat(TokenKind::Comma).is_some() {
                names.push(self.expect_name()?);
            }
            self.expect(TokenKind::In)?;
            let values = self.parse_expr_list()?;
            self.expect(TokenKind::Do)?;
            let body = self.parse_block_until(&[TokenKind::End])?;

            StmtKind::GenericFor {
                names,
                values,
                body,
            }
        };

        let end = self.expect(TokenKind::End)?.span;
        Ok(Stmt::new(kind, start.join(&end)))
    }

    fn parse_goto_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::Goto)?.span;
        let label = self.expect_name()?;
        let span = start.join(&label.span);
        Ok(Stmt::new(StmtKind::Goto(label), span))
    }

    fn parse_label_stmt(&mut self) -> ParseResult<Stmt> {
        let start = self.expect(TokenKind::DoubleColon)?.span;
        let label = self.expect_name()?;
        let end = self.expect(TokenKind::DoubleColon)?.span;
        Ok(Stmt::new(StmtKind::Label(label), start.join(&end)))
    }

    fn parse_assignment_or_call_stmt(&mut self) -> ParseResult<Stmt> {
        let expression = self.parse_prefix_expr()?;

        if self.at(TokenKind::Equal) || self.at(TokenKind::Comma) {
            let start = expression.span;
            let mut targets = vec![self.make_assignment_target(expression)?];
            while self.eat(TokenKind::Comma).is_some() {
                let target = self.parse_prefix_expr()?;
                targets.push(self.make_assignment_target(target)?);
            }
            self.expect(TokenKind::Equal)?;
            let values = self.parse_expr_list()?;
            let span = start.join(&values.last().unwrap().span);

            return Ok(Stmt::new(StmtKind::Assign { targets, values }, span));
        }

        let span = expression.span;
        match expression.kind {
            ExprKind::Call(call) => Ok(Stmt::new(StmtKind::Call(call), span)),
            _ => Err(self.error_at(ParseErrorKind::ExpectedCallOrAssignment, span)),
        }
    }

    fn make_assignment_target(&self, expression: Expr) -> ParseResult<AssignmentTarget> {
        let span = expression.span;
        let kind = match expression.kind {
            ExprKind::Name(name) => AssignmentTargetKind::Name(name),
            ExprKind::Index { table, key } => AssignmentTargetKind::Index { table, key },
            ExprKind::Field { table, field } => AssignmentTargetKind::Field { table, field },
            _ => {
                return Err(self.error_at(ParseErrorKind::InvalidAssignmentTarget, span));
            }
        };

        Ok(AssignmentTarget::new(kind, span))
    }

    fn parse_return_stmt(&mut self, terminators: &[TokenKind]) -> ParseResult<ReturnStmt> {
        let start = self.expect(TokenKind::Return)?.span;
        let values = if self.can_start_expr() {
            self.parse_expr_list()?
        } else {
            Vec::new()
        };
        let semicolon = self.eat(TokenKind::Semicolon);
        let end = semicolon
            .as_ref()
            .map(|token| token.span)
            .or_else(|| values.last().map(|expression| expression.span))
            .unwrap_or(start);

        if !self.at_any(terminators) && !self.at_eof() {
            return Err(self.error(ParseErrorKind::StatementAfterReturn));
        }

        Ok(ReturnStmt::new(values, start.join(&end)))
    }

    fn parse_expr_list(&mut self) -> ParseResult<Vec<Expr>> {
        let mut expressions = vec![self.parse_expr()?];
        while self.eat(TokenKind::Comma).is_some() {
            expressions.push(self.parse_expr()?);
        }
        Ok(expressions)
    }

    fn parse_expr(&mut self) -> ParseResult<Expr> {
        self.parse_expr_with_binding_power(0)
    }

    fn parse_expr_with_binding_power(&mut self, minimum: u8) -> ParseResult<Expr> {
        let mut left = if let Some(operator) = self.unary_operator() {
            let start = self.advance().unwrap().span;
            let expression = self.parse_expr_with_binding_power(11)?;
            let span = start.join(&expression.span);
            Expr::new(
                ExprKind::Unary {
                    operator,
                    expression: Box::new(expression),
                },
                span,
            )
        } else {
            self.parse_atom()?
        };

        while let Some((operator, left_power, right_power)) = self.binary_operator() {
            if left_power < minimum {
                break;
            }

            self.advance();
            let right = self.parse_expr_with_binding_power(right_power)?;
            let span = left.span.join(&right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    operator,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn parse_atom(&mut self) -> ParseResult<Expr> {
        match self.kind() {
            Some(TokenKind::LeftBrace) => return self.parse_table(),
            Some(TokenKind::Name | TokenKind::LeftParen) => return self.parse_prefix_expr(),
            _ => {}
        }

        let Some(token) = self.advance() else {
            return Err(self.error(ParseErrorKind::ExpectedExpression { actual: None }));
        };

        match token {
            Spanned {
                value: Token::Nil,
                span,
            } => Ok(Expr::new(ExprKind::Nil, span)),
            Spanned {
                value: Token::False,
                span,
            } => Ok(Expr::new(ExprKind::Boolean(false), span)),
            Spanned {
                value: Token::True,
                span,
            } => Ok(Expr::new(ExprKind::Boolean(true), span)),
            Spanned {
                value: Token::Number(value),
                span,
            } => {
                let kind = match value {
                    Number::Integer(value) => ExprKind::Integer(value),
                    Number::Float(value) => ExprKind::Float(value),
                };
                Ok(Expr::new(kind, span))
            }
            Spanned {
                value: Token::String(value),
                span,
            } => Ok(Expr::new(ExprKind::String(value), span)),
            Spanned {
                value: Token::Ellipsis,
                span,
            } => Ok(Expr::new(ExprKind::Vararg, span)),
            Spanned {
                value: Token::Function,
                span,
            } => {
                let body = self.parse_function_body()?;
                let expression_span = span.join(&body.span);
                Ok(Expr::new(
                    ExprKind::Function(Box::new(body)),
                    expression_span,
                ))
            }
            Spanned { value, span } => Err(self.error_at(
                ParseErrorKind::ExpectedExpression {
                    actual: Some(value.discriminant()),
                },
                span,
            )),
        }
    }

    fn parse_prefix_expr(&mut self) -> ParseResult<Expr> {
        let mut expression = match self.kind() {
            Some(TokenKind::Name) => {
                let name = self.expect_name()?;
                Expr::new(ExprKind::Name(name.value), name.span)
            }
            Some(TokenKind::LeftParen) => {
                let open = self.advance().unwrap();
                let inner = self.parse_expr()?;
                let close = self.expect(TokenKind::RightParen)?;
                Expr::new(
                    ExprKind::Parenthesized(Box::new(inner)),
                    open.span.join(&close.span),
                )
            }
            actual => return Err(self.error(ParseErrorKind::ExpectedExpression { actual })),
        };

        loop {
            match self.kind() {
                Some(TokenKind::LeftBracket) => {
                    self.advance();
                    let key = self.parse_expr()?;
                    let close = self.expect(TokenKind::RightBracket)?;
                    let span = expression.span.join(&close.span);
                    expression = Expr::new(
                        ExprKind::Index {
                            table: Box::new(expression),
                            key: Box::new(key),
                        },
                        span,
                    );
                }
                Some(TokenKind::Dot) => {
                    self.advance();
                    let field = self.expect_name()?;
                    let span = expression.span.join(&field.span);
                    expression = Expr::new(
                        ExprKind::Field {
                            table: Box::new(expression),
                            field,
                        },
                        span,
                    );
                }
                Some(TokenKind::Colon) => {
                    self.advance();
                    let method = self.expect_name()?;
                    let (arguments, end) = self.parse_call_args()?;
                    let span = expression.span.join(&end);
                    expression = Expr::new(
                        ExprKind::Call(Call::Method {
                            receiver: Box::new(expression),
                            method,
                            arguments,
                        }),
                        span,
                    );
                }
                Some(TokenKind::LeftParen | TokenKind::LeftBrace | TokenKind::String) => {
                    let (arguments, end) = self.parse_call_args()?;
                    let span = expression.span.join(&end);
                    expression = Expr::new(
                        ExprKind::Call(Call::Function {
                            callee: Box::new(expression),
                            arguments,
                        }),
                        span,
                    );
                }
                _ => break,
            }
        }

        Ok(expression)
    }

    fn parse_call_args(&mut self) -> ParseResult<(Vec<Expr>, Span)> {
        match self.kind() {
            Some(TokenKind::LeftParen) => {
                self.advance();
                let arguments = if self.at(TokenKind::RightParen) {
                    Vec::new()
                } else {
                    self.parse_expr_list()?
                };
                let close = self.expect(TokenKind::RightParen)?;
                Ok((arguments, close.span))
            }
            Some(TokenKind::LeftBrace) => {
                let table = self.parse_table()?;
                let span = table.span;
                Ok((vec![table], span))
            }
            Some(TokenKind::String) => {
                let string = self.parse_atom()?;
                let span = string.span;
                Ok((vec![string], span))
            }
            _ => Err(self.error(ParseErrorKind::ExpectedArguments)),
        }
    }

    fn parse_table(&mut self) -> ParseResult<Expr> {
        let open = self.expect(TokenKind::LeftBrace)?;
        let mut fields = Vec::new();

        while !self.at(TokenKind::RightBrace) {
            if self.at_eof() {
                return Err(self.expected(TokenKind::RightBrace));
            }

            let field = if let Some(field_open) = self.eat(TokenKind::LeftBracket) {
                let key = self.parse_expr()?;
                self.expect(TokenKind::RightBracket)?;
                self.expect(TokenKind::Equal)?;
                let value = self.parse_expr()?;
                let span = field_open.span.join(&value.span);
                TableField::new(TableFieldKind::Indexed { key, value }, span)
            } else if self.at(TokenKind::Name) && self.nth_kind(1) == Some(TokenKind::Equal) {
                let name = self.expect_name()?;
                self.expect(TokenKind::Equal)?;
                let value = self.parse_expr()?;
                let span = name.span.join(&value.span);
                TableField::new(TableFieldKind::Named { name, value }, span)
            } else {
                let value = self.parse_expr()?;
                let span = value.span;
                TableField::new(TableFieldKind::Value(value), span)
            };
            fields.push(field);

            if self.eat(TokenKind::Comma).is_none() && self.eat(TokenKind::Semicolon).is_none() {
                break;
            }
        }

        let close = self.expect(TokenKind::RightBrace)?;
        Ok(Expr::new(
            ExprKind::Table(fields),
            open.span.join(&close.span),
        ))
    }

    fn unary_operator(&self) -> Option<UnaryOperator> {
        match self.kind()? {
            TokenKind::Minus => Some(UnaryOperator::Negate),
            TokenKind::Not => Some(UnaryOperator::Not),
            TokenKind::Hash => Some(UnaryOperator::Length),
            TokenKind::Tilde => Some(UnaryOperator::BitwiseNot),
            _ => None,
        }
    }

    fn binary_operator(&self) -> Option<(BinaryOperator, u8, u8)> {
        let (operator, precedence, right_associative) = match self.kind()? {
            TokenKind::Or => (BinaryOperator::Or, 1, false),
            TokenKind::And => (BinaryOperator::And, 2, false),
            TokenKind::Less => (BinaryOperator::LessThan, 3, false),
            TokenKind::LessEqual => (BinaryOperator::LessThanOrEqual, 3, false),
            TokenKind::Greater => (BinaryOperator::GreaterThan, 3, false),
            TokenKind::GreaterEqual => (BinaryOperator::GreaterThanOrEqual, 3, false),
            TokenKind::EqualEqual => (BinaryOperator::Equal, 3, false),
            TokenKind::TildeEqual => (BinaryOperator::NotEqual, 3, false),
            TokenKind::Pipe => (BinaryOperator::BitwiseOr, 4, false),
            TokenKind::Tilde => (BinaryOperator::BitwiseXor, 5, false),
            TokenKind::Ampersand => (BinaryOperator::BitwiseAnd, 6, false),
            TokenKind::ShiftLeft => (BinaryOperator::ShiftLeft, 7, false),
            TokenKind::ShiftRight => (BinaryOperator::ShiftRight, 7, false),
            TokenKind::DotDot => (BinaryOperator::Concat, 8, true),
            TokenKind::Plus => (BinaryOperator::Add, 9, false),
            TokenKind::Minus => (BinaryOperator::Subtract, 9, false),
            TokenKind::Star => (BinaryOperator::Multiply, 10, false),
            TokenKind::Slash => (BinaryOperator::Divide, 10, false),
            TokenKind::SlashSlash => (BinaryOperator::FloorDivide, 10, false),
            TokenKind::Percent => (BinaryOperator::Modulo, 10, false),
            TokenKind::Caret => (BinaryOperator::Power, 12, true),
            _ => return None,
        };
        let right_power = if right_associative {
            precedence
        } else {
            precedence + 1
        };
        Some((operator, precedence, right_power))
    }

    fn can_start_expr(&self) -> bool {
        matches!(
            self.kind(),
            Some(
                TokenKind::Nil
                    | TokenKind::False
                    | TokenKind::True
                    | TokenKind::Number
                    | TokenKind::String
                    | TokenKind::Ellipsis
                    | TokenKind::Function
                    | TokenKind::LeftBrace
                    | TokenKind::Name
                    | TokenKind::LeftParen
                    | TokenKind::Minus
                    | TokenKind::Not
                    | TokenKind::Hash
                    | TokenKind::Tilde
            )
        )
    }

    fn current(&self) -> Option<&Spanned<Token>> {
        self.tokens.front()
    }

    fn kind(&self) -> Option<TokenKind> {
        self.current().map(|token| token.value.discriminant())
    }

    fn nth_kind(&self, offset: usize) -> Option<TokenKind> {
        self.tokens
            .get(offset)
            .map(|token| token.value.discriminant())
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.kind() == Some(kind)
    }

    fn at_any(&self, kinds: &[TokenKind]) -> bool {
        self.kind().is_some_and(|kind| kinds.contains(&kind))
    }

    fn at_eof(&self) -> bool {
        self.current().is_none() || self.at(TokenKind::Eof)
    }

    fn advance(&mut self) -> Option<Spanned<Token>> {
        self.tokens.pop_front()
    }

    fn eat(&mut self, kind: TokenKind) -> Option<Spanned<Token>> {
        self.at(kind).then(|| self.advance().unwrap())
    }

    fn expect(&mut self, kind: TokenKind) -> ParseResult<Spanned<Token>> {
        if self.at(kind) {
            Ok(self.advance().unwrap())
        } else {
            Err(self.expected(kind))
        }
    }

    fn expect_name(&mut self) -> ParseResult<Spanned<Symbol>> {
        match self.advance() {
            Some(Spanned {
                value: Token::Name(value),
                span,
            }) => Ok(Spanned { value, span }),
            Some(Spanned { value, span }) => Err(self.error_at(
                ParseErrorKind::ExpectedToken {
                    expected: TokenKind::Name,
                    actual: Some(value.discriminant()),
                },
                span,
            )),
            None => Err(self.error_at(
                ParseErrorKind::ExpectedToken {
                    expected: TokenKind::Name,
                    actual: None,
                },
                self.eof_span(),
            )),
        }
    }

    fn expected(&self, expected: TokenKind) -> ParseError {
        self.error(ParseErrorKind::ExpectedToken {
            expected,
            actual: self.kind(),
        })
    }

    fn span(&self) -> Span {
        self.current()
            .map(|token| token.span)
            .unwrap_or_else(|| self.eof_span())
    }

    fn error(&self, kind: ParseErrorKind) -> ParseError {
        self.error_at(kind, self.span())
    }

    fn error_at(&self, kind: ParseErrorKind, span: Span) -> ParseError {
        ParseError { kind, span }
    }

    fn eof_span(&self) -> Span {
        Span::new(self.source_id, self.source_end, self.source_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    const SOURCE_ID: SourceId = SourceId::new(11);

    fn parse(source: &str) -> Chunk {
        let tokens = lex(SOURCE_ID, source).expect("source should lex");
        parse_chunk(SOURCE_ID, tokens).expect("source should parse")
    }

    #[test]
    fn parses_the_initial_vertical_slice() {
        let chunk = parse("local function add(a) return a + 2 * 3 end");
        let StmtKind::LocalFunction { name, body } = &chunk.statements[0].kind else {
            panic!("expected local function")
        };

        assert_eq!(name.value.as_str(), "add");
        assert_eq!(body.parameters[0].value.as_str(), "a");
        let returned = &body.body.return_statement.as_ref().unwrap().values[0];
        let ExprKind::Binary {
            operator: BinaryOperator::Add,
            right,
            ..
        } = &returned.kind
        else {
            panic!("expected addition")
        };
        assert!(matches!(
            right.kind,
            ExprKind::Binary {
                operator: BinaryOperator::Multiply,
                ..
            }
        ));
    }

    #[test]
    fn respects_lua_associativity_and_unary_power_precedence() {
        let chunk = parse("return -2^2, 'a'..'b'..'c'");
        let values = &chunk.return_statement.as_ref().unwrap().values;

        let ExprKind::Unary {
            operator: UnaryOperator::Negate,
            expression,
        } = &values[0].kind
        else {
            panic!("expected negation")
        };
        assert!(matches!(
            expression.kind,
            ExprKind::Binary {
                operator: BinaryOperator::Power,
                ..
            }
        ));

        let ExprKind::Binary {
            operator: BinaryOperator::Concat,
            right,
            ..
        } = &values[1].kind
        else {
            panic!("expected concatenation")
        };
        assert!(matches!(
            right.kind,
            ExprKind::Binary {
                operator: BinaryOperator::Concat,
                ..
            }
        ));
    }

    #[test]
    fn parses_control_flow_and_for_loops() {
        let chunk = parse(
            "while ready do break end \
             repeat tick() until done \
             if a then f() elseif b then g() else h() end \
             for i = 1, 10, 2 do sum = sum + i end \
             for k, v in pairs(t) do use(k, v) end",
        );

        assert!(matches!(chunk.statements[0].kind, StmtKind::While { .. }));
        assert!(matches!(chunk.statements[1].kind, StmtKind::Repeat { .. }));
        assert!(matches!(chunk.statements[2].kind, StmtKind::If { .. }));
        assert!(matches!(
            chunk.statements[3].kind,
            StmtKind::NumericFor { .. }
        ));
        assert!(matches!(
            chunk.statements[4].kind,
            StmtKind::GenericFor { .. }
        ));
    }

    #[test]
    fn parses_calls_assignments_tables_and_functions() {
        let chunk = parse(
            "obj.field, t[key] = make { named = 1, [key] = 2, 3 }, function(x, ...) return x end \
             obj:method 'argument'",
        );

        let StmtKind::Assign { targets, values } = &chunk.statements[0].kind else {
            panic!("expected assignment")
        };
        assert!(matches!(
            targets[0].kind,
            AssignmentTargetKind::Field { .. }
        ));
        assert!(matches!(
            targets[1].kind,
            AssignmentTargetKind::Index { .. }
        ));
        assert!(matches!(values[0].kind, ExprKind::Call(_)));
        assert!(matches!(values[1].kind, ExprKind::Function(_)));
        assert!(matches!(
            chunk.statements[1].kind,
            StmtKind::Call(Call::Method { .. })
        ));
    }

    #[test]
    fn parses_the_remaining_operators() {
        let chunk = parse(
            "return not false or true and 1 < 2, \
             #items, 7 // 2 % 3, 1 << 2 >> 1 | 3 ~ 4 & 5, \
             a <= b, a >= b, a == b, a ~= b",
        );
        let values = &chunk.return_statement.as_ref().unwrap().values;

        assert_eq!(values.len(), 8);
        assert!(matches!(
            values[0].kind,
            ExprKind::Binary {
                operator: BinaryOperator::Or,
                ..
            }
        ));
        assert!(matches!(
            values[1].kind,
            ExprKind::Unary {
                operator: UnaryOperator::Length,
                ..
            }
        ));
        assert!(matches!(
            values[7].kind,
            ExprKind::Binary {
                operator: BinaryOperator::NotEqual,
                ..
            }
        ));
    }

    #[test]
    fn parses_named_method_functions_and_chained_prefixes() {
        let chunk = parse(
            "function module.sub:method(a, ...) \
             return (factory())(a).value \
             end",
        );
        let StmtKind::Function { name, body } = &chunk.statements[0].kind else {
            panic!("expected named function")
        };

        assert_eq!(name.name.value.as_str(), "module");
        assert_eq!(name.fields[0].value.as_str(), "sub");
        assert_eq!(name.method.as_ref().unwrap().value.as_str(), "method");
        assert!(body.is_variadic);
        assert!(matches!(
            body.body.return_statement.as_ref().unwrap().values[0].kind,
            ExprKind::Field { .. }
        ));
    }

    #[test]
    fn parses_local_attributes_labels_and_goto() {
        let chunk = parse("local x <const>, y <close> = 1, resource ::again:: goto again");
        let StmtKind::Local { names, .. } = &chunk.statements[0].kind else {
            panic!("expected local declaration")
        };

        assert_eq!(
            names[0].attribute.as_ref().unwrap().value,
            LocalAttribute::Const
        );
        assert_eq!(
            names[1].attribute.as_ref().unwrap().value,
            LocalAttribute::Close
        );
        assert!(matches!(chunk.statements[1].kind, StmtKind::Label(_)));
        assert!(matches!(chunk.statements[2].kind, StmtKind::Goto(_)));
    }

    #[test]
    fn rejects_unknown_local_attributes_with_their_name() {
        let tokens = lex(SOURCE_ID, "local x <XXX> = 10").unwrap();
        let error = parse_chunk(SOURCE_ID, tokens).unwrap_err();

        assert_eq!(
            error.kind,
            ParseErrorKind::InvalidLocalAttribute {
                attribute: Symbol::from("XXX")
            }
        );
        assert_eq!(error.to_string(), "unknown attribute 'XXX'");
        assert_eq!(error.span, Span::new(SOURCE_ID, 9, 12));
    }

    #[test]
    fn rejects_non_terminal_returns() {
        let tokens = lex(SOURCE_ID, "return 1 local x = 2").unwrap();
        let error = parse_chunk(SOURCE_ID, tokens).unwrap_err();

        assert_eq!(error.kind, ParseErrorKind::StatementAfterReturn);
        assert_eq!(error.span.start, 9);
    }

    #[test]
    fn rejects_non_call_expression_statements_and_bad_targets() {
        for (source, expected) in [
            ("value", ParseErrorKind::ExpectedCallOrAssignment),
            ("f() = 1", ParseErrorKind::InvalidAssignmentTarget),
        ] {
            let tokens = lex(SOURCE_ID, source).unwrap();
            assert_eq!(parse_chunk(SOURCE_ID, tokens).unwrap_err().kind, expected);
        }
    }
}
