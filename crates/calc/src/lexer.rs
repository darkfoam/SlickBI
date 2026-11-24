//! Tokeniser for the calculation language.
//!
//! Every token carries a byte span, so a parse or type error can be underlined in the editor
//! without re-scanning the source.

use std::fmt;

use thiserror::Error;

/// A half-open byte range in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// The first byte of the range.
    pub start: usize,
    /// One past the last byte of the range.
    pub end: usize,
}

impl Span {
    /// Creates a span from byte offsets.
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The number of bytes covered.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Reports whether the span covers no bytes.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The smallest span covering both inputs.
    pub fn merge(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A reserved word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    /// `AND`
    And,
    /// `OR`
    Or,
    /// `NOT`
    Not,
    /// `TRUE`
    True,
    /// `FALSE`
    False,
    /// `NULL`
    Null,
    /// `IF`
    If,
    /// `THEN`
    Then,
    /// `ELSE`
    Else,
    /// `END`
    End,
    /// `CASE`
    Case,
    /// `WHEN`
    When,
}

impl Keyword {
    /// The keyword's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::And => "AND",
            Keyword::Or => "OR",
            Keyword::Not => "NOT",
            Keyword::True => "TRUE",
            Keyword::False => "FALSE",
            Keyword::Null => "NULL",
            Keyword::If => "IF",
            Keyword::Then => "THEN",
            Keyword::Else => "ELSE",
            Keyword::End => "END",
            Keyword::Case => "CASE",
            Keyword::When => "WHEN",
        }
    }

    fn from_ident(text: &str) -> Option<Keyword> {
        let keyword = match text {
            "AND" => Keyword::And,
            "OR" => Keyword::Or,
            "NOT" => Keyword::Not,
            "TRUE" => Keyword::True,
            "FALSE" => Keyword::False,
            "NULL" => Keyword::Null,
            "IF" => Keyword::If,
            "THEN" => Keyword::Then,
            "ELSE" => Keyword::Else,
            "END" => Keyword::End,
            "CASE" => Keyword::Case,
            "WHEN" => Keyword::When,
            _ => return None,
        };
        Some(keyword)
    }
}

/// The kind of a token.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// A whole-number literal.
    Integer(i64),
    /// A decimal literal.
    Number(f64),
    /// A single-quoted text literal, unescaped.
    Text(String),
    /// A bracketed field reference, without the brackets.
    Field(String),
    /// A function name.
    Ident(String),
    /// A reserved word.
    Keyword(Keyword),
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `,`
    Comma,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `=` or `==`
    Equal,
    /// `!=` or `<>`
    NotEqual,
    /// `<`
    Less,
    /// `<=`
    LessEqual,
    /// `>`
    Greater,
    /// `>=`
    GreaterEqual,
    /// The end of the input.
    Eof,
}

impl TokenKind {
    /// A human-readable description for error messages.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Integer(value) => format!("{value}"),
            TokenKind::Number(value) => format!("{value}"),
            TokenKind::Text(value) => format!("'{value}'"),
            TokenKind::Field(name) => format!("[{name}]"),
            TokenKind::Ident(name) => format!("'{name}'"),
            TokenKind::Keyword(keyword) => format!("'{}'", keyword.as_str()),
            TokenKind::LParen => "'('".to_owned(),
            TokenKind::RParen => "')'".to_owned(),
            TokenKind::Comma => "','".to_owned(),
            TokenKind::Plus => "'+'".to_owned(),
            TokenKind::Minus => "'-'".to_owned(),
            TokenKind::Star => "'*'".to_owned(),
            TokenKind::Slash => "'/'".to_owned(),
            TokenKind::Percent => "'%'".to_owned(),
            TokenKind::Equal => "'='".to_owned(),
            TokenKind::NotEqual => "'<>'".to_owned(),
            TokenKind::Less => "'<'".to_owned(),
            TokenKind::LessEqual => "'<='".to_owned(),
            TokenKind::Greater => "'>'".to_owned(),
            TokenKind::GreaterEqual => "'>='".to_owned(),
            TokenKind::Eof => "the end of the expression".to_owned(),
        }
    }
}

/// A token and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// What the token is.
    pub kind: TokenKind,
    /// Where it appears in the source.
    pub span: Span,
}

/// A failure while tokenising.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LexError {
    /// A character cannot start any token.
    #[error("unexpected character '{ch}'")]
    UnexpectedCharacter {
        /// The character.
        ch: char,
        /// Where it appears.
        span: Span,
    },
    /// A text literal is never closed.
    #[error("this text literal is missing its closing quote")]
    UnterminatedText {
        /// Where the literal starts.
        span: Span,
    },
    /// A field reference is never closed.
    #[error("this field reference is missing its closing bracket")]
    UnterminatedField {
        /// Where the reference starts.
        span: Span,
    },
    /// A field reference is empty.
    #[error("a field reference cannot be empty")]
    EmptyField {
        /// Where the reference appears.
        span: Span,
    },
    /// A number could not be read.
    #[error("this number is malformed")]
    MalformedNumber {
        /// Where the number appears.
        span: Span,
    },
}

impl LexError {
    /// The source span this error refers to.
    pub fn span(&self) -> Span {
        match self {
            LexError::UnexpectedCharacter { span, .. }
            | LexError::UnterminatedText { span }
            | LexError::UnterminatedField { span }
            | LexError::EmptyField { span }
            | LexError::MalformedNumber { span } => *span,
        }
    }
}

/// Tokenises a calculation.
pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    Lexer {
        source,
        position: 0,
    }
    .run()
}

struct Lexer<'a> {
    source: &'a str,
    position: usize,
}

impl<'a> Lexer<'a> {
    fn run(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let finished = token.kind == TokenKind::Eof;
            tokens.push(token);
            if finished {
                return Ok(tokens);
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_whitespace();
        let start = self.position;

        let Some(ch) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                span: Span::new(start, start),
            });
        };

        let kind = match ch {
            '(' => {
                self.bump();
                TokenKind::LParen
            }
            ')' => {
                self.bump();
                TokenKind::RParen
            }
            ',' => {
                self.bump();
                TokenKind::Comma
            }
            '+' => {
                self.bump();
                TokenKind::Plus
            }
            '-' => {
                self.bump();
                TokenKind::Minus
            }
            '*' => {
                self.bump();
                TokenKind::Star
            }
            '/' => {
                self.bump();
                TokenKind::Slash
            }
            '%' => {
                self.bump();
                TokenKind::Percent
            }
            '=' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                }
                TokenKind::Equal
            }
            '!' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    TokenKind::NotEqual
                } else {
                    return Err(LexError::UnexpectedCharacter {
                        ch: '!',
                        span: Span::new(start, self.position),
                    });
                }
            }
            '<' => {
                self.bump();
                match self.peek() {
                    Some('=') => {
                        self.bump();
                        TokenKind::LessEqual
                    }
                    Some('>') => {
                        self.bump();
                        TokenKind::NotEqual
                    }
                    _ => TokenKind::Less,
                }
            }
            '>' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    TokenKind::GreaterEqual
                } else {
                    TokenKind::Greater
                }
            }
            '[' => {
                self.bump();
                return self.field(start);
            }
            '\'' => {
                self.bump();
                return self.text(start);
            }
            ch if ch.is_ascii_digit() => return self.number(start),
            ch if ch.is_alphabetic() || ch == '_' => self.ident(),
            other => {
                return Err(LexError::UnexpectedCharacter {
                    ch: other,
                    span: Span::new(start, self.position + other.len_utf8()),
                });
            }
        };

        Ok(Token {
            kind,
            span: Span::new(start, self.position),
        })
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn peek_second(&self) -> Option<char> {
        let mut chars = self.source[self.position..].chars();
        chars.next();
        chars.next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.position += ch.len_utf8();
        Some(ch)
    }

    fn field(&mut self, start: usize) -> Result<Token, LexError> {
        let mut name = String::new();
        loop {
            match self.bump() {
                Some(']') => {
                    if self.peek() == Some(']') {
                        self.bump();
                        name.push(']');
                    } else {
                        break;
                    }
                }
                Some(ch) => name.push(ch),
                None => {
                    return Err(LexError::UnterminatedField {
                        span: Span::new(start, self.position),
                    });
                }
            }
        }

        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(LexError::EmptyField {
                span: Span::new(start, self.position),
            });
        }

        Ok(Token {
            kind: TokenKind::Field(name),
            span: Span::new(start, self.position),
        })
    }

    fn text(&mut self, start: usize) -> Result<Token, LexError> {
        let mut value = String::new();
        loop {
            match self.bump() {
                Some('\'') => {
                    if self.peek() == Some('\'') {
                        self.bump();
                        value.push('\'');
                    } else {
                        break;
                    }
                }
                Some(ch) => value.push(ch),
                None => {
                    return Err(LexError::UnterminatedText {
                        span: Span::new(start, self.position),
                    });
                }
            }
        }

        Ok(Token {
            kind: TokenKind::Text(value),
            span: Span::new(start, self.position),
        })
    }

    fn number(&mut self, start: usize) -> Result<Token, LexError> {
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.bump();
        }

        let mut fractional = false;
        if self.peek() == Some('.') {
            if self.peek_second().is_some_and(|ch| ch.is_ascii_digit()) {
                fractional = true;
                self.bump();
                while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                    self.bump();
                }
            } else {
                return Err(LexError::MalformedNumber {
                    span: Span::new(start, self.position + 1),
                });
            }
        }

        let span = Span::new(start, self.position);
        let text = &self.source[start..self.position];

        if fractional {
            text.parse::<f64>()
                .map(|value| Token {
                    kind: TokenKind::Number(value),
                    span,
                })
                .map_err(|_| LexError::MalformedNumber { span })
        } else {
            text.parse::<i64>()
                .map(|value| Token {
                    kind: TokenKind::Integer(value),
                    span,
                })
                .map_err(|_| LexError::MalformedNumber { span })
        }
    }

    fn ident(&mut self) -> TokenKind {
        let start = self.position;
        while self
            .peek()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            self.bump();
        }
        let text = &self.source[start..self.position];
        let upper = text.to_ascii_uppercase();

        match Keyword::from_ident(&upper) {
            Some(keyword) => TokenKind::Keyword(keyword),
            None => TokenKind::Ident(upper),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source)
            .expect("tokenises")
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn tokenises_arithmetic() {
        assert_eq!(
            kinds("1 + 2.5 * [Revenue]"),
            vec![
                TokenKind::Integer(1),
                TokenKind::Plus,
                TokenKind::Number(2.5),
                TokenKind::Star,
                TokenKind::Field("Revenue".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn keywords_are_case_insensitive() {
        assert_eq!(
            kinds("if true then 1 else 0 end"),
            vec![
                TokenKind::Keyword(Keyword::If),
                TokenKind::Keyword(Keyword::True),
                TokenKind::Keyword(Keyword::Then),
                TokenKind::Integer(1),
                TokenKind::Keyword(Keyword::Else),
                TokenKind::Integer(0),
                TokenKind::Keyword(Keyword::End),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn function_names_are_uppercased() {
        assert_eq!(
            kinds("sum(x)"),
            vec![
                TokenKind::Ident("SUM".into()),
                TokenKind::LParen,
                TokenKind::Ident("X".into()),
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn field_names_keep_their_spacing_and_escape_brackets() {
        assert_eq!(
            kinds("[Net Revenue (USD)]"),
            vec![TokenKind::Field("Net Revenue (USD)".into()), TokenKind::Eof]
        );
        assert_eq!(
            kinds("[a]]b]"),
            vec![TokenKind::Field("a]b".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn text_literals_escape_quotes_by_doubling() {
        assert_eq!(
            kinds("'it''s'"),
            vec![TokenKind::Text("it's".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn comparison_operators_have_aliases() {
        assert_eq!(
            kinds("== <> != <= >="),
            vec![
                TokenKind::Equal,
                TokenKind::NotEqual,
                TokenKind::NotEqual,
                TokenKind::LessEqual,
                TokenKind::GreaterEqual,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn spans_cover_the_token_text() {
        let source = "1 + [Revenue]";
        let tokens = tokenize(source).expect("tokenises");
        let field = tokens
            .iter()
            .find(|token| matches!(token.kind, TokenKind::Field(_)))
            .expect("a field token");
        assert_eq!(&source[field.span.start..field.span.end], "[Revenue]");
    }

    #[test]
    fn unterminated_text_is_reported_at_its_start() {
        let error = tokenize("'open").expect_err("must fail");
        assert!(matches!(error, LexError::UnterminatedText { .. }));
        assert_eq!(error.span().start, 0);
    }

    #[test]
    fn unterminated_field_is_reported() {
        let error = tokenize("[Revenue").expect_err("must fail");
        assert!(matches!(error, LexError::UnterminatedField { .. }));
    }

    #[test]
    fn empty_field_is_rejected() {
        let error = tokenize("[   ]").expect_err("must fail");
        assert!(matches!(error, LexError::EmptyField { .. }));
    }

    #[test]
    fn a_bare_bang_is_not_an_operator() {
        let error = tokenize("1 ! 2").expect_err("must fail");
        assert!(matches!(
            error,
            LexError::UnexpectedCharacter { ch: '!', .. }
        ));
    }

    #[test]
    fn a_trailing_dot_is_a_malformed_number() {
        let error = tokenize("1.").expect_err("must fail");
        assert!(matches!(error, LexError::MalformedNumber { .. }));
        assert_eq!(error.span(), Span::new(0, 2));
    }
}
