//! The resolver: CST in, owned document model out.

use crate::document::Json5Document;
use crate::error::{Json5Error, Json5ErrorKind};
use crate::value::Json5Array;
use crate::value::{Json5Member, Json5Object, Json5Value, Meta, NameStyle};
use std::collections::HashSet;
use std::io::{Cursor, Read};
use strata_json5_parser::decode::{self, LoneSurrogates};
use strata_json5_parser::{
    Json5Parser, Json5ParserBuilder, Json5Token, StrictRuleSet, SyntaxToken, Value,
};
use strata_lexer::{LexerConfig, MatcherSet, SourceStore, Span};

/// Policy for repeated object member names.
///
/// The JSON5 spec inherits RFC 8259 §4's latitude: names *should* be unique, software receiving
/// duplicates behaves unpredictably, so a resolver chooses a policy up front. Names are compared
/// decoded, so different spellings of one name (quoted, single-quoted, identifier,
/// escape-carrying) collide.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DuplicateKeys {
    /// Retain every member in source order; [`Json5Object::get`] resolves to the last
    /// occurrence. The default.
    #[default]
    Retain,
    /// Reject the document with [`Json5ErrorKind::DuplicateMember`] at the repeated name.
    Error,
}

/// A reusable JSON5 resolver: parses input and resolves the CST into a [`Json5Document`].
///
/// `M` is the token rule set handed down to the parser; it defaults to the strict JSON5 rules.
/// [`Json5Resolver::new`] is the fully strict path — every JSON5 extension is a mandatory
/// language feature, so there are no tolerance switches. [`Json5Resolver::builder`] exposes the
/// parser's spec-permitted depth limit plus the two resolution policies ([`DuplicateKeys`],
/// [`LoneSurrogates`]).
#[derive(Debug, Clone)]
pub struct Json5Resolver<M = StrictRuleSet> {
    parser: Json5Parser<M>,
    duplicate_keys: DuplicateKeys,
    lone_surrogates: LoneSurrogates,
}

impl Json5Resolver<StrictRuleSet> {
    /// Creates a fully strict resolver: strict parser defaults (the JSON5 extensions are core
    /// language, not options), duplicates retained, lone surrogates rejected.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser: Json5Parser::new(),
            duplicate_keys: DuplicateKeys::default(),
            lone_surrogates: LoneSurrogates::default(),
        }
    }

    /// Starts a builder for customizing options or replacing lower layers.
    #[must_use]
    pub fn builder() -> Json5ResolverBuilder<StrictRuleSet> {
        Json5ResolverBuilder::new()
    }
}

impl Default for Json5Resolver<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Json5Resolver<M>
where
    M: MatcherSet<Kind = Json5Token> + Clone,
{
    /// Parses and resolves a complete JSON5 text from `reader`.
    pub fn resolve<R: Read>(&self, reader: R) -> Result<Json5Document, Json5Error> {
        self.resolve_document(self.parser.parse(reader)?)
    }

    /// Parses and resolves a complete JSON5 text from a string slice.
    pub fn resolve_str(&self, input: &str) -> Result<Json5Document, Json5Error> {
        self.resolve(Cursor::new(input.as_bytes()))
    }

    /// Parses and resolves a complete JSON5 text from a byte slice.
    pub fn resolve_bytes(&self, input: &[u8]) -> Result<Json5Document, Json5Error> {
        self.resolve(Cursor::new(input))
    }

    /// Resolves an already-parsed [`Document`](strata_json5_parser::Document), carrying its
    /// recorded warnings onto the resolved document.
    ///
    /// The document's retained source must still back the tree's spans; a document whose store
    /// was reclaimed first resolves to [`Json5ErrorKind::SourceUnavailable`].
    pub fn resolve_document(
        &self,
        document: strata_json5_parser::Document,
    ) -> Result<Json5Document, Json5Error> {
        let (root, trailing, source, warnings) = document.into_parts();
        let resolution = Resolution {
            source: &source,
            lone_surrogates: self.lone_surrogates,
            duplicate_keys: self.duplicate_keys,
        };
        let root = resolution.resolve_value(&root)?;

        Ok(Json5Document::new(root, trailing, warnings, source))
    }

    /// The configured duplicate-member policy.
    #[must_use]
    pub const fn duplicate_keys(&self) -> DuplicateKeys {
        self.duplicate_keys
    }

    /// The configured lone-surrogate policy.
    #[must_use]
    pub const fn lone_surrogates(&self) -> LoneSurrogates {
        self.lone_surrogates
    }

    /// The underlying parser.
    #[must_use]
    pub const fn parser(&self) -> &Json5Parser<M> {
        &self.parser
    }
}

/// Builder for [`Json5Resolver`], strict by default (there is no other mode: the only knobs are
/// the spec-permitted depth limit, the lower-layer replacement hooks, and the resolution
/// policies).
#[derive(Debug, Clone)]
pub struct Json5ResolverBuilder<M = StrictRuleSet> {
    parser: Json5ParserBuilder<M>,
    duplicate_keys: DuplicateKeys,
    lone_surrogates: LoneSurrogates,
}

impl Json5ResolverBuilder<StrictRuleSet> {
    /// Starts from the fully strict defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser: Json5ParserBuilder::new(),
            duplicate_keys: DuplicateKeys::default(),
            lone_surrogates: LoneSurrogates::default(),
        }
    }
}

impl Default for Json5ResolverBuilder<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> Json5ResolverBuilder<M> {
    /// Replaces the token rule set handed down to the parser.
    pub fn with_rules<N>(self, rules: N) -> Json5ResolverBuilder<N>
    where
        N: MatcherSet<Kind = Json5Token> + Clone,
    {
        Json5ResolverBuilder {
            parser: self.parser.with_rules(rules),
            duplicate_keys: self.duplicate_keys,
            lone_surrogates: self.lone_surrogates,
        }
    }

    /// Replaces the lexer configuration handed down to the parser.
    #[must_use]
    pub fn with_lexer_config(mut self, config: LexerConfig) -> Self {
        self.parser = self.parser.with_lexer_config(config);
        self
    }

    /// Sets the parser's nesting-depth limit (the spec's one permitted implementation limit).
    #[must_use]
    pub fn max_depth(mut self, limit: usize) -> Self {
        self.parser = self.parser.max_depth(limit);
        self
    }

    /// Sets the repeated-member-name policy. Retain by default.
    #[must_use]
    pub const fn duplicate_keys(mut self, policy: DuplicateKeys) -> Self {
        self.duplicate_keys = policy;
        self
    }

    /// Sets the lone-surrogate policy for string decoding. Error by default.
    #[must_use]
    pub const fn lone_surrogates(mut self, policy: LoneSurrogates) -> Self {
        self.lone_surrogates = policy;
        self
    }

    /// Finishes the builder.
    pub fn build(self) -> Json5Resolver<M>
    where
        M: MatcherSet<Kind = Json5Token> + Clone,
    {
        Json5Resolver {
            parser: self.parser.build(),
            duplicate_keys: self.duplicate_keys,
            lone_surrogates: self.lone_surrogates,
        }
    }
}

/// One resolution pass over a parsed tree, reading raw lexemes from the retained source.
struct Resolution<'source> {
    source: &'source SourceStore,
    lone_surrogates: LoneSurrogates,
    duplicate_keys: DuplicateKeys,
}

impl Resolution<'_> {
    fn resolve_value(&self, value: &Value) -> Result<Json5Value, Json5Error> {
        match value {
            Value::Null(token) => Ok(Json5Value::Null(token_meta(token))),
            Value::Bool { value, token } => Ok(Json5Value::Bool(*value, token_meta(token))),
            Value::Number(token) => {
                let raw = self.raw_bytes(token.span())?;
                let number =
                    decode::parse_number(&raw).map_err(|error| Self::decode_error(token, error))?;

                Ok(Json5Value::Number(number, token_meta(token)))
            }
            Value::String(token) => {
                let raw = self.raw_bytes(token.span())?;
                let text = decode::unescape_string(&raw, self.lone_surrogates)
                    .map_err(|error| Self::decode_error(token, error))?;

                Ok(Json5Value::String(text, token_meta(token)))
            }
            Value::Array(array) => {
                let elements = array
                    .elements()
                    .iter()
                    .map(|element| self.resolve_value(element.value()))
                    .collect::<Result<Vec<_>, _>>()?;
                let meta = node_meta(array.open(), array.close());

                Ok(Json5Value::Array(Json5Array::new(meta, elements)))
            }
            Value::Object(object) => {
                let mut members = Vec::with_capacity(object.members().len());

                for member in object.members() {
                    let (name, style) = self.decode_name(member.name())?;
                    let name_meta = token_meta(member.name());
                    let value = self.resolve_value(member.value())?;
                    members.push(Json5Member::new(name, style, name_meta, value));
                }

                if self.duplicate_keys == DuplicateKeys::Error {
                    reject_duplicates(&members)?;
                }

                let meta = node_meta(object.open(), object.close());

                Ok(Json5Value::Object(Json5Object::new(meta, members)))
            }
        }
    }

    /// Decodes a member-name token — quoted string or identifier — into its owned text and
    /// written style, mapping decode failures onto resolver errors.
    fn decode_name(&self, token: &SyntaxToken) -> Result<(String, NameStyle), Json5Error> {
        let raw = self.raw_bytes(token.span())?;
        let style = decode::member_name_shape(&raw);
        let name = decode::decode_member_name(&raw, self.lone_surrogates)
            .map_err(|error| Self::decode_error(token, error))?;

        Ok((name, style))
    }

    /// Reads the raw bytes behind `span`, borrowing when one buffer covers it.
    fn raw_bytes(&self, span: Span) -> Result<std::borrow::Cow<'_, [u8]>, Json5Error> {
        if let Some(bytes) = self.source.bytes(span) {
            return Ok(std::borrow::Cow::Borrowed(bytes));
        }

        match self.source.copy_bytes(span) {
            Some(bytes) => Ok(std::borrow::Cow::Owned(bytes)),
            None => Err(Json5Error {
                position: strata_lexer::Position::START,
                kind: Json5ErrorKind::SourceUnavailable { span },
            }),
        }
    }

    /// Maps a decode failure at `token` onto a resolver error with an absolute position.
    ///
    /// The byte offset is exact: it advances the token's start by the failure's offset inside
    /// the lexeme. The line and column stay those of the token's start line — JSON5 string
    /// lexemes *can* span lines (line continuations, raw U+2028/U+2029), so for a failure past
    /// such a break the reported line may understate and the column overstate; the byte offset
    /// remains the authoritative location.
    fn decode_error(token: &SyntaxToken, error: decode::Error) -> Json5Error {
        let position = token
            .start()
            .advanced_by(u64::try_from(error.offset).unwrap_or(u64::MAX));

        match error.kind {
            decode::ErrorKind::LoneSurrogate { unit } => Json5Error {
                position,
                kind: Json5ErrorKind::LoneSurrogate { unit },
            },
            decode::ErrorKind::MalformedToken => Json5Error {
                position,
                kind: Json5ErrorKind::InvalidLexeme { error },
            },
        }
    }
}

/// Builds the fidelity metadata of a scalar or name token.
fn token_meta(token: &SyntaxToken) -> Meta {
    Meta::new(
        token.span(),
        token.start(),
        token.end(),
        token.leading().to_vec(),
    )
}

/// Builds the fidelity metadata of a container from its open and close tokens.
fn node_meta(open: &SyntaxToken, close: &SyntaxToken) -> Meta {
    let start = open.span().start();
    let span = Span::from_start_len(start, close.span().end().saturating_sub(start));

    Meta::new(span, open.start(), close.end(), open.leading().to_vec())
}

/// Rejects the first member shadowing an earlier name, blaming the later occurrence.
fn reject_duplicates(members: &[Json5Member]) -> Result<(), Json5Error> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(members.len());

    for member in members {
        if !seen.insert(member.name()) {
            return Err(Json5Error {
                position: member.name_meta().start(),
                kind: Json5ErrorKind::DuplicateMember {
                    name: member.name().to_owned(),
                },
            });
        }
    }

    Ok(())
}
