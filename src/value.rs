//! The owned document model: values with decoded content and per-node fidelity.

use strata_json5_parser::{Trivia, TriviaKind};
use strata_lexer::{Position, Span};

pub use strata_json5_parser::decode::Number as Json5Number;
pub use strata_json5_parser::decode::NumberFormat;

/// The written style of an object member name: double-quoted, single-quoted, or an unquoted
/// identifier. Re-exported from the parser's decode module ([`NameShape`]); see
/// [`Json5Member::name_style`].
///
/// [`NameShape`]: strata_json5_parser::decode::NameShape
pub use strata_json5_parser::decode::NameShape as NameStyle;

/// Per-node source fidelity: where the node sits and what preceded it.
///
/// Metas are only created by resolution, from CST nodes whose positions are ordered by
/// construction; the raw source bytes behind [`Meta::span`] stay reachable through
/// [`Json5Document::bytes`](crate::Json5Document::bytes). The leading trivia includes any
/// comments written before the node; [`Meta::comments`] filters down to just those, and
/// [`Json5Document::comment_text`](crate::Json5Document::comment_text) extracts their content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    span: Span,
    start: Position,
    end: Position,
    leading: Vec<Trivia>,
}

impl Meta {
    pub(crate) const fn new(
        span: Span,
        start: Position,
        end: Position,
        leading: Vec<Trivia>,
    ) -> Self {
        Self {
            span,
            start,
            end,
            leading,
        }
    }

    /// The absolute byte range of the node's lexemes, first to last (leading trivia excluded).
    #[inline]
    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Position of the node's first lexeme byte.
    #[inline]
    #[must_use]
    pub const fn start(&self) -> Position {
        self.start
    }

    /// Position just past the node's last lexeme byte.
    #[inline]
    #[must_use]
    pub const fn end(&self) -> Position {
        self.end
    }

    /// Trivia between the previous token and this node, in source order.
    #[inline]
    #[must_use]
    pub fn leading(&self) -> &[Trivia] {
        &self.leading
    }

    /// The comments among the leading trivia, in source order.
    pub fn comments(&self) -> impl Iterator<Item = &Trivia> {
        self.leading.iter().filter(|piece| {
            matches!(
                piece.kind(),
                TriviaKind::LineComment | TriviaKind::BlockComment
            )
        })
    }
}

/// An owned JSON5 value with decoded content and fidelity metadata.
///
/// Strings are unescaped and numbers converted with JSON5 semantics ([`Json5Number`] preserves
/// integer identity where the value fits — hex integers included — and `Infinity`/`NaN` become
/// floats); the raw source text of every node stays reachable through its [`Meta`] span and the
/// document. Object member order, duplicate members, and comments are retained.
#[derive(Debug, Clone, PartialEq)]
pub enum Json5Value {
    /// The `null` literal.
    Null(Meta),
    /// A `true` or `false` literal.
    Bool(bool, Meta),
    /// A number, converted integer-preserving.
    Number(Json5Number, NumberFormat, Meta),
    /// A string, unescaped.
    String(String, Meta),
    /// An array.
    Array(Json5Array),
    /// An object.
    Object(Json5Object),
}

impl Json5Value {
    /// The node's fidelity metadata.
    #[must_use]
    pub const fn meta(&self) -> &Meta {
        match self {
            Self::Null(meta)
            | Self::Bool(_, meta)
            | Self::Number(_, _, meta)
            | Self::String(_, meta) => meta,
            Self::Array(array) => array.meta(),
            Self::Object(object) => object.meta(),
        }
    }

    /// The absolute byte range the node covers.
    #[must_use]
    pub const fn span(&self) -> Span {
        self.meta().span()
    }

    /// Returns `true` for `null`.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null(_))
    }

    /// The boolean value, if this is a boolean.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value, _) => Some(*value),
            _ => None,
        }
    }

    /// The converted number, if this is a number.
    #[must_use]
    pub const fn as_number(&self) -> Option<Json5Number> {
        match self {
            Self::Number(value, _, _) => Some(*value),
            _ => None,
        }
    }

    /// The written form of the number, if this is a number.
    ///
    /// Author-intent fidelity: `1.23e+5` reports [`NumberFormat::Exponential`] and `0x1F`
    /// reports [`NumberFormat::Hex`] even though the converted values are plain numbers; the
    /// raw text stays reachable through the node's span.
    #[must_use]
    pub const fn number_format(&self) -> Option<NumberFormat> {
        match self {
            Self::Number(_, format, _) => Some(*format),
            _ => None,
        }
    }

    /// The number as `f64`, if this is a number; integers convert (rounding beyond 2^53).
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        self.as_number().map(Json5Number::as_f64)
    }

    /// The decoded string content, if this is a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value, _) => Some(value),
            _ => None,
        }
    }

    /// The array, if this is an array.
    #[must_use]
    pub const fn as_array(&self) -> Option<&Json5Array> {
        match self {
            Self::Array(array) => Some(array),
            _ => None,
        }
    }

    /// The object, if this is an object.
    #[must_use]
    pub const fn as_object(&self) -> Option<&Json5Object> {
        match self {
            Self::Object(object) => Some(object),
            _ => None,
        }
    }
}

/// An owned JSON5 array.
#[derive(Debug, Clone, PartialEq)]
pub struct Json5Array {
    meta: Meta,
    elements: Vec<Json5Value>,
}

impl Json5Array {
    pub(crate) const fn new(meta: Meta, elements: Vec<Json5Value>) -> Self {
        Self { meta, elements }
    }

    /// The array's fidelity metadata; the span covers `[` through `]`.
    #[inline]
    #[must_use]
    pub const fn meta(&self) -> &Meta {
        &self.meta
    }

    /// The elements in source order.
    #[inline]
    #[must_use]
    pub fn elements(&self) -> &[Json5Value] {
        &self.elements
    }

    /// The number of elements.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Returns `true` when the array has no elements.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The element at `index`, if present.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Json5Value> {
        self.elements.get(index)
    }

    /// Iterates the elements in source order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Json5Value> {
        self.elements.iter()
    }
}

/// An owned JSON5 object: members in source order, duplicates retained.
///
/// The JSON5 spec merely observes that software receiving an object with non-unique names
/// behaves unpredictably, inheriting RFC 8259 §4's latitude. The model keeps what was written;
/// [`Json5Object::get`] resolves a name to its **last** occurrence — the behavior of most JSON
/// implementations (ECMA-262's `JSON.parse` keeps the last value) — while
/// [`Json5Object::members`], [`Json5Object::get_all`], and [`Json5Object::duplicates`] expose
/// the full fidelity. Names are compared decoded, so different spellings of one name collide. A
/// resolver configured with [`DuplicateKeys::Error`](crate::DuplicateKeys::Error) rejects
/// duplicates up front instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Json5Object {
    meta: Meta,
    members: Vec<Json5Member>,
}

impl Json5Object {
    pub(crate) const fn new(meta: Meta, members: Vec<Json5Member>) -> Self {
        Self { meta, members }
    }

    /// The object's fidelity metadata; the span covers `{` through `}`.
    #[inline]
    #[must_use]
    pub const fn meta(&self) -> &Meta {
        &self.meta
    }

    /// The members in source order, duplicates included.
    #[inline]
    #[must_use]
    pub fn members(&self) -> &[Json5Member] {
        &self.members
    }

    /// The number of members, duplicates included.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns `true` when the object has no members.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The value of the **last** member named `name`, if any. `name` is matched against the
    /// decoded member names, whatever their written style.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Json5Value> {
        self.members
            .iter()
            .rev()
            .find(|member| member.name() == name)
            .map(Json5Member::value)
    }

    /// Every member named `name`, in source order.
    pub fn get_all<'object>(
        &'object self,
        name: &'object str,
    ) -> impl Iterator<Item = &'object Json5Member> {
        self.members
            .iter()
            .filter(move |member| member.name() == name)
    }

    /// Returns `true` when a member named `name` exists.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.members.iter().any(|member| member.name() == name)
    }

    /// Iterates the members in source order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Json5Member> {
        self.members.iter()
    }

    /// The members shadowed by a later member with the same name, in source order.
    pub fn duplicates(&self) -> impl Iterator<Item = &Json5Member> {
        self.members
            .iter()
            .enumerate()
            .filter_map(|(index, member)| {
                let shadowed = self
                    .members
                    .get(index.saturating_add(1)..)
                    .is_some_and(|later| later.iter().any(|other| other.name() == member.name()));

                shadowed.then_some(member)
            })
    }
}

/// One owned object member: decoded name, written name style, name fidelity, and value.
#[derive(Debug, Clone, PartialEq)]
pub struct Json5Member {
    name: String,
    name_style: NameStyle,
    name_meta: Meta,
    value: Json5Value,
}

impl Json5Member {
    pub(crate) const fn new(
        name: String,
        name_style: NameStyle,
        name_meta: Meta,
        value: Json5Value,
    ) -> Self {
        Self {
            name,
            name_style,
            name_meta,
            value,
        }
    }

    /// The decoded member name: quoted names are unescaped, identifier names have their
    /// `\uXXXX` escapes resolved — `{"a": 1}`, `{'a': 1}`, and `{a: 1}` all expose the name
    /// `a`.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The written style of the member name: double-quoted, single-quoted, or an unquoted
    /// identifier. The raw spelling itself stays reachable through [`Json5Member::name_meta`]
    /// and the document.
    #[inline]
    #[must_use]
    pub const fn name_style(&self) -> NameStyle {
        self.name_style
    }

    /// The name token's fidelity metadata; its span covers the raw name including any quotes,
    /// and its leading trivia carries any comments written before the member.
    #[inline]
    #[must_use]
    pub const fn name_meta(&self) -> &Meta {
        &self.name_meta
    }

    /// The member value.
    #[inline]
    #[must_use]
    pub const fn value(&self) -> &Json5Value {
        &self.value
    }
}
