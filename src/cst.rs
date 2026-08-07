//! The concrete syntax tree (CST): a lossless, zero-copy document shape for JSON5.
//!
//! The shape deliberately mirrors `strata-json`'s CST — the same node names, the same
//! leading-trivia attachment, the same source-retention model — but is a distinct set of types:
//! `strata-json`'s `SyntaxToken` is hardwired to its `JsonToken` kind and its `TriviaKind` has no
//! comment variants, so it cannot represent identifier member names or retained comments. (Making
//! those types kind-generic upstream would let a superset crate reuse them outright; noted as a
//! possible `strata-json` follow-up.)
//!
//! JSON5 additions visible here:
//!
//! - [`TriviaKind`] gains `LineComment` and `BlockComment`: comments are retained trivia attached
//!   to the following token, exactly like whitespace runs.
//! - A [`Member`]'s `name` token can be an identifier (or one of the word tokens
//!   `true`/`false`/`null`/`Infinity`/`NaN`, which are valid identifier names) as well as a
//!   string.
//! - [`Document::warnings`] carries the spec-mandated parse warnings (unescaped U+2028/U+2029 in
//!   strings).
//!
//! Duplicate member names are retained verbatim, in source order. The [spec] merely observes
//! that software receiving an object with non-unique names behaves unpredictably; a
//! fidelity-preserving CST records what was written and leaves duplicate policy downstream, so
//! there is no option.
//!
//! [spec]: https://spec.json5.org/

use crate::rules::Json5Token;
use std::fmt;
use strata_lexer::{Position, SourceRef, SourceStore, Span};

/// Classification of a retained trivia piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriviaKind {
    /// A run of JSON5 whitespace, including line terminators and any byte-order mark.
    Whitespace,
    /// A `//` comment (terminating line break excluded — it is whitespace).
    LineComment,
    /// A `/* ... */` comment, delimiters included.
    BlockComment,
}

/// A retained piece of grammatically insignificant source content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trivia {
    /// What kind of trivia this piece is.
    pub kind: TriviaKind,
    /// Absolute byte range in the source.
    pub span: Span,
    /// Position of the first byte.
    pub start: Position,
    /// Position just past the last byte.
    pub end: Position,
}

impl Trivia {
    /// The number of line breaks inside this trivia piece.
    ///
    /// Line counting is the byte-oriented lexer core's: `\r`, `\n`, and `\r\n` count; U+2028 and
    /// U+2029 do not advance the diagnostic line number.
    pub const fn line_breaks(&self) -> u64 {
        self.end.line.saturating_sub(self.start.line)
    }

    /// The number of blank lines inside this trivia piece.
    ///
    /// A whitespace run separating two tokens contains one line break when the tokens merely sit
    /// on adjacent lines; every additional break is a blank line — the kind of implicit visual
    /// grouping the Strata ecosystem preserves.
    pub const fn blank_lines(&self) -> u64 {
        self.line_breaks().saturating_sub(1)
    }
}

/// A significant token in the CST: kind, span, positions, and the trivia that preceded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxToken {
    /// The token kind.
    pub kind: Json5Token,
    /// Absolute byte range of the lexeme (excluding leading trivia).
    pub span: Span,
    /// Position of the first lexeme byte.
    pub start: Position,
    /// Position just past the last lexeme byte.
    pub end: Position,
    /// Trivia between the previous significant token and this one, in source order — whitespace
    /// runs and comments.
    pub leading: Vec<Trivia>,
}

/// A JSON5 value node.
///
/// Scalar variants carry the token whose span addresses the raw source bytes — a string token's
/// span includes both quotes, and its content is *not* decoded here. Container variants keep
/// every structural token, including the commas between (and trailing) entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// The `null` literal.
    Null(SyntaxToken),
    /// A `true` or `false` literal.
    Bool {
        /// The literal's value.
        value: bool,
        /// The literal token.
        token: SyntaxToken,
    },
    /// A number; the token span covers the raw, unconverted source text. This includes hex
    /// integers, signed numbers, and `Infinity`/`NaN` (whose tokens keep their word kinds
    /// [`Json5Token::Infinity`]/[`Json5Token::NaN`] when unsigned).
    Number(SyntaxToken),
    /// A string; the token span covers the raw source text including both quotes (single or
    /// double).
    String(SyntaxToken),
    /// An array.
    Array(ArrayNode),
    /// An object.
    Object(ObjectNode),
}

impl Value {
    /// The first significant token of this value, whose leading trivia is everything between the
    /// previous token and this value.
    pub const fn first_token(&self) -> &SyntaxToken {
        match self {
            Self::Null(token)
            | Self::Number(token)
            | Self::String(token)
            | Self::Bool { token, .. } => token,
            Self::Array(array) => &array.open,
            Self::Object(object) => &object.open,
        }
    }

    /// The last significant token of this value.
    pub const fn last_token(&self) -> &SyntaxToken {
        match self {
            Self::Null(token)
            | Self::Number(token)
            | Self::String(token)
            | Self::Bool { token, .. } => token,
            Self::Array(array) => &array.close,
            Self::Object(object) => &object.close,
        }
    }

    /// The absolute byte range this value covers, from its first to its last lexeme byte
    /// (leading trivia excluded).
    pub const fn span(&self) -> Span {
        Span::new(self.first_token().span.start, self.last_token().span.end)
    }

    /// Visits every significant token of this value in source order.
    pub fn for_each_token<'value>(&'value self, visit: &mut impl FnMut(&'value SyntaxToken)) {
        match self {
            Self::Null(token)
            | Self::Number(token)
            | Self::String(token)
            | Self::Bool { token, .. } => visit(token),
            Self::Array(array) => {
                visit(&array.open);

                for element in &array.elements {
                    element.value.for_each_token(visit);

                    if let Some(comma) = &element.comma {
                        visit(comma);
                    }
                }

                visit(&array.close);
            }
            Self::Object(object) => {
                visit(&object.open);

                for member in &object.members {
                    visit(&member.name);
                    visit(&member.colon);
                    member.value.for_each_token(visit);

                    if let Some(comma) = &member.comma {
                        visit(comma);
                    }
                }

                visit(&object.close);
            }
        }
    }
}

/// An array node: `[` *elements* `]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayNode {
    /// The `[` token.
    pub open: SyntaxToken,
    /// The elements in source order.
    pub elements: Vec<Element>,
    /// The `]` token.
    pub close: SyntaxToken,
}

/// One array element and the comma that followed it, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// The element value.
    pub value: Value,
    /// The `,` after this element; `None` only on a last element without a trailing comma (JSON5
    /// permits one trailing comma in a non-empty array).
    pub comma: Option<SyntaxToken>,
}

/// An object node: `{` *members* `}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectNode {
    /// The `{` token.
    pub open: SyntaxToken,
    /// The members in source order, duplicates included.
    pub members: Vec<Member>,
    /// The `}` token.
    pub close: SyntaxToken,
}

/// One object member: *name* `:` *value*, plus the comma that followed it, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The member name. Its kind distinguishes the written form: [`Json5Token::String`] (span
    /// includes both quotes), [`Json5Token::Identifier`], or one of the word kinds
    /// (`Json(True/False/Null)`, `Infinity`, `NaN`) — all valid `IdentifierName`s.
    pub name: SyntaxToken,
    /// The `:` token.
    pub colon: SyntaxToken,
    /// The member value.
    pub value: Value,
    /// The `,` after this member; `None` only on a last member without a trailing comma (JSON5
    /// permits one trailing comma in a non-empty object).
    pub comma: Option<SyntaxToken>,
}

/// Categories of spec-mandated parse warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningKind {
    /// An unescaped U+2028 LINE SEPARATOR or U+2029 PARAGRAPH SEPARATOR inside a string.
    ///
    /// These are legal JSON5 string content, but the [spec] says parsers *should* produce a
    /// warning when they are found unescaped in strings (they break ECMAScript compatibility
    /// before ES2019). The parse succeeds and the warning is recorded on the document.
    ///
    /// [spec]: https://spec.json5.org/
    UnescapedLineSeparator {
        /// The offending character: U+2028 or U+2029.
        separator: char,
    },
}

/// A spec-mandated parse warning: the parse succeeded, but the input carries a construct the
/// JSON5 spec says parsers *should* warn about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Position of the offending character.
    pub position: Position,
    /// What was found.
    pub kind: WarningKind,
}

impl fmt::Display for Warning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            WarningKind::UnescapedLineSeparator { separator } => write!(
                formatter,
                "unescaped U+{:04X} in a string at line {}, column {} (byte {})",
                u32::from(separator),
                self.position.line,
                self.position.column,
                self.position.byte
            ),
        }
    }
}

/// A parsed document: the root value, trailing trivia, warnings, and the retained source.
///
/// The document owns the [`SourceStore`] holding the original bytes, so every span in the tree
/// stays addressable — zero-copy — for as long as the document lives. [`Document::source_ref`]
/// hands out promotable handles that survive even store reclamation (see `strata-lexer`'s
/// retention layer); [`Document::bytes`] is the plain borrow path.
#[derive(Debug)]
pub struct Document {
    root: Value,
    trailing: Vec<Trivia>,
    warnings: Vec<Warning>,
    source: SourceStore,
}

impl Document {
    /// Assembles a document from its parts. Used by the parser.
    pub(crate) const fn new(
        root: Value,
        trailing: Vec<Trivia>,
        warnings: Vec<Warning>,
        source: SourceStore,
    ) -> Self {
        Self {
            root,
            trailing,
            warnings,
            source,
        }
    }

    /// The root value. Any JSON5 value is a valid top-level document.
    pub const fn root(&self) -> &Value {
        &self.root
    }

    /// Trivia between the end of the root value and the end of input.
    pub fn trailing_trivia(&self) -> &[Trivia] {
        &self.trailing
    }

    /// The spec-mandated warnings recorded during the parse (see [`WarningKind`]), in source
    /// order. An empty slice means a warning-free document.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Borrows the raw source bytes covered by `span`, zero-copy.
    ///
    /// Returns `None` if the span is not (or no longer) backed by the retained source, e.g.
    /// after [`Document::source_mut`] was used to retire buffers.
    pub fn bytes(&self, span: Span) -> Option<&[u8]> {
        self.source.bytes(span)
    }

    /// Borrows the raw lexeme bytes of `token`, zero-copy.
    pub fn token_bytes(&self, token: &SyntaxToken) -> Option<&[u8]> {
        self.bytes(token.span)
    }

    /// Creates a promotable, owned-on-demand handle to the bytes covered by `span`.
    ///
    /// This is the resolver-facing path: the handle stays zero-copy while the store retains the
    /// buffer and degrades to a small owned copy if the store reclaims it.
    pub fn source_ref(&mut self, span: Span) -> Option<SourceRef> {
        self.source.source_ref(span)
    }

    /// The retained source store.
    pub const fn source(&self) -> &SourceStore {
        &self.source
    }

    /// Mutable access to the retained source store, e.g. for owner-side reclamation
    /// (`maintain` / `retire_all`).
    pub const fn source_mut(&mut self) -> &mut SourceStore {
        &mut self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn position(byte: u64, line: u64, column: u64) -> Position {
        Position { byte, line, column }
    }

    fn trivia(kind: TriviaKind, start: Position, end: Position) -> Trivia {
        Trivia {
            kind,
            span: Span::new(start.byte, end.byte),
            start,
            end,
        }
    }

    #[test]
    fn blank_line_counting_matches_strata_json() {
        let piece = trivia(TriviaKind::Whitespace, position(1, 1, 2), position(2, 1, 3));
        assert_eq!(piece.line_breaks(), 0);
        assert_eq!(piece.blank_lines(), 0);

        let piece = trivia(TriviaKind::Whitespace, position(1, 1, 2), position(3, 3, 1));
        assert_eq!(piece.line_breaks(), 2);
        assert_eq!(piece.blank_lines(), 1);
    }

    #[test]
    fn value_token_walk_covers_containers() {
        let token = |kind: Json5Token, start: u64, end: u64| SyntaxToken {
            kind,
            span: Span::new(start, end),
            start: position(start, 1, start.saturating_add(1)),
            end: position(end, 1, end.saturating_add(1)),
            leading: Vec::new(),
        };
        let array = Value::Array(ArrayNode {
            open: token(Json5Token::Json(crate::rules::JsonToken::LeftBracket), 0, 1),
            elements: vec![Element {
                value: Value::Number(token(Json5Token::Number, 1, 2)),
                comma: Some(token(
                    Json5Token::Json(crate::rules::JsonToken::Comma),
                    2,
                    3,
                )),
            }],
            close: token(
                Json5Token::Json(crate::rules::JsonToken::RightBracket),
                3,
                4,
            ),
        });

        assert_eq!(array.span(), Span::new(0, 4));

        let mut kinds = Vec::new();
        array.for_each_token(&mut |token| kinds.push(token.kind));
        assert_eq!(
            kinds,
            vec![
                Json5Token::Json(crate::rules::JsonToken::LeftBracket),
                Json5Token::Number,
                Json5Token::Json(crate::rules::JsonToken::Comma),
                Json5Token::Json(crate::rules::JsonToken::RightBracket),
            ]
        );
    }

    #[test]
    fn warning_display_names_the_code_point() {
        let warning = Warning {
            position: position(5, 2, 3),
            kind: WarningKind::UnescapedLineSeparator {
                separator: '\u{2028}',
            },
        };
        let message = warning.to_string();
        assert!(message.contains("U+2028"), "{message}");
        assert!(message.contains("line 2"), "{message}");
    }
}
