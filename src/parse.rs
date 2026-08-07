//! The strict JSON5 parser: token stream in, CST out.
//!
//! [`Json5Parser::new`] is the strict path — and, unlike JSON, strict JSON5 needs no
//! tolerance switches: every JSON5 extension (identifier names, single quotes, comments, trailing
//! commas, hex numbers, ...) is a mandatory language feature, and a leading byte-order mark is
//! ordinary whitespace, so there is no `allow_bom`. The [spec]'s only parser latitude is the
//! implementation limit on nesting depth ([`Json5ParserBuilder::max_depth`], mirroring
//! `strata-json`'s option and default) and the *should*-level warning for unescaped
//! U+2028/U+2029 in strings, which is always recorded on the parsed [`Document`] rather than
//! optioned. The spec's permission to "accept non-JSON5 forms or extensions" is deliberately not
//! exercised: per the ecosystem's strict-by-default policy, this parser accepts exactly JSON5.
//!
//! [spec]: https://spec.json5.org/

use crate::cst::{
    ArrayNode, Document, Element, Member, ObjectNode, SyntaxToken, Trivia, TriviaKind, Value,
    Warning, WarningKind,
};
use crate::rules::{Json5Token, JsonToken, StrictRuleSet, advance_position, strict_rule_set};
use std::error::Error;
use std::fmt;
use std::io::{Cursor, Read};
use strata_lexer::{
    LexError, Lexer, LexerConfig, MatcherSet, Position, SourceStore, SpecScanner, Token,
};

/// The default nesting-depth limit, shared with `strata-json`.
///
/// The [spec] permits implementations to set limits on the maximum depth of nesting; this crate
/// uses the same default as `strata-json` (128). Override it with
/// [`Json5ParserBuilder::max_depth`]. The limit also bounds the parser's recursion, so raising
/// it substantially trades stack headroom for depth.
///
/// [spec]: https://spec.json5.org/
pub const DEFAULT_MAX_DEPTH: usize = strata_json::DEFAULT_MAX_DEPTH;

/// A parse failure, carrying the position at which it was observed.
#[derive(Debug)]
pub struct Json5Error {
    /// Position of the offending byte or token start.
    pub position: Position,
    /// What went wrong.
    pub kind: Json5ErrorKind,
}

/// Categories of parse failures.
#[derive(Debug)]
pub enum Json5ErrorKind {
    /// The lexer failed: I/O, an unrecognized byte, or a token-shape violation (invalid string
    /// escapes, unescaped line terminators in strings, number-grammar violations, unterminated
    /// strings or block comments, invalid UTF-8) reported at the offending byte.
    Lex(LexError),
    /// A token of an unexpected kind where the grammar required something else.
    Expected {
        /// What the grammar required here.
        expected: &'static str,
        /// The token kind actually found.
        found: Json5Token,
    },
    /// The input ended where the grammar required more.
    UnexpectedEof {
        /// What the grammar required here.
        expected: &'static str,
    },
    /// A significant token followed the complete top-level value.
    TrailingContent {
        /// The token kind found after the value.
        found: Json5Token,
    },
    /// Nesting exceeded the configured depth limit (a spec-permitted implementation limit).
    DepthLimitExceeded {
        /// The configured limit.
        limit: usize,
    },
}

impl fmt::Display for Json5ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(error) => write!(formatter, "{error}"),
            Self::Expected { expected, found } => {
                write!(formatter, "expected {expected}, found {}", found.describe())
            }
            Self::UnexpectedEof { expected } => {
                write!(formatter, "unexpected end of input, expected {expected}")
            }
            Self::TrailingContent { found } => write!(
                formatter,
                "unexpected {} after the top-level value",
                found.describe()
            ),
            Self::DepthLimitExceeded { limit } => {
                write!(formatter, "nesting depth limit of {limit} exceeded")
            }
        }
    }
}

impl fmt::Display for Json5Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            // The lexer's message already carries its byte position.
            Json5ErrorKind::Lex(error) => write!(formatter, "{error}"),
            kind => write!(
                formatter,
                "{kind} at line {}, column {} (byte {})",
                self.position.line, self.position.column, self.position.byte
            ),
        }
    }
}

impl Error for Json5Error {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            Json5ErrorKind::Lex(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LexError> for Json5Error {
    fn from(error: LexError) -> Self {
        Self {
            position: error.position,
            kind: Json5ErrorKind::Lex(error),
        }
    }
}

/// A reusable, strict JSON5 parser.
///
/// `M` is the token rule set handed to the lexer for every parse; it defaults to the strict
/// JSON5 rules. The parser clones the rules into a fresh scanner per [`Json5Parser::parse`]
/// call, so one configured parser can serve many inputs.
#[derive(Debug, Clone)]
pub struct Json5Parser<M = StrictRuleSet> {
    rules: M,
    max_depth: usize,
    lexer_config: LexerConfig,
}

impl Json5Parser<StrictRuleSet> {
    /// Creates a strict JSON5 parser with [`DEFAULT_MAX_DEPTH`] nesting.
    pub fn new() -> Self {
        Self {
            rules: strict_rule_set(),
            max_depth: DEFAULT_MAX_DEPTH,
            lexer_config: LexerConfig::new(),
        }
    }

    /// Starts a builder for the spec-permitted depth limit or for replacing lower layers.
    pub fn builder() -> Json5ParserBuilder<StrictRuleSet> {
        Json5ParserBuilder::new()
    }
}

impl Default for Json5Parser<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Json5Parser<M>
where
    M: MatcherSet<Kind = Json5Token> + Clone,
{
    /// Parses a complete JSON5 text from `reader` into a [`Document`].
    ///
    /// Any JSON5 value is a valid top-level document. The entire input must be consumed:
    /// trailing content other than whitespace and comments is an error. On success the consumed
    /// source bytes are retained in the document's [`SourceStore`], keeping the CST zero-copy,
    /// and the spec-mandated warnings are recorded on the document
    /// ([`Document::warnings`]).
    pub fn parse<R: Read>(&self, reader: R) -> Result<Document, Json5Error> {
        // Compaction must stay off so the consumed bytes survive until they are parked into the
        // document's source store.
        let config = self.lexer_config.with_compact_after(None);
        let scanner = SpecScanner::new(self.rules.clone(), Json5Token::Eof);
        let mut driver = Driver {
            lexer: Lexer::with_config(reader, scanner, config),
            pending: Vec::new(),
            peeked: None,
            max_depth: self.max_depth,
        };

        let root = driver.parse_value(0)?;
        let eof = driver.next_token()?;

        if eof.kind != Json5Token::Eof {
            return Err(Json5Error {
                position: eof.start,
                kind: Json5ErrorKind::TrailingContent { found: eof.kind },
            });
        }

        let (mut input, _scanner) = driver.lexer.into_parts();
        let mut source = SourceStore::new();
        input.park_consumed_buffer(&mut source)?;

        let warnings = collect_warnings(&root, &source);

        Ok(Document::new(root, eof.leading, warnings, source))
    }

    /// Parses a complete JSON5 text from a string slice.
    pub fn parse_str(&self, input: &str) -> Result<Document, Json5Error> {
        self.parse(Cursor::new(input.as_bytes()))
    }

    /// Parses a complete JSON5 text from a byte slice. The bytes must be UTF-8; violations
    /// inside strings, comments, and identifiers are lexical errors, and any other non-ASCII
    /// byte outside JSON5 whitespace matches no rule.
    pub fn parse_bytes(&self, input: &[u8]) -> Result<Document, Json5Error> {
        self.parse(Cursor::new(input))
    }

    /// The token rule set handed to the lexer at each parse.
    pub const fn rules(&self) -> &M {
        &self.rules
    }

    /// The configured nesting-depth limit.
    pub const fn max_depth(&self) -> usize {
        self.max_depth
    }
}

/// Builder for [`Json5Parser`], strict by default (there is no other mode: the only knobs are
/// the spec-permitted depth limit and the lower-layer replacement hooks).
#[derive(Debug, Clone)]
pub struct Json5ParserBuilder<M = StrictRuleSet> {
    rules: M,
    max_depth: usize,
    lexer_config: LexerConfig,
}

impl Json5ParserBuilder<StrictRuleSet> {
    /// Starts from the strict defaults.
    pub fn new() -> Self {
        Self {
            rules: strict_rule_set(),
            max_depth: DEFAULT_MAX_DEPTH,
            lexer_config: LexerConfig::new(),
        }
    }
}

impl Default for Json5ParserBuilder<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Json5ParserBuilder<M> {
    /// Replaces the token rule set handed to the lexer.
    ///
    /// This is the rules-layer composition point: tests can mock the rules, and adjusted rule
    /// sets can be swapped in without touching the grammar driver. The set must still produce
    /// [`Json5Token`] kinds; a superset *language* instead defines its own kind enum and parser,
    /// reusing this crate's rule structs via
    /// [`MatcherExt::map_kind`](strata_lexer::MatcherExt::map_kind) — exactly as this crate
    /// reuses `strata-json`'s.
    pub fn with_rules<N>(self, rules: N) -> Json5ParserBuilder<N>
    where
        N: MatcherSet<Kind = Json5Token> + Clone,
    {
        Json5ParserBuilder {
            rules,
            max_depth: self.max_depth,
            lexer_config: self.lexer_config,
        }
    }

    /// Replaces the lexer configuration (reader capacities, position tracking, ...).
    ///
    /// The parser always disables buffer compaction (`compact_after = None`) at parse time: the
    /// consumed bytes must survive until they are parked into the document's source store.
    #[must_use]
    pub const fn with_lexer_config(mut self, config: LexerConfig) -> Self {
        self.lexer_config = config;
        self
    }

    /// Sets the nesting-depth limit.
    ///
    /// The [spec] permits implementations to limit the maximum depth of nesting. The limit
    /// counts nested arrays/objects — with a limit of 1, `[1]` parses and `[[1]]` does not. It
    /// also bounds the parser's recursion depth; see [`DEFAULT_MAX_DEPTH`].
    ///
    /// [spec]: https://spec.json5.org/
    #[must_use]
    pub const fn max_depth(mut self, limit: usize) -> Self {
        self.max_depth = limit;
        self
    }

    /// Finishes the builder.
    pub fn build(self) -> Json5Parser<M>
    where
        M: MatcherSet<Kind = Json5Token> + Clone,
    {
        Json5Parser {
            rules: self.rules,
            max_depth: self.max_depth,
            lexer_config: self.lexer_config,
        }
    }
}

/// Returns whether `kind` can serve as an object member name.
///
/// The [spec] allows a string or an ECMAScript 5.1 `IdentifierName`. `IdentifierName` (unlike
/// `Identifier`) includes reserved words, so the classified word tokens — `true`, `false`,
/// `null`, `Infinity`, `NaN` — are valid names too.
///
/// [spec]: https://spec.json5.org/
const fn is_member_name(kind: Json5Token) -> bool {
    matches!(
        kind,
        Json5Token::Identifier
            | Json5Token::String
            | Json5Token::Infinity
            | Json5Token::NaN
            | Json5Token::Json(JsonToken::True | JsonToken::False | JsonToken::Null)
    )
}

/// Walks every string token of the finished tree and records the spec-mandated warnings for
/// unescaped U+2028/U+2029 (see [`WarningKind::UnescapedLineSeparator`]).
fn collect_warnings(root: &Value, source: &SourceStore) -> Vec<Warning> {
    let mut warnings = Vec::new();

    root.for_each_token(&mut |token| {
        if token.kind == Json5Token::String
            && let Some(bytes) = source.bytes(token.span)
        {
            scan_string_warnings(bytes, token.start, &mut warnings);
        }
    });

    warnings
}

/// Scans one string lexeme (quotes included, already validated by the string rule) for raw
/// U+2028/U+2029 characters, escape-aware: a separator directly preceded by a backslash is a
/// line continuation, not an unescaped occurrence.
fn scan_string_warnings(bytes: &[u8], start: Position, warnings: &mut Vec<Warning>) {
    let Ok(text) = core::str::from_utf8(bytes) else {
        // The string rule already validated the lexeme; nothing to scan otherwise.
        return;
    };

    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if escaped {
            // The character after a backslash is escape content — including a second backslash
            // (`\\`), which does *not* escape what follows it.
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if matches!(ch, '\u{2028}' | '\u{2029}') {
            warnings.push(Warning {
                position: advance_position(start, bytes.get(..index).unwrap_or_default()),
                kind: WarningKind::UnescapedLineSeparator { separator: ch },
            });
        }
    }
}

/// The per-parse grammar driver: pulls tokens, folds trivia into the next significant token, and
/// recursively builds the CST.
struct Driver<R: Read, M: MatcherSet<Kind = Json5Token>> {
    lexer: Lexer<R, SpecScanner<M, Json5Token>>,
    pending: Vec<Trivia>,
    peeked: Option<SyntaxToken>,
    max_depth: usize,
}

impl<R: Read, M: MatcherSet<Kind = Json5Token>> Driver<R, M> {
    /// Returns the next significant token with its leading trivia attached.
    fn next_token(&mut self) -> Result<SyntaxToken, Json5Error> {
        if let Some(token) = self.peeked.take() {
            return Ok(token);
        }

        loop {
            let token = self.lexer.next_token()?;

            match token.kind {
                Json5Token::Whitespace => self.push_trivia(TriviaKind::Whitespace, &token),
                Json5Token::LineComment => self.push_trivia(TriviaKind::LineComment, &token),
                Json5Token::BlockComment => self.push_trivia(TriviaKind::BlockComment, &token),
                _ => {
                    return Ok(SyntaxToken {
                        kind: token.kind,
                        span: token.span,
                        start: token.start,
                        end: token.end,
                        leading: std::mem::take(&mut self.pending),
                    });
                }
            }
        }
    }

    /// Peeks the kind of the next significant token without consuming it.
    fn peek_kind(&mut self) -> Result<Json5Token, Json5Error> {
        if self.peeked.is_none() {
            self.peeked = Some(self.next_token()?);
        }

        Ok(self
            .peeked
            .as_ref()
            .map_or(Json5Token::Eof, |token| token.kind))
    }

    fn push_trivia(&mut self, kind: TriviaKind, token: &Token<Json5Token>) {
        self.pending.push(Trivia {
            kind,
            span: token.span,
            start: token.start,
            end: token.end,
        });
    }

    /// Parses one value. `depth` is the number of containers already entered.
    fn parse_value(&mut self, depth: usize) -> Result<Value, Json5Error> {
        let token = self.next_token()?;

        match token.kind {
            Json5Token::Json(JsonToken::LeftBrace) => self.parse_object(token, depth),
            Json5Token::Json(JsonToken::LeftBracket) => self.parse_array(token, depth),
            Json5Token::String => Ok(Value::String(token)),
            Json5Token::Number | Json5Token::Infinity | Json5Token::NaN => Ok(Value::Number(token)),
            Json5Token::Json(JsonToken::True) => Ok(Value::Bool { value: true, token }),
            Json5Token::Json(JsonToken::False) => Ok(Value::Bool {
                value: false,
                token,
            }),
            Json5Token::Json(JsonToken::Null) => Ok(Value::Null(token)),
            Json5Token::Eof => Err(Json5Error {
                position: token.start,
                kind: Json5ErrorKind::UnexpectedEof {
                    expected: "a JSON5 value",
                },
            }),
            found => Err(Json5Error {
                position: token.start,
                kind: Json5ErrorKind::Expected {
                    expected: "a JSON5 value",
                    found,
                },
            }),
        }
    }

    /// Checks the depth limit on entering a container, returning the child depth.
    fn enter_container(&self, depth: usize, open: &SyntaxToken) -> Result<usize, Json5Error> {
        let entered = depth.saturating_add(1);

        if entered > self.max_depth {
            Err(Json5Error {
                position: open.start,
                kind: Json5ErrorKind::DepthLimitExceeded {
                    limit: self.max_depth,
                },
            })
        } else {
            Ok(entered)
        }
    }

    fn parse_object(&mut self, open: SyntaxToken, depth: usize) -> Result<Value, Json5Error> {
        let depth = self.enter_container(depth, &open)?;
        let mut members = Vec::new();

        if self.peek_kind()? == Json5Token::Json(JsonToken::RightBrace) {
            let close = self.next_token()?;
            return Ok(Value::Object(ObjectNode {
                open,
                members,
                close,
            }));
        }

        loop {
            let name = self.next_token()?;

            if !is_member_name(name.kind) {
                return Err(match name.kind {
                    Json5Token::Eof => Json5Error {
                        position: name.start,
                        kind: Json5ErrorKind::UnexpectedEof {
                            expected: "an object member name (an identifier or a string)",
                        },
                    },
                    found => Json5Error {
                        position: name.start,
                        kind: Json5ErrorKind::Expected {
                            expected: "an object member name (an identifier or a string)",
                            found,
                        },
                    },
                });
            }

            let colon = self.expect(Json5Token::Json(JsonToken::Colon), "':'")?;
            let value = self.parse_value(depth)?;
            let separator = self.next_token()?;

            match separator.kind {
                Json5Token::Json(JsonToken::Comma) => {
                    members.push(Member {
                        name,
                        colon,
                        value,
                        comma: Some(separator),
                    });

                    // JSON5: one trailing comma is allowed in a non-empty object.
                    if self.peek_kind()? == Json5Token::Json(JsonToken::RightBrace) {
                        let close = self.next_token()?;
                        return Ok(Value::Object(ObjectNode {
                            open,
                            members,
                            close,
                        }));
                    }
                }
                Json5Token::Json(JsonToken::RightBrace) => {
                    members.push(Member {
                        name,
                        colon,
                        value,
                        comma: None,
                    });

                    return Ok(Value::Object(ObjectNode {
                        open,
                        members,
                        close: separator,
                    }));
                }
                Json5Token::Eof => {
                    return Err(Json5Error {
                        position: separator.start,
                        kind: Json5ErrorKind::UnexpectedEof {
                            expected: "',' or '}'",
                        },
                    });
                }
                found => {
                    return Err(Json5Error {
                        position: separator.start,
                        kind: Json5ErrorKind::Expected {
                            expected: "',' or '}'",
                            found,
                        },
                    });
                }
            }
        }
    }

    fn parse_array(&mut self, open: SyntaxToken, depth: usize) -> Result<Value, Json5Error> {
        let depth = self.enter_container(depth, &open)?;
        let mut elements = Vec::new();

        if self.peek_kind()? == Json5Token::Json(JsonToken::RightBracket) {
            let close = self.next_token()?;
            return Ok(Value::Array(ArrayNode {
                open,
                elements,
                close,
            }));
        }

        loop {
            let value = self.parse_value(depth)?;
            let separator = self.next_token()?;

            match separator.kind {
                Json5Token::Json(JsonToken::Comma) => {
                    elements.push(Element {
                        value,
                        comma: Some(separator),
                    });

                    // JSON5: one trailing comma is allowed in a non-empty array.
                    if self.peek_kind()? == Json5Token::Json(JsonToken::RightBracket) {
                        let close = self.next_token()?;
                        return Ok(Value::Array(ArrayNode {
                            open,
                            elements,
                            close,
                        }));
                    }
                }
                Json5Token::Json(JsonToken::RightBracket) => {
                    elements.push(Element { value, comma: None });

                    return Ok(Value::Array(ArrayNode {
                        open,
                        elements,
                        close: separator,
                    }));
                }
                Json5Token::Eof => {
                    return Err(Json5Error {
                        position: separator.start,
                        kind: Json5ErrorKind::UnexpectedEof {
                            expected: "',' or ']'",
                        },
                    });
                }
                found => {
                    return Err(Json5Error {
                        position: separator.start,
                        kind: Json5ErrorKind::Expected {
                            expected: "',' or ']'",
                            found,
                        },
                    });
                }
            }
        }
    }

    /// Consumes the next token, requiring `kind`.
    fn expect(
        &mut self,
        kind: Json5Token,
        describe: &'static str,
    ) -> Result<SyntaxToken, Json5Error> {
        let token = self.next_token()?;

        if token.kind == kind {
            Ok(token)
        } else if token.kind == Json5Token::Eof {
            Err(Json5Error {
                position: token.start,
                kind: Json5ErrorKind::UnexpectedEof { expected: describe },
            })
        } else {
            Err(Json5Error {
                position: token.start,
                kind: Json5ErrorKind::Expected {
                    expected: describe,
                    found: token.kind,
                },
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_matches_the_builder_defaults() {
        let parser = Json5Parser::new();
        assert_eq!(parser.max_depth(), DEFAULT_MAX_DEPTH);

        let built = Json5Parser::builder().build();
        assert_eq!(built.max_depth(), DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn builder_sets_the_depth_limit() {
        let parser = Json5Parser::builder().max_depth(4).build();
        assert_eq!(parser.max_depth(), 4);
    }

    #[test]
    fn error_display_includes_position() {
        let Err(error) = Json5Parser::new().parse_str("[1 2]") else {
            panic!("expected a parse error");
        };
        let message = error.to_string();
        assert!(message.contains("expected ',' or ']'"), "{message}");
        assert!(message.contains("line 1"), "{message}");
        assert!(message.contains("byte 3"), "{message}");
    }

    #[test]
    fn member_name_error_names_the_expectation() {
        let Err(error) = Json5Parser::new().parse_str("{1: 2}") else {
            panic!("expected a parse error");
        };
        let message = error.to_string();
        assert!(message.contains("member name"), "{message}");
        assert!(message.contains("found a number"), "{message}");
    }

    #[test]
    fn depth_limit_counts_containers() {
        let parser = Json5Parser::builder().max_depth(1).build();
        assert!(parser.parse_str("[1]").is_ok());

        let Err(error) = parser.parse_str("[[1]]") else {
            panic!("expected a depth error");
        };
        assert!(matches!(
            error.kind,
            Json5ErrorKind::DepthLimitExceeded { limit: 1 }
        ));
    }

    #[test]
    fn warning_scan_is_escape_aware() {
        let mut warnings = Vec::new();
        // `'a<U+2028>b'` — raw separator: one warning.
        let raw = "'a\u{2028}b'";
        scan_string_warnings(raw.as_bytes(), Position::START, &mut warnings);
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings.first().map(|warning| warning.position.byte),
            Some(2)
        );

        // `'a\<U+2028>b'` — line continuation: no warning.
        let mut warnings = Vec::new();
        let continued = "'a\\\u{2028}b'";
        scan_string_warnings(continued.as_bytes(), Position::START, &mut warnings);
        assert!(warnings.is_empty());

        // `'\\<U+2028>'` — escaped backslash, then a *raw* separator: one warning.
        let mut warnings = Vec::new();
        let double = "'\\\\\u{2028}'";
        scan_string_warnings(double.as_bytes(), Position::START, &mut warnings);
        assert_eq!(warnings.len(), 1);
    }
}
