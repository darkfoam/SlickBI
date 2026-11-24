//! Type checking and lowering of expressions to SQL.
//!
//! One pass does both: the type of a node is computed while its SQL is built, so the two can
//! never disagree. Every literal becomes a `?` placeholder and its value is recorded in
//! [`Params`] in the order the placeholders appear.
//!
//! The dialect emitted here is DuckDB's. Function names are lowercase in the output so that
//! golden SQL files diff cleanly.

use crate::lexer::Span;
use crate::parser::{BinaryOp, Expr, ExprKind, UnaryOp};
use crate::{CalcError, FieldResolver, ParamValue, Params, Sql, ValueType};

/// Aggregates are the functions that collapse rows.
fn is_aggregate(name: &str) -> bool {
    matches!(
        name,
        "SUM" | "AVG" | "MIN" | "MAX" | "COUNT" | "COUNTD" | "MEDIAN" | "STDEV" | "VAR"
    )
}

/// Lowers an expression, then rejects an expression that mixes aggregates with bare fields.
pub(crate) fn lower(
    expr: &Expr,
    fields: &dyn FieldResolver,
    params: &mut Params,
) -> Result<Sql, CalcError> {
    let sql = lower_expression(expr, fields, params)?;

    if sql.aggregate
        && let Some(span) = row_field_outside_aggregate(expr, false)
    {
        return Err(CalcError::MixedAggregateContext { span });
    }

    Ok(sql)
}

fn lower_expression(
    expr: &Expr,
    fields: &dyn FieldResolver,
    params: &mut Params,
) -> Result<Sql, CalcError> {
    match &expr.kind {
        ExprKind::Integer(value) => {
            params.push(ParamValue::Integer(*value));
            Ok(scalar("?", ValueType::Integer, false))
        }
        ExprKind::Number(value) => {
            params.push(ParamValue::Number(*value));
            Ok(scalar("?", ValueType::Number, false))
        }
        ExprKind::Text(value) => {
            params.push(ParamValue::Text(value.clone()));
            Ok(scalar("?", ValueType::Text, false))
        }
        ExprKind::Boolean(value) => Ok(scalar(
            if *value { "TRUE" } else { "FALSE" },
            ValueType::Boolean,
            false,
        )),
        ExprKind::Null => Ok(scalar("NULL", ValueType::Null, false)),
        ExprKind::Field { name } => {
            let binding = fields
                .resolve_field(name)
                .ok_or_else(|| CalcError::UnknownField {
                    name: name.clone(),
                    span: expr.span,
                })?;
            Ok(scalar(&binding.sql, binding.value_type, false))
        }
        ExprKind::Unary { op, operand } => {
            let operand_sql = lower_expression(operand, fields, params)?;
            let span = operand.span;

            match op {
                UnaryOp::Negate => {
                    require_type(
                        "-",
                        1,
                        &operand_sql,
                        span,
                        ValueType::is_numeric,
                        "a number",
                    )?;
                    Ok(scalar(
                        &format!("(-{})", operand_sql.text),
                        operand_sql.value_type,
                        operand_sql.aggregate,
                    ))
                }
                UnaryOp::Not => {
                    require_type("NOT", 1, &operand_sql, span, is_boolean, "a boolean")?;
                    Ok(scalar(
                        &format!("(NOT {})", operand_sql.text),
                        ValueType::Boolean,
                        operand_sql.aggregate,
                    ))
                }
            }
        }
        ExprKind::Binary { op, left, right } => {
            lower_binary(*op, left, right, expr.span, fields, params)
        }
        ExprKind::Call { name, args } => lower_call(name, args, expr.span, fields, params),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let condition_sql = lower_expression(condition, fields, params)?;
            require_condition(&condition_sql, condition.span)?;

            let then_sql = lower_expression(then_branch, fields, params)?;
            let else_sql = lower_expression(else_branch, fields, params)?;

            let value_type = then_sql.value_type.unify(else_sql.value_type).ok_or(
                CalcError::IncompatibleBranches {
                    then_type: then_sql.value_type,
                    else_type: else_sql.value_type,
                    span: expr.span,
                },
            )?;

            Ok(scalar(
                &format!(
                    "CASE WHEN {} THEN {} ELSE {} END",
                    condition_sql.text, then_sql.text, else_sql.text
                ),
                value_type,
                condition_sql.aggregate || then_sql.aggregate || else_sql.aggregate,
            ))
        }
    }
}

fn lower_binary(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    span: Span,
    fields: &dyn FieldResolver,
    params: &mut Params,
) -> Result<Sql, CalcError> {
    let left_sql = lower_expression(left, fields, params)?;
    let right_sql = lower_expression(right, fields, params)?;
    let aggregate = left_sql.aggregate || right_sql.aggregate;

    match op {
        BinaryOp::And | BinaryOp::Or => {
            require_type(
                op.as_str(),
                1,
                &left_sql,
                left.span,
                is_boolean,
                "a boolean",
            )?;
            require_type(
                op.as_str(),
                2,
                &right_sql,
                right.span,
                is_boolean,
                "a boolean",
            )?;
            Ok(scalar(
                &format!("({} {} {})", left_sql.text, op.as_str(), right_sql.text),
                ValueType::Boolean,
                aggregate,
            ))
        }
        BinaryOp::Equal | BinaryOp::NotEqual => {
            if !comparable(left_sql.value_type, right_sql.value_type) {
                return Err(CalcError::InvalidOperand {
                    operator: op.as_str(),
                    left: left_sql.value_type,
                    right: right_sql.value_type,
                    span,
                });
            }
            Ok(scalar(
                &format!("({} {} {})", left_sql.text, op.as_str(), right_sql.text),
                ValueType::Boolean,
                aggregate,
            ))
        }
        BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
            if !orderable(left_sql.value_type, right_sql.value_type) {
                return Err(CalcError::InvalidOperand {
                    operator: op.as_str(),
                    left: left_sql.value_type,
                    right: right_sql.value_type,
                    span,
                });
            }
            Ok(scalar(
                &format!("({} {} {})", left_sql.text, op.as_str(), right_sql.text),
                ValueType::Boolean,
                aggregate,
            ))
        }
        BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply => {
            require_numeric_pair(op, &left_sql, &right_sql, span)?;
            Ok(scalar(
                &format!("({} {} {})", left_sql.text, op.as_str(), right_sql.text),
                arithmetic_type(left_sql.value_type, right_sql.value_type),
                aggregate,
            ))
        }
        BinaryOp::Divide => {
            require_numeric_pair(op, &left_sql, &right_sql, span)?;
            Ok(scalar(
                &format!("({} / {})", left_sql.text, right_sql.text),
                ValueType::Number,
                aggregate,
            ))
        }
        BinaryOp::Modulo => {
            require_numeric_pair(op, &left_sql, &right_sql, span)?;
            Ok(scalar(
                &format!("mod({}, {})", left_sql.text, right_sql.text),
                arithmetic_type(left_sql.value_type, right_sql.value_type),
                aggregate,
            ))
        }
    }
}

fn lower_call(
    name: &str,
    args: &[Expr],
    span: Span,
    fields: &dyn FieldResolver,
    params: &mut Params,
) -> Result<Sql, CalcError> {
    if matches!(name, "DATEDIFF" | "DATETRUNC") {
        return lower_date_unit_call(name, args, span, fields, params);
    }

    let lowered: Vec<Sql> = args
        .iter()
        .map(|arg| lower_expression(arg, fields, params))
        .collect::<Result<_, _>>()?;

    let aggregate = lowered.iter().any(|arg| arg.aggregate);

    let sql = match name {
        "SUM" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            let value_type = if lowered[0].value_type == ValueType::Null {
                ValueType::Number
            } else {
                lowered[0].value_type
            };
            aggregate_of(format!("sum({})", lowered[0].text), value_type)
        }
        "AVG" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            aggregate_of(format!("avg({})", lowered[0].text), ValueType::Number)
        }
        "MEDIAN" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            aggregate_of(format!("median({})", lowered[0].text), ValueType::Number)
        }
        "STDEV" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            aggregate_of(
                format!("stddev_samp({})", lowered[0].text),
                ValueType::Number,
            )
        }
        "VAR" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            aggregate_of(format!("var_samp({})", lowered[0].text), ValueType::Number)
        }
        "MIN" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            let value_type = lowered[0].value_type;
            aggregate_of(format!("min({})", lowered[0].text), value_type)
        }
        "MAX" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            let value_type = lowered[0].value_type;
            aggregate_of(format!("max({})", lowered[0].text), value_type)
        }
        "COUNT" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            aggregate_of(format!("count({})", lowered[0].text), ValueType::Integer)
        }
        "COUNTD" => {
            expect_args(name, args, 1, span)?;
            require_aggregate_argument(1, &lowered, &args[0])?;
            aggregate_of(
                format!("count(DISTINCT {})", lowered[0].text),
                ValueType::Integer,
            )
        }
        "ABS" => {
            expect_args(name, args, 1, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            scalar(
                &format!("abs({})", lowered[0].text),
                lowered[0].value_type,
                aggregate,
            )
        }
        "ROUND" => {
            expect_args_between(name, args, 1, 2, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            let text = if lowered.len() == 2 {
                require_type(
                    name,
                    2,
                    &lowered[1],
                    args[1].span,
                    is_integer,
                    "a whole number",
                )?;
                format!("round({}, {})", lowered[0].text, lowered[1].text)
            } else {
                format!("round({})", lowered[0].text)
            };
            scalar(&text, ValueType::Number, aggregate)
        }
        "CEILING" => {
            expect_args(name, args, 1, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            scalar(
                &format!("ceil({})", lowered[0].text),
                ValueType::Number,
                aggregate,
            )
        }
        "FLOOR" => {
            expect_args(name, args, 1, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            scalar(
                &format!("floor({})", lowered[0].text),
                ValueType::Number,
                aggregate,
            )
        }
        "SQRT" => {
            expect_args(name, args, 1, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            scalar(
                &format!("sqrt({})", lowered[0].text),
                ValueType::Number,
                aggregate,
            )
        }
        "POWER" => {
            expect_args(name, args, 2, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            require_type(
                name,
                2,
                &lowered[1],
                args[1].span,
                ValueType::is_numeric,
                "a number",
            )?;
            scalar(
                &format!("pow({}, {})", lowered[0].text, lowered[1].text),
                ValueType::Number,
                aggregate,
            )
        }
        "MOD" => {
            expect_args(name, args, 2, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_numeric,
                "a number",
            )?;
            require_type(
                name,
                2,
                &lowered[1],
                args[1].span,
                ValueType::is_numeric,
                "a number",
            )?;
            scalar(
                &format!("mod({}, {})", lowered[0].text, lowered[1].text),
                arithmetic_type(lowered[0].value_type, lowered[1].value_type),
                aggregate,
            )
        }
        "UPPER" | "LOWER" | "TRIM" => {
            expect_args(name, args, 1, span)?;
            require_type(name, 1, &lowered[0], args[0].span, is_text, "text")?;
            let function = match name {
                "UPPER" => "upper",
                "LOWER" => "lower",
                _ => "trim",
            };
            scalar(
                &format!("{function}({})", lowered[0].text),
                ValueType::Text,
                aggregate,
            )
        }
        "LENGTH" => {
            expect_args(name, args, 1, span)?;
            require_type(name, 1, &lowered[0], args[0].span, is_text, "text")?;
            scalar(
                &format!("length({})", lowered[0].text),
                ValueType::Integer,
                aggregate,
            )
        }
        "LEFT" | "RIGHT" => {
            expect_args(name, args, 2, span)?;
            require_type(name, 1, &lowered[0], args[0].span, is_text, "text")?;
            require_type(
                name,
                2,
                &lowered[1],
                args[1].span,
                is_integer,
                "a whole number",
            )?;
            let function = if name == "LEFT" { "left" } else { "right" };
            scalar(
                &format!("{function}({}, {})", lowered[0].text, lowered[1].text),
                ValueType::Text,
                aggregate,
            )
        }
        "MID" => {
            expect_args(name, args, 3, span)?;
            require_type(name, 1, &lowered[0], args[0].span, is_text, "text")?;
            require_type(
                name,
                2,
                &lowered[1],
                args[1].span,
                is_integer,
                "a whole number",
            )?;
            require_type(
                name,
                3,
                &lowered[2],
                args[2].span,
                is_integer,
                "a whole number",
            )?;
            scalar(
                &format!(
                    "substr({}, {}, {})",
                    lowered[0].text, lowered[1].text, lowered[2].text
                ),
                ValueType::Text,
                aggregate,
            )
        }
        "REPLACE" => {
            expect_args(name, args, 3, span)?;
            for (index, arg) in args.iter().enumerate() {
                require_type(name, index + 1, &lowered[index], arg.span, is_text, "text")?;
            }
            scalar(
                &format!(
                    "replace({}, {}, {})",
                    lowered[0].text, lowered[1].text, lowered[2].text
                ),
                ValueType::Text,
                aggregate,
            )
        }
        "CONTAINS" | "STARTSWITH" | "ENDSWITH" => {
            expect_args(name, args, 2, span)?;
            require_type(name, 1, &lowered[0], args[0].span, is_text, "text")?;
            require_type(name, 2, &lowered[1], args[1].span, is_text, "text")?;
            let function = match name {
                "CONTAINS" => "contains",
                "STARTSWITH" => "starts_with",
                _ => "ends_with",
            };
            scalar(
                &format!("{function}({}, {})", lowered[0].text, lowered[1].text),
                ValueType::Boolean,
                aggregate,
            )
        }
        "CONCAT" => {
            if args.is_empty() {
                return Err(CalcError::WrongArgumentCount {
                    function: name.to_owned(),
                    expected: "at least 1",
                    found: 0,
                    span,
                });
            }
            for (index, arg) in args.iter().enumerate() {
                require_type(name, index + 1, &lowered[index], arg.span, is_text, "text")?;
            }
            let parts: Vec<&str> = lowered.iter().map(|arg| arg.text.as_str()).collect();
            scalar(
                &format!("concat({})", parts.join(", ")),
                ValueType::Text,
                aggregate,
            )
        }
        "COALESCE" => {
            if args.is_empty() {
                return Err(CalcError::WrongArgumentCount {
                    function: name.to_owned(),
                    expected: "at least 1",
                    found: 0,
                    span,
                });
            }
            let value_type = unify_all(&lowered, span)?;
            let parts: Vec<&str> = lowered.iter().map(|arg| arg.text.as_str()).collect();
            scalar(
                &format!("coalesce({})", parts.join(", ")),
                value_type,
                aggregate,
            )
        }
        "IFNULL" => {
            expect_args(name, args, 2, span)?;
            let value_type = unify_all(&lowered, span)?;
            scalar(
                &format!("coalesce({}, {})", lowered[0].text, lowered[1].text),
                value_type,
                aggregate,
            )
        }
        "NULLIF" => {
            expect_args(name, args, 2, span)?;
            if !comparable(lowered[0].value_type, lowered[1].value_type) {
                return Err(CalcError::InvalidOperand {
                    operator: "NULLIF",
                    left: lowered[0].value_type,
                    right: lowered[1].value_type,
                    span,
                });
            }
            let value_type = if lowered[0].value_type == ValueType::Null {
                lowered[1].value_type
            } else {
                lowered[0].value_type
            };
            scalar(
                &format!("nullif({}, {})", lowered[0].text, lowered[1].text),
                value_type,
                aggregate,
            )
        }
        "ISNULL" => {
            expect_args(name, args, 1, span)?;
            scalar(
                &format!("({} IS NULL)", lowered[0].text),
                ValueType::Boolean,
                aggregate,
            )
        }
        "NOTNULL" => {
            expect_args(name, args, 1, span)?;
            scalar(
                &format!("({} IS NOT NULL)", lowered[0].text),
                ValueType::Boolean,
                aggregate,
            )
        }
        "DATE" => {
            expect_args(name, args, 1, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                is_date_input,
                "text or a date",
            )?;
            scalar(
                &format!("CAST({} AS DATE)", lowered[0].text),
                ValueType::Date,
                aggregate,
            )
        }
        "YEAR" | "MONTH" | "DAY" => {
            expect_args(name, args, 1, span)?;
            require_type(
                name,
                1,
                &lowered[0],
                args[0].span,
                ValueType::is_temporal,
                "a date or timestamp",
            )?;
            let function = match name {
                "YEAR" => "year",
                "MONTH" => "month",
                _ => "day",
            };
            scalar(
                &format!("{function}({})", lowered[0].text),
                ValueType::Integer,
                aggregate,
            )
        }
        "TODAY" => {
            expect_args(name, args, 0, span)?;
            scalar("current_date", ValueType::Date, false)
        }
        "NOW" => {
            expect_args(name, args, 0, span)?;
            scalar("now()", ValueType::Timestamp, false)
        }
        _ => {
            return Err(CalcError::UnknownFunction {
                name: name.to_owned(),
                span,
            });
        }
    };

    Ok(sql)
}

fn lower_date_unit_call(
    name: &str,
    args: &[Expr],
    span: Span,
    fields: &dyn FieldResolver,
    params: &mut Params,
) -> Result<Sql, CalcError> {
    let expected = if name == "DATEDIFF" { 3 } else { 2 };
    expect_args(name, args, expected, span)?;

    let unit = date_unit(name, &args[0])?;

    let mut lowered = Vec::with_capacity(expected - 1);
    for arg in &args[1..] {
        lowered.push(lower_expression(arg, fields, params)?);
    }
    let aggregate = lowered.iter().any(|arg| arg.aggregate);

    for (index, arg) in args[1..].iter().enumerate() {
        require_type(
            name,
            index + 2,
            &lowered[index],
            arg.span,
            ValueType::is_temporal,
            "a date or timestamp",
        )?;
    }

    if name == "DATEDIFF" {
        Ok(scalar(
            &format!(
                "date_diff('{unit}', {}, {})",
                lowered[0].text, lowered[1].text
            ),
            ValueType::Integer,
            aggregate,
        ))
    } else {
        let value_type = if matches!(unit, "hour" | "minute" | "second") {
            ValueType::Timestamp
        } else if lowered[0].value_type == ValueType::Null {
            ValueType::Date
        } else {
            lowered[0].value_type
        };
        Ok(scalar(
            &format!("date_trunc('{unit}', {})", lowered[0].text),
            value_type,
            aggregate,
        ))
    }
}

/// Validates the unit argument and returns its canonical spelling.
fn date_unit(function: &str, expr: &Expr) -> Result<&'static str, CalcError> {
    let ExprKind::Text(text) = &expr.kind else {
        return Err(CalcError::InvalidArgument {
            function: function.to_owned(),
            message: "the first argument must be a quoted unit, for example 'day'".to_owned(),
            span: expr.span,
        });
    };

    let unit = match text.to_ascii_lowercase().as_str() {
        "year" => "year",
        "quarter" => "quarter",
        "month" => "month",
        "week" => "week",
        "day" => "day",
        "hour" => "hour",
        "minute" => "minute",
        "second" => "second",
        other => {
            return Err(CalcError::InvalidArgument {
                function: function.to_owned(),
                message: format!(
                    "'{other}' is not a date unit; use year, quarter, month, week, day, hour, minute or second"
                ),
                span: expr.span,
            });
        }
    };

    Ok(unit)
}

fn scalar(text: &str, value_type: ValueType, aggregate: bool) -> Sql {
    Sql {
        text: text.to_owned(),
        value_type,
        aggregate,
    }
}

fn aggregate_of(text: String, value_type: ValueType) -> Sql {
    Sql {
        text,
        value_type,
        aggregate: true,
    }
}

fn expect_args(
    function: &str,
    args: &[Expr],
    expected: usize,
    span: Span,
) -> Result<(), CalcError> {
    if args.len() == expected {
        return Ok(());
    }
    Err(CalcError::WrongArgumentCount {
        function: function.to_owned(),
        expected: match expected {
            1 => "1",
            2 => "2",
            3 => "3",
            _ => "a different number of",
        },
        found: args.len(),
        span,
    })
}

fn expect_args_between(
    function: &str,
    args: &[Expr],
    minimum: usize,
    maximum: usize,
    span: Span,
) -> Result<(), CalcError> {
    if (minimum..=maximum).contains(&args.len()) {
        return Ok(());
    }
    Err(CalcError::WrongArgumentCount {
        function: function.to_owned(),
        expected: match maximum {
            2 => "1 or 2",
            _ => "a different number of",
        },
        found: args.len(),
        span,
    })
}

fn require_aggregate_argument(index: usize, lowered: &[Sql], arg: &Expr) -> Result<(), CalcError> {
    if lowered[index - 1].aggregate {
        return Err(CalcError::NestedAggregate { span: arg.span });
    }
    Ok(())
}

fn require_type(
    function: &str,
    index: usize,
    value: &Sql,
    span: Span,
    accepts: impl Fn(ValueType) -> bool,
    expected: &'static str,
) -> Result<(), CalcError> {
    if value.value_type == ValueType::Null || accepts(value.value_type) {
        return Ok(());
    }
    Err(CalcError::ArgumentType {
        function: function.to_owned(),
        argument: index,
        expected,
        found: value.value_type,
        span,
    })
}

fn require_condition(value: &Sql, span: Span) -> Result<(), CalcError> {
    if value.value_type == ValueType::Null || value.value_type == ValueType::Boolean {
        return Ok(());
    }
    Err(CalcError::ExpectedBoolean {
        found: value.value_type,
        span,
    })
}

fn require_numeric_pair(
    op: BinaryOp,
    left: &Sql,
    right: &Sql,
    span: Span,
) -> Result<(), CalcError> {
    if (left.value_type == ValueType::Null || left.value_type.is_numeric())
        && (right.value_type == ValueType::Null || right.value_type.is_numeric())
    {
        return Ok(());
    }
    Err(CalcError::InvalidOperand {
        operator: op.as_str(),
        left: left.value_type,
        right: right.value_type,
        span,
    })
}

fn arithmetic_type(left: ValueType, right: ValueType) -> ValueType {
    if left == ValueType::Integer && right == ValueType::Integer {
        ValueType::Integer
    } else {
        ValueType::Number
    }
}

fn unify_all(lowered: &[Sql], span: Span) -> Result<ValueType, CalcError> {
    let mut value_type = ValueType::Null;
    for arg in lowered {
        value_type = value_type
            .unify(arg.value_type)
            .ok_or(CalcError::IncompatibleBranches {
                then_type: value_type,
                else_type: arg.value_type,
                span,
            })?;
    }
    Ok(value_type)
}

fn comparable(left: ValueType, right: ValueType) -> bool {
    left == ValueType::Null
        || right == ValueType::Null
        || left == right
        || (left.is_numeric() && right.is_numeric())
        || (left.is_temporal() && right.is_temporal())
}

fn orderable(left: ValueType, right: ValueType) -> bool {
    comparable(left, right) && !(left == ValueType::Boolean && right == ValueType::Boolean)
}

fn is_boolean(value_type: ValueType) -> bool {
    value_type == ValueType::Boolean
}

fn is_text(value_type: ValueType) -> bool {
    value_type == ValueType::Text
}

fn is_integer(value_type: ValueType) -> bool {
    value_type == ValueType::Integer
}

fn is_date_input(value_type: ValueType) -> bool {
    value_type == ValueType::Text || value_type.is_temporal()
}

/// Returns the first field reference that is not inside an aggregate call.
fn row_field_outside_aggregate(expr: &Expr, inside: bool) -> Option<Span> {
    match &expr.kind {
        ExprKind::Field { .. } => (!inside).then_some(expr.span),
        ExprKind::Call { name, args } => {
            let inside = inside || is_aggregate(name);
            args.iter()
                .find_map(|arg| row_field_outside_aggregate(arg, inside))
        }
        ExprKind::Unary { operand, .. } => row_field_outside_aggregate(operand, inside),
        ExprKind::Binary { left, right, .. } => row_field_outside_aggregate(left, inside)
            .or_else(|| row_field_outside_aggregate(right, inside)),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => row_field_outside_aggregate(condition, inside)
            .or_else(|| row_field_outside_aggregate(then_branch, inside))
            .or_else(|| row_field_outside_aggregate(else_branch, inside)),
        ExprKind::Integer(_)
        | ExprKind::Number(_)
        | ExprKind::Text(_)
        | ExprKind::Boolean(_)
        | ExprKind::Null => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fields::fields;

    fn compiled(source: &str) -> crate::Compiled {
        crate::compile(source, &fields()).expect("compiles")
    }

    fn error(source: &str) -> CalcError {
        crate::compile(source, &fields()).expect_err("must fail")
    }

    #[test]
    fn aggregates_lower_to_their_duckdb_names() {
        assert_eq!(compiled("SUM([Revenue])").sql, "sum(\"revenue\")");
        assert_eq!(compiled("AVG([Revenue])").sql, "avg(\"revenue\")");
        assert_eq!(compiled("MEDIAN([Revenue])").sql, "median(\"revenue\")");
        assert_eq!(compiled("STDEV([Revenue])").sql, "stddev_samp(\"revenue\")");
        assert_eq!(compiled("VAR([Revenue])").sql, "var_samp(\"revenue\")");
        assert_eq!(compiled("MIN([Units])").sql, "min(\"units\")");
        assert_eq!(compiled("MAX([Region])").sql, "max(\"region\")");
        assert_eq!(compiled("COUNT([Units])").sql, "count(\"units\")");
        assert_eq!(
            compiled("COUNTD([Region])").sql,
            "count(DISTINCT \"region\")"
        );
    }

    #[test]
    fn aggregate_types_are_precise() {
        assert_eq!(compiled("SUM([Units])").value_type, ValueType::Integer);
        assert_eq!(compiled("SUM([Revenue])").value_type, ValueType::Number);
        assert_eq!(compiled("AVG([Units])").value_type, ValueType::Number);
        assert_eq!(compiled("COUNT([Units])").value_type, ValueType::Integer);
        assert_eq!(compiled("MAX([Region])").value_type, ValueType::Text);
    }

    #[test]
    fn arithmetic_preserves_integer_type() {
        let result = compiled("[Units] + 1");
        assert_eq!(result.sql, "(\"units\" + ?)");
        assert_eq!(result.value_type, ValueType::Integer);

        let result = compiled("[Units] / 2");
        assert_eq!(result.sql, "(\"units\" / ?)");
        assert_eq!(result.value_type, ValueType::Number);
    }

    #[test]
    fn modulo_lowers_to_a_function() {
        let result = compiled("[Units] % 3");
        assert_eq!(result.sql, "mod(\"units\", ?)");
        assert_eq!(result.params, vec![ParamValue::Integer(3)]);
    }

    #[test]
    fn comparisons_produce_booleans() {
        let result = compiled("[Revenue] > 100");
        assert_eq!(result.sql, "(\"revenue\" > ?)");
        assert_eq!(result.value_type, ValueType::Boolean);
        assert_eq!(result.params, vec![ParamValue::Integer(100)]);
    }

    #[test]
    fn logical_operators_require_booleans() {
        let result = compiled("[Active] AND [Revenue] > 0");
        assert_eq!(result.sql, "(\"active\" AND (\"revenue\" > ?))");

        let failure = error("[Revenue] AND [Active]");
        assert!(matches!(failure, CalcError::ArgumentType { .. }));
    }

    #[test]
    fn conditionals_lower_to_case() {
        let result = compiled("IF [Active] THEN [Units] ELSE 0 END");
        assert_eq!(result.sql, "CASE WHEN \"active\" THEN \"units\" ELSE ? END");
        assert_eq!(result.value_type, ValueType::Integer);
        assert_eq!(result.params, vec![ParamValue::Integer(0)]);
    }

    #[test]
    fn branches_must_unify() {
        let failure = error("IF [Active] THEN 1 ELSE 'x' END");
        assert!(matches!(failure, CalcError::IncompatibleBranches { .. }));
    }

    #[test]
    fn a_null_branch_takes_the_other_type() {
        let result = compiled("IF [Active] THEN [Region] END");
        assert_eq!(
            result.sql,
            "CASE WHEN \"active\" THEN \"region\" ELSE NULL END"
        );
        assert_eq!(result.value_type, ValueType::Text);
    }

    #[test]
    fn a_condition_must_be_boolean() {
        let failure = error("IF [Revenue] THEN 1 ELSE 0 END");
        match failure {
            CalcError::ExpectedBoolean { found, .. } => assert_eq!(found, ValueType::Number),
            other => panic!("expected a boolean error, got {other}"),
        }
    }

    #[test]
    fn text_functions_lower_to_duckdb() {
        assert_eq!(compiled("UPPER([Region])").sql, "upper(\"region\")");
        assert_eq!(compiled("LENGTH([Region])").sql, "length(\"region\")");
        assert_eq!(
            compiled("MID([Region], 2, 3)").sql,
            "substr(\"region\", ?, ?)"
        );
        assert_eq!(
            compiled("CONTAINS([Region], 'EU')").sql,
            "contains(\"region\", ?)"
        );
        assert_eq!(
            compiled("CONCAT([Region], ' / ', [Region])").sql,
            "concat(\"region\", ?, \"region\")"
        );
    }

    #[test]
    fn dates_lower_to_duckdb() {
        assert_eq!(
            compiled("DATEDIFF('day', [OrderDate], TODAY())").sql,
            "date_diff('day', \"order_date\", current_date)"
        );
        assert_eq!(
            compiled("DATETRUNC('month', [OrderDate])").sql,
            "date_trunc('month', \"order_date\")"
        );
        assert_eq!(compiled("DATE('2025-01-01')").sql, "CAST(? AS DATE)");
        assert_eq!(compiled("YEAR([OrderDate])").sql, "year(\"order_date\")");
        assert_eq!(compiled("NOW()").sql, "now()");
    }

    #[test]
    fn a_date_unit_must_be_one_of_the_known_units() {
        let failure = error("DATEDIFF('fortnight', [OrderDate], TODAY())");
        assert!(matches!(failure, CalcError::InvalidArgument { .. }));
    }

    #[test]
    fn a_date_unit_must_be_a_literal() {
        let failure = error("DATETRUNC([Region], [OrderDate])");
        match failure {
            CalcError::InvalidArgument { message, .. } => {
                assert!(message.contains("quoted unit"), "{message}");
            }
            other => panic!("expected an invalid argument, got {other}"),
        }
    }

    #[test]
    fn null_helpers_lower() {
        assert_eq!(compiled("ISNULL([Region])").sql, "(\"region\" IS NULL)");
        assert_eq!(
            compiled("COALESCE([Units], 0)").sql,
            "coalesce(\"units\", ?)"
        );
        assert_eq!(compiled("NULLIF([Units], 0)").sql, "nullif(\"units\", ?)");
    }

    #[test]
    fn an_unknown_function_names_itself() {
        let failure = error("TOTAL([Revenue])");
        match failure {
            CalcError::UnknownFunction { name, .. } => assert_eq!(name, "TOTAL"),
            other => panic!("expected an unknown function, got {other}"),
        }
    }

    #[test]
    fn wrong_argument_counts_are_reported_with_the_call_site() {
        let failure = error("ROUND([Revenue], 1, 2)");
        match failure {
            CalcError::WrongArgumentCount {
                function,
                expected,
                found,
                ..
            } => {
                assert_eq!(function, "ROUND");
                assert_eq!(expected, "1 or 2");
                assert_eq!(found, 3);
            }
            other => panic!("expected an argument count error, got {other}"),
        }
    }

    #[test]
    fn argument_types_are_checked() {
        let failure = error("UPPER([Revenue])");
        match failure {
            CalcError::ArgumentType {
                function,
                argument,
                found,
                ..
            } => {
                assert_eq!(function, "UPPER");
                assert_eq!(argument, 1);
                assert_eq!(found, ValueType::Number);
            }
            other => panic!("expected an argument type error, got {other}"),
        }
    }

    #[test]
    fn a_constant_may_accompany_an_aggregate() {
        let result = compiled("SUM([Revenue]) * 1.1");
        assert_eq!(result.sql, "(sum(\"revenue\") * ?)");
        assert!(result.aggregate);
    }

    #[test]
    fn a_bare_field_may_not_accompany_an_aggregate() {
        let failure = error("SUM([Revenue]) + [Units]");
        match failure {
            CalcError::MixedAggregateContext { span } => {
                assert_eq!(span.start, 17);
                assert_eq!(span.end, 24);
            }
            other => panic!("expected a mixed context error, got {other}"),
        }
    }

    #[test]
    fn a_field_inside_a_scalar_function_inside_an_aggregate_is_fine() {
        let result = compiled("SUM(ABS([Revenue]))");
        assert_eq!(result.sql, "sum(abs(\"revenue\"))");
        assert!(result.aggregate);
    }

    #[test]
    fn nested_aggregates_are_rejected_at_the_inner_call() {
        let failure = error("SUM(AVG([Revenue]))");
        assert!(matches!(failure, CalcError::NestedAggregate { .. }));
    }

    #[test]
    fn parameters_follow_the_order_of_the_placeholders() {
        let result = compiled("[Region] = 'EMEA' OR [Revenue] > 10");
        assert_eq!(result.sql, "((\"region\" = ?) OR (\"revenue\" > ?))");
        assert_eq!(
            result.params,
            vec![ParamValue::Text("EMEA".into()), ParamValue::Integer(10)]
        );
    }

    #[test]
    fn unary_minus_is_parenthesised() {
        let result = compiled("ABS(-[Revenue])");
        assert_eq!(result.sql, "abs((-\"revenue\"))");
    }

    #[test]
    fn not_is_parenthesised() {
        let result = compiled("NOT [Active]");
        assert_eq!(result.sql, "(NOT \"active\")");
    }

    #[test]
    fn check_reports_the_type_without_exposing_parameters() {
        let expr = crate::parse("SUM([Revenue])").expect("parses");
        let checked = crate::check(&expr, &fields()).expect("checks");
        assert_eq!(checked.value_type, ValueType::Number);
        assert!(checked.aggregate);
        assert_eq!(checked.text, "sum(\"revenue\")");
    }

    #[test]
    fn value_types_map_to_document_types() {
        assert_eq!(
            ValueType::Integer.data_type(),
            Some(slickbi_spec::DataType::Integer)
        );
        assert_eq!(
            ValueType::Timestamp.data_type(),
            Some(slickbi_spec::DataType::Timestamp)
        );
        assert_eq!(ValueType::Null.data_type(), None);
    }
}
