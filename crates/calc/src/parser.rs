//! Recursive-descent parser for the calculation language.
//!
//! The grammar, loosest binding first:
//!
//! ```text
//! expression    := or
//! or            := and ( OR and )*
//! and           := not ( AND not )*
//! not           := NOT not | comparison
//! comparison    := additive ( ( = | <> | < | <= | > | >= ) additive )?
//! additive      := multiplicative ( ( + | - ) multiplicative )*
//! multiplicative:= unary ( ( * | / | % ) unary )*
//! unary         := ( - | + ) unary | primary
//! primary       := literal | [Field] | IDENT '(' arguments ')' | '(' expression ')'
//!                | IF expression THEN expression ( ELSE expression )? END
//!                | CASE ( WHEN expression THEN expression )+ ( ELSE expression )? END
//! arguments     := ( expression ( ',' expression )* )?
//! ```
//!
//! `CASE` is parsed into nested `IF` nodes, so there is one conditional shape downstream.

use std::fmt;

use thiserror::Error;

use crate::lexer::{Keyword, Span, Token, TokenKind};

/// An expression node and the source it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// What the node is.
    pub kind: ExprKind,
    /// Where the node appears in the source.
    pub span: Span,
}

/// The shape of an expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A whole-number literal.
    Integer(i64),
    /// A decimal literal.
    Number(f64),
    /// A text literal.
    Text(String),
    /// A boolean literal.
    Boolean(bool),
    /// The untyped null literal.
    Null,
    /// A reference to a field by name.
    Field {
        /// The name between the brackets.
        name: String,
    },
    /// A prefix operator.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// Its operand.
        operand: Box<Expr>,
    },
    /// An infix operator.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        left: Box<Expr>,
        /// The right operand.
        right: Box<Expr>,
    },
    /// A function call.
    Call {
        /// The function name, uppercased.
        name: String,
        /// The arguments, in order.
        args: Vec<Expr>,
    },
    /// A conditional with an `ELSE` branch; a missing branch is a null literal.
    If {
        /// The condition, which must be boolean.
        condition: Box<Expr>,
        /// The value when the condition holds.
        then_branch: Box<Expr>,
        /// The value otherwise.
        else_branch: Box<Expr>,
    },
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Arithmetic negation.
    Negate,
    /// Logical negation.
    Not,
}

impl UnaryOp {
    /// The operator as written.
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOp::Negate => "-",
            UnaryOp::Not => "NOT",
        }
    }
}

/// An infix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `/`
    Divide,
    /// `%`
    Modulo,
    /// `=`
    Equal,
    /// `<>`
    NotEqual,
    /// `<`
    Less,
    /// `<=`
    LessEqual,
    /// `>`
    Greater,
    /// `>=`
    GreaterEqual,
    /// `AND`
    And,
    /// `OR`
    Or,
}

impl BinaryOp {
    /// The operator as written in the language.
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Modulo => "%",
            BinaryOp::Equal => "=",
            BinaryOp::NotEqual => "<>",
            BinaryOp::Less => "<",
            BinaryOp::LessEqual => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterEqual => ">=",
            BinaryOp::And => "AND",
            BinaryOp::Or => "OR",
        }
    }
}

/// A failure while parsing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    /// The input contains no expression.
    #[error("the expression is empty")]
    Empty {
        /// Where the input ends.
        span: Span,
    },
    /// A token does not fit the grammar.
    #[error("expected {expected} but found {found}")]
    UnexpectedToken {
        /// What the grammar required.
        expected: &'static str,
        /// What was there instead.
        found: String,
        /// Where it was.
        span: Span,
    },
    /// The input ends before the expression does.
    #[error("expected {expected} but the expression ended")]
    UnexpectedEnd {
        /// What the grammar required.
        expected: &'static str,
        /// Where the input ends.
        span: Span,
    },
}

impl ParseError {
    /// The source span this error refers to.
    pub fn span(&self) -> Span {
        match self {
            ParseError::Empty { span }
            | ParseError::UnexpectedToken { span, .. }
            | ParseError::UnexpectedEnd { span, .. } => *span,
        }
    }
}

/// Parses a token stream into an expression tree.
pub fn parse(tokens: Vec<Token>) -> Result<Expr, ParseError> {
    let mut parser = Parser {
        tokens,
        position: 0,
    };

    if matches!(parser.peek_kind(), TokenKind::Eof) {
        return Err(ParseError::Empty {
            span: parser.peek().span,
        });
    }

    let expression = parser.expression()?;

    if !matches!(parser.peek_kind(), TokenKind::Eof) {
        return Err(ParseError::UnexpectedToken {
            expected: "the end of the expression",
            found: parser.peek_kind().describe(),
            span: parser.peek().span,
        });
    }

    Ok(expression)
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ExprKind::Integer(value) => write!(f, "{value}"),
            ExprKind::Number(value) => write!(f, "{value}"),
            ExprKind::Text(value) => write!(f, "'{}'", value.replace('\'', "''")),
            ExprKind::Boolean(value) => write!(f, "{value}"),
            ExprKind::Null => f.write_str("NULL"),
            ExprKind::Field { name } => write!(f, "[{name}]"),
            ExprKind::Unary { op, operand } => write!(f, "{} {operand}", op.as_str()),
            ExprKind::Binary { op, left, right } => {
                write!(f, "({left} {} {right})", op.as_str())
            }
            ExprKind::Call { name, args } => {
                let args = args
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{name}({args})")
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => write!(
                f,
                "IF {condition} THEN {then_branch} ELSE {else_branch} END"
            ),
        }
    }
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn expression(&mut self) -> Result<Expr, ParseError> {
        self.or_expression()
    }

    fn or_expression(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.and_expression()?;
        while self.keyword() == Some(Keyword::Or) {
            self.advance();
            let right = self.and_expression()?;
            left = binary(BinaryOp::Or, left, right);
        }
        Ok(left)
    }

    fn and_expression(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.not_expression()?;
        while self.keyword() == Some(Keyword::And) {
            self.advance();
            let right = self.not_expression()?;
            left = binary(BinaryOp::And, left, right);
        }
        Ok(left)
    }

    fn not_expression(&mut self) -> Result<Expr, ParseError> {
        if self.keyword() == Some(Keyword::Not) {
            let start = self.advance().span;
            let operand = self.not_expression()?;
            let span = start.merge(operand.span);
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                },
                span,
            });
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.additive()?;

        let Some(operator) = self.comparison_operator() else {
            return Ok(left);
        };
        self.advance();

        let right = self.additive()?;

        if self.comparison_operator().is_some() {
            return Err(ParseError::UnexpectedToken {
                expected: "a single comparison; use AND to combine conditions",
                found: self.peek_kind().describe(),
                span: self.peek().span,
            });
        }

        Ok(binary(operator, left, right))
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.multiplicative()?;
        loop {
            let operator = match self.peek_kind() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Subtract,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.multiplicative()?;
            left = binary(operator, left, right);
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.unary()?;
        loop {
            let operator = match self.peek_kind() {
                TokenKind::Star => BinaryOp::Multiply,
                TokenKind::Slash => BinaryOp::Divide,
                TokenKind::Percent => BinaryOp::Modulo,
                _ => return Ok(left),
            };
            self.advance();
            let right = self.unary()?;
            left = binary(operator, left, right);
        }
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                let start = self.advance().span;
                let operand = self.unary()?;
                let span = start.merge(operand.span);
                Ok(Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Negate,
                        operand: Box::new(operand),
                    },
                    span,
                })
            }
            TokenKind::Plus => {
                self.advance();
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let token = self.peek().clone();

        let expression = match token.kind {
            TokenKind::Integer(value) => expression(ExprKind::Integer(value), token.span),
            TokenKind::Number(value) => expression(ExprKind::Number(value), token.span),
            TokenKind::Text(value) => expression(ExprKind::Text(value), token.span),
            TokenKind::Field(name) => expression(ExprKind::Field { name }, token.span),
            TokenKind::Keyword(Keyword::True) => expression(ExprKind::Boolean(true), token.span),
            TokenKind::Keyword(Keyword::False) => expression(ExprKind::Boolean(false), token.span),
            TokenKind::Keyword(Keyword::Null) => expression(ExprKind::Null, token.span),
            TokenKind::Keyword(Keyword::If) => return self.if_expression(),
            TokenKind::Keyword(Keyword::Case) => return self.case_expression(),
            TokenKind::Ident(name) => return self.call(name, token.span),
            TokenKind::LParen => {
                self.advance();
                let inner = self.expression()?;
                self.expect_rparen()?;
                return Ok(inner);
            }
            _ => {
                return Err(ParseError::UnexpectedToken {
                    expected: "a value",
                    found: token.kind.describe(),
                    span: token.span,
                });
            }
        };

        self.advance();
        Ok(expression)
    }

    fn call(&mut self, name: String, start: Span) -> Result<Expr, ParseError> {
        self.advance();

        if !matches!(self.peek_kind(), TokenKind::LParen) {
            return Err(ParseError::UnexpectedToken {
                expected: "'(' after a function name; fields are written [Like This]",
                found: self.peek_kind().describe(),
                span: self.peek().span,
            });
        }
        self.advance();

        let mut args = Vec::new();
        if !matches!(self.peek_kind(), TokenKind::RParen) {
            loop {
                args.push(self.expression()?);
                if matches!(self.peek_kind(), TokenKind::Comma) {
                    self.advance();
                    continue;
                }
                break;
            }
        }

        let end = self.expect_rparen()?.span;
        Ok(Expr {
            kind: ExprKind::Call { name, args },
            span: start.merge(end),
        })
    }

    fn if_expression(&mut self) -> Result<Expr, ParseError> {
        let start = self.advance().span;

        let condition = self.expression()?;
        self.expect_keyword(Keyword::Then)?;
        let then_branch = self.expression()?;

        let else_branch = if self.keyword() == Some(Keyword::Else) {
            self.advance();
            self.expression()?
        } else {
            expression(ExprKind::Null, then_branch.span)
        };

        let end = self.expect_keyword(Keyword::End)?.span;

        Ok(Expr {
            kind: ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            },
            span: start.merge(end),
        })
    }

    fn case_expression(&mut self) -> Result<Expr, ParseError> {
        let start = self.advance().span;

        if self.keyword() != Some(Keyword::When) {
            return Err(ParseError::UnexpectedToken {
                expected: "'WHEN' (only searched CASE is supported)",
                found: self.peek_kind().describe(),
                span: self.peek().span,
            });
        }

        let mut branches = Vec::new();
        while self.keyword() == Some(Keyword::When) {
            self.advance();
            let condition = self.expression()?;
            self.expect_keyword(Keyword::Then)?;
            let value = self.expression()?;
            branches.push((condition, value));
        }

        let fallback = if self.keyword() == Some(Keyword::Else) {
            self.advance();
            Some(self.expression()?)
        } else {
            None
        };

        let end = self.expect_keyword(Keyword::End)?.span;
        let span = start.merge(end);

        let mut result = fallback.unwrap_or_else(|| expression(ExprKind::Null, span));

        for (condition, value) in branches.into_iter().rev() {
            result = expression(
                ExprKind::If {
                    condition: Box::new(condition),
                    then_branch: Box::new(value),
                    else_branch: Box::new(result),
                },
                span,
            );
        }

        Ok(result)
    }

    fn expect_rparen(&mut self) -> Result<Token, ParseError> {
        if matches!(self.peek_kind(), TokenKind::RParen) {
            return Ok(self.advance());
        }
        Err(self.unexpected("')'"))
    }

    fn expect_keyword(&mut self, keyword: Keyword) -> Result<Token, ParseError> {
        if self.keyword() == Some(keyword) {
            return Ok(self.advance());
        }
        Err(self.unexpected(match keyword {
            Keyword::Then => "'THEN'",
            Keyword::Else => "'ELSE'",
            Keyword::End => "'END'",
            Keyword::When => "'WHEN'",
            _ => keyword.as_str(),
        }))
    }

    fn unexpected(&self, expected: &'static str) -> ParseError {
        let token = self.peek();
        if matches!(token.kind, TokenKind::Eof) {
            ParseError::UnexpectedEnd {
                expected,
                span: token.span,
            }
        } else {
            ParseError::UnexpectedToken {
                expected,
                found: token.kind.describe(),
                span: token.span,
            }
        }
    }

    fn comparison_operator(&self) -> Option<BinaryOp> {
        match self.peek_kind() {
            TokenKind::Equal => Some(BinaryOp::Equal),
            TokenKind::NotEqual => Some(BinaryOp::NotEqual),
            TokenKind::Less => Some(BinaryOp::Less),
            TokenKind::LessEqual => Some(BinaryOp::LessEqual),
            TokenKind::Greater => Some(BinaryOp::Greater),
            TokenKind::GreaterEqual => Some(BinaryOp::GreaterEqual),
            _ => None,
        }
    }

    fn keyword(&self) -> Option<Keyword> {
        match self.peek_kind() {
            TokenKind::Keyword(keyword) => Some(*keyword),
            _ => None,
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.position].clone();
        if self.position + 1 < self.tokens.len() {
            self.position += 1;
        }
        token
    }
}

fn expression(kind: ExprKind, span: Span) -> Expr {
    Expr { kind, span }
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    let span = left.span.merge(right.span);
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;

    fn parse_source(source: &str) -> Expr {
        parse(tokenize(source).expect("tokenises")).expect("parses")
    }

    fn fails(source: &str) -> ParseError {
        parse(tokenize(source).expect("tokenises")).expect_err("must fail")
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        let expr = parse_source("1 + 2 * 3");
        let ExprKind::Binary {
            op: BinaryOp::Add,
            right,
            ..
        } = &expr.kind
        else {
            panic!("expected addition at the root, got {expr:?}");
        };
        assert!(matches!(
            right.kind,
            ExprKind::Binary {
                op: BinaryOp::Multiply,
                ..
            }
        ));
    }

    #[test]
    fn comparison_binds_looser_than_arithmetic() {
        let expr = parse_source("1 + 2 < 4");
        assert!(matches!(
            expr.kind,
            ExprKind::Binary {
                op: BinaryOp::Less,
                ..
            }
        ));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        let expr = parse_source("[A] AND [B] OR [C]");
        assert!(matches!(
            expr.kind,
            ExprKind::Binary {
                op: BinaryOp::Or,
                ..
            }
        ));
    }

    #[test]
    fn not_binds_tighter_than_and() {
        let expr = parse_source("NOT TRUE AND FALSE");
        let ExprKind::Binary {
            op: BinaryOp::And,
            left,
            ..
        } = &expr.kind
        else {
            panic!("expected AND at the root, got {expr:?}");
        };
        assert!(matches!(
            left.kind,
            ExprKind::Unary {
                op: UnaryOp::Not,
                ..
            }
        ));
    }

    #[test]
    fn calls_may_have_no_arguments() {
        let expr = parse_source("TODAY()");
        match expr.kind {
            ExprKind::Call { name, args } => {
                assert_eq!(name, "TODAY");
                assert!(args.is_empty());
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_else_becomes_null() {
        let expr = parse_source("IF TRUE THEN 1 END");
        let ExprKind::If { else_branch, .. } = &expr.kind else {
            panic!("expected a conditional, got {expr:?}");
        };
        assert!(matches!(else_branch.kind, ExprKind::Null));
    }

    #[test]
    fn case_lowers_to_nested_conditionals() {
        let expr = parse_source("CASE WHEN TRUE THEN 1 WHEN FALSE THEN 2 ELSE 3 END");
        let ExprKind::If { else_branch, .. } = &expr.kind else {
            panic!("expected a conditional, got {expr:?}");
        };
        assert!(
            matches!(else_branch.kind, ExprKind::If { .. }),
            "the second WHEN must be in the ELSE branch"
        );
    }

    #[test]
    fn parentheses_do_not_change_the_span_of_the_inner_expression() {
        let source = "([Revenue])";
        let expr = parse_source(source);
        assert_eq!(&source[expr.span.start..expr.span.end], "[Revenue]");
    }

    #[test]
    fn a_condition_chain_is_rejected() {
        let error = fails("1 < 2 < 3");
        assert!(error.to_string().contains("single comparison"));
    }

    #[test]
    fn a_bare_identifier_is_not_a_field() {
        let error = fails("Revenue + 1");
        assert!(error.to_string().contains("fields are written [Like This]"));
    }

    #[test]
    fn a_missing_end_is_reported() {
        let error = fails("IF TRUE THEN 1");
        assert!(matches!(
            error,
            ParseError::UnexpectedEnd {
                expected: "'END'",
                ..
            }
        ));
    }

    #[test]
    fn trailing_tokens_are_rejected() {
        let error = fails("1 2");
        assert!(error.to_string().contains("end of the expression"));
    }

    #[test]
    fn an_empty_input_is_reported() {
        let error = fails("");
        assert!(matches!(error, ParseError::Empty { .. }));
    }

    #[test]
    fn the_root_span_covers_the_whole_expression() {
        let source = "SUM([Revenue]) / 2";
        let expr = parse_source(source);
        assert_eq!(expr.span.start, 0);
        assert_eq!(expr.span.end, source.len());
    }
}
