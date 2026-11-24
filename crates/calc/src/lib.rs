#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! The calculation language.
//!
//! A calculation is a small typed expression language that is parsed, checked against the fields
//! a model exposes, and lowered to SQL. It exists so that a calculation fails in the editor with
//! a span, rather than in the database with an opaque message.
//!
//! ```
//! use slickbi_calc::{FieldBinding, FieldResolver, ValueType, compile};
//!
//! struct Fields(Vec<FieldBinding>);
//! impl FieldResolver for Fields {
//!     fn resolve_field(&self, name: &str) -> Option<&FieldBinding> {
//!         self.0.iter().find(|field| field.name == name)
//!     }
//! }
//!
//! let fields = Fields(vec![
//!     FieldBinding::new("Revenue", "\"revenue\"", ValueType::Number),
//!     FieldBinding::new("Units", "\"units\"", ValueType::Integer),
//! ]);
//!
//! let compiled = compile("SUM([Revenue]) / COUNT([Units])", &fields).expect("valid");
//! assert_eq!(compiled.sql, "(sum(\"revenue\") / count(\"units\"))");
//! assert_eq!(compiled.value_type, ValueType::Number);
//! assert!(compiled.aggregate);
//! ```

mod lexer;
mod lower;
mod parser;

use std::fmt;

use thiserror::Error;

pub use lexer::Span;
pub use parser::Expr;

/// The type of a value inside a calculation.
///
/// This is deliberately narrower than the SQL type system: a calculation can only produce one of
/// these, and [`ValueType::Null`] is the type of the untyped null literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    /// A whole number.
    Integer,
    /// A number with a fractional part.
    Number,
    /// Text.
    Text,
    /// A boolean.
    Boolean,
    /// A calendar date.
    Date,
    /// A date with a time.
    Timestamp,
    /// The type of the untyped null literal.
    Null,
}

impl ValueType {
    /// The SQL type this value maps to.
    pub fn sql_name(self) -> &'static str {
        match self {
            ValueType::Integer => "BIGINT",
            ValueType::Number => "DOUBLE",
            ValueType::Text => "VARCHAR",
            ValueType::Boolean => "BOOLEAN",
            ValueType::Date => "DATE",
            ValueType::Timestamp => "TIMESTAMP",
            ValueType::Null => "NULL",
        }
    }

    /// Whether this is a numeric type.
    pub fn is_numeric(self) -> bool {
        matches!(self, ValueType::Integer | ValueType::Number)
    }

    /// Whether this is a date-like type.
    pub fn is_temporal(self) -> bool {
        matches!(self, ValueType::Date | ValueType::Timestamp)
    }

    /// The unifies two branch types, or reports a mismatch.
    pub(crate) fn unify(self, other: ValueType) -> Option<ValueType> {
        match (self, other) {
            (ValueType::Null, other) | (other, ValueType::Null) => Some(other),
            (left, right) if left == right => Some(left),
            (left, right) if left.is_numeric() && right.is_numeric() => Some(ValueType::Number),
            (left, right) if left.is_temporal() && right.is_temporal() => {
                Some(ValueType::Timestamp)
            }
            _ => None,
        }
    }

    /// The document-model type this maps to.
    ///
    /// [`ValueType::Null`] has no field type, so it maps to `None`.
    pub fn data_type(self) -> Option<slickbi_spec::DataType> {
        use slickbi_spec::DataType;
        match self {
            ValueType::Integer => Some(DataType::Integer),
            ValueType::Number => Some(DataType::Number),
            ValueType::Text => Some(DataType::Text),
            ValueType::Boolean => Some(DataType::Boolean),
            ValueType::Date => Some(DataType::Date),
            ValueType::Timestamp => Some(DataType::Timestamp),
            ValueType::Null => None,
        }
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ValueType::Integer => "integer",
            ValueType::Number => "number",
            ValueType::Text => "text",
            ValueType::Boolean => "boolean",
            ValueType::Date => "date",
            ValueType::Timestamp => "timestamp",
            ValueType::Null => "null",
        };
        f.write_str(name)
    }
}

/// One field a calculation may reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldBinding {
    /// The name used inside brackets, for example `[Revenue]`.
    pub name: String,
    /// The SQL expression the field lowers to.
    pub sql: String,
    /// The field's type.
    pub value_type: ValueType,
}

impl FieldBinding {
    /// Creates a binding.
    pub fn new(name: impl Into<String>, sql: impl Into<String>, value_type: ValueType) -> Self {
        Self {
            name: name.into(),
            sql: sql.into(),
            value_type,
        }
    }
}

/// Resolves field names to bindings for one model.
pub trait FieldResolver {
    /// Returns the binding for a field name, or `None` when the model has no such field.
    fn resolve_field(&self, name: &str) -> Option<&FieldBinding>;
}

/// A literal value captured while lowering an expression.
///
/// Values are bound as query parameters rather than interpolated, so text can never break out of
/// the generated SQL.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// A null literal. Rarely bound, since `NULL` is emitted directly.
    Null,
    /// A whole number.
    Integer(i64),
    /// A number.
    Number(f64),
    /// Text.
    Text(String),
    /// A boolean.
    Boolean(bool),
}

/// Ordered query parameters produced by lowering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params {
    values: Vec<ParamValue>,
}

impl Params {
    /// Records a value and returns its zero-based position.
    pub fn push(&mut self, value: ParamValue) -> usize {
        self.values.push(value);
        self.values.len() - 1
    }

    /// Returns the recorded values in order.
    pub fn values(&self) -> &[ParamValue] {
        &self.values
    }

    /// Returns the number of recorded values.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Reports whether no values were recorded.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// A checked expression lowered to SQL.
#[derive(Debug, Clone, PartialEq)]
pub struct Sql {
    /// The SQL text, with `?` placeholders for bound values.
    pub text: String,
    /// The type of the expression.
    pub value_type: ValueType,
    /// Whether the expression contains an aggregate function.
    pub aggregate: bool,
}

/// A fully compiled calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    /// The SQL text, with `?` placeholders for bound values.
    pub sql: String,
    /// The type of the expression.
    pub value_type: ValueType,
    /// Whether the expression contains an aggregate function.
    pub aggregate: bool,
    /// The values for the placeholders, in order.
    pub params: Vec<ParamValue>,
}

/// Anything that can go wrong while compiling a calculation.
#[derive(Debug, Error)]
pub enum CalcError {
    /// The text could not be tokenised.
    #[error("{0}")]
    Lex(#[from] lexer::LexError),
    /// The tokens do not form an expression.
    #[error("{0}")]
    Parse(#[from] parser::ParseError),
    /// A bracketed name does not match any field.
    #[error("unknown field '[{name}]'")]
    UnknownField {
        /// The name as written.
        name: String,
        /// Where it was written.
        span: Span,
    },
    /// A function name is not part of the language.
    #[error("unknown function '{name}'")]
    UnknownFunction {
        /// The name as written.
        name: String,
        /// Where it was written.
        span: Span,
    },
    /// A function was called with the wrong number of arguments.
    #[error("'{function}' takes {expected} argument(s), but {found} were given")]
    WrongArgumentCount {
        /// The function.
        function: String,
        /// A description of the expected count.
        expected: &'static str,
        /// The count supplied.
        found: usize,
        /// The call site.
        span: Span,
    },
    /// An argument has the wrong type.
    #[error("'{function}' argument {argument} must be {expected}, but is {found}")]
    ArgumentType {
        /// The function.
        function: String,
        /// The one-based argument position.
        argument: usize,
        /// A description of the expected type.
        expected: &'static str,
        /// The type supplied.
        found: ValueType,
        /// The argument's span.
        span: Span,
    },
    /// An operator does not accept these operand types.
    #[error("'{operator}' cannot be applied to {left} and {right}")]
    InvalidOperand {
        /// The operator.
        operator: &'static str,
        /// The left type.
        left: ValueType,
        /// The right type.
        right: ValueType,
        /// The expression span.
        span: Span,
    },
    /// A condition is not boolean.
    #[error("a condition must be boolean, but this is {found}")]
    ExpectedBoolean {
        /// The type supplied.
        found: ValueType,
        /// The condition's span.
        span: Span,
    },
    /// The two branches of a conditional have incompatible types.
    #[error(
        "the branches of this condition are {then_type} and {else_type}, which cannot be combined"
    )]
    IncompatibleBranches {
        /// The `then` type.
        then_type: ValueType,
        /// The `else` type.
        else_type: ValueType,
        /// The condition's span.
        span: Span,
    },
    /// Aggregates and row-level fields appear in the same expression.
    #[error(
        "this expression mixes an aggregate with a row-level field; put every field inside an aggregate"
    )]
    MixedAggregateContext {
        /// The offending field.
        span: Span,
    },
    /// An aggregate was nested inside another aggregate.
    #[error("an aggregate cannot be nested inside another aggregate")]
    NestedAggregate {
        /// The inner aggregate.
        span: Span,
    },
    /// A function received a value it cannot accept, such as an unknown date unit.
    #[error("'{function}': {message}")]
    InvalidArgument {
        /// The function.
        function: String,
        /// What is wrong.
        message: String,
        /// The argument's span.
        span: Span,
    },
}

impl CalcError {
    /// The source span this error refers to, when it has one.
    pub fn span(&self) -> Option<Span> {
        match self {
            CalcError::Lex(error) => Some(error.span()),
            CalcError::Parse(error) => Some(error.span()),
            CalcError::UnknownField { span, .. }
            | CalcError::UnknownFunction { span, .. }
            | CalcError::WrongArgumentCount { span, .. }
            | CalcError::ArgumentType { span, .. }
            | CalcError::InvalidOperand { span, .. }
            | CalcError::ExpectedBoolean { span, .. }
            | CalcError::IncompatibleBranches { span, .. }
            | CalcError::MixedAggregateContext { span }
            | CalcError::NestedAggregate { span }
            | CalcError::InvalidArgument { span, .. } => Some(*span),
        }
    }
}

/// Parses a calculation into an expression tree.
pub fn parse(source: &str) -> Result<Expr, CalcError> {
    let tokens = lexer::tokenize(source)?;
    Ok(parser::parse(tokens)?)
}

/// Type-checks an expression against the fields a model exposes.
pub fn check(expr: &Expr, fields: &dyn FieldResolver) -> Result<Sql, CalcError> {
    let mut params = Params::default();
    lower::lower(expr, fields, &mut params)
}

/// Lowers an expression to SQL, recording bound values in `params`.
pub fn lower(
    expr: &Expr,
    fields: &dyn FieldResolver,
    params: &mut Params,
) -> Result<Sql, CalcError> {
    lower::lower(expr, fields, params)
}

/// Parses, checks and lowers a calculation in one step.
pub fn compile(source: &str, fields: &dyn FieldResolver) -> Result<Compiled, CalcError> {
    let expr = parse(source)?;
    let mut params = Params::default();
    let sql = lower::lower(&expr, fields, &mut params)?;
    Ok(Compiled {
        sql: sql.text,
        value_type: sql.value_type,
        aggregate: sql.aggregate,
        params: params.values,
    })
}

#[cfg(test)]
pub(crate) mod test_fields {
    use super::{FieldBinding, FieldResolver, ValueType};

    /// A resolver used across the unit tests.
    pub struct Fields(pub Vec<FieldBinding>);

    impl FieldResolver for Fields {
        fn resolve_field(&self, name: &str) -> Option<&FieldBinding> {
            self.0.iter().find(|field| field.name == name)
        }
    }

    /// The standard fixture model.
    pub fn fields() -> Fields {
        Fields(vec![
            FieldBinding::new("Revenue", "\"revenue\"", ValueType::Number),
            FieldBinding::new("Units", "\"units\"", ValueType::Integer),
            FieldBinding::new("Region", "\"region\"", ValueType::Text),
            FieldBinding::new("OrderDate", "\"order_date\"", ValueType::Date),
            FieldBinding::new("Active", "\"active\"", ValueType::Boolean),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::test_fields::{Fields, fields};
    use super::*;

    #[test]
    fn compiles_a_ratio_of_aggregates() {
        let compiled = compile("SUM([Revenue]) / COUNT([Units])", &fields()).expect("valid");
        assert_eq!(compiled.sql, "(sum(\"revenue\") / count(\"units\"))");
        assert_eq!(compiled.value_type, ValueType::Number);
        assert!(compiled.aggregate);
        assert!(compiled.params.is_empty());
    }

    #[test]
    fn binds_literals_as_parameters() {
        let compiled = compile("[Revenue] * 1.2", &fields()).expect("valid");
        assert_eq!(compiled.sql, "(\"revenue\" * ?)");
        assert_eq!(compiled.params, vec![ParamValue::Number(1.2)]);
        assert!(!compiled.aggregate);
    }

    #[test]
    fn reports_unknown_fields_with_a_span() {
        let error = compile("[Profit] + 1", &fields()).expect_err("must fail");
        match error {
            CalcError::UnknownField { name, span } => {
                assert_eq!(name, "Profit");
                assert_eq!(span.start, 0);
                assert_eq!(span.end, 8);
            }
            other => panic!("expected an unknown field, got {other}"),
        }
    }

    #[test]
    fn rejects_mixed_aggregate_context() {
        let error = compile("SUM([Revenue]) + [Units]", &fields()).expect_err("must fail");
        assert!(matches!(error, CalcError::MixedAggregateContext { .. }));
    }

    #[test]
    fn rejects_nested_aggregates() {
        let error = compile("SUM(AVG([Revenue]))", &fields()).expect_err("must fail");
        assert!(matches!(error, CalcError::NestedAggregate { .. }));
    }

    #[test]
    fn spans_point_at_the_offending_text() {
        let source = "1 + [Nope]";
        let error = compile(source, &fields()).expect_err("must fail");
        let span = error.span().expect("a span");
        assert_eq!(&source[span.start..span.end], "[Nope]");
    }

    #[test]
    fn empty_input_is_a_parse_error() {
        let error = compile("   ", &fields()).expect_err("must fail");
        assert!(matches!(error, CalcError::Parse(_)));
    }

    #[test]
    fn a_field_name_may_contain_spaces_and_brackets() {
        let fields = Fields(vec![FieldBinding::new(
            "Net Revenue (USD)",
            "\"net\"",
            ValueType::Number,
        )]);
        let compiled = compile("[Net Revenue (USD)] * 2", &fields).expect("valid");
        assert_eq!(compiled.sql, "(\"net\" * ?)");
    }
}
