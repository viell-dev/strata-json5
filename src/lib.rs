//! Strict JSON5 parser for the Strata parser ecosystem, composed on top of `strata-json`.
//!
//! `strata-json5` turns a byte stream into a lossless, zero-copy concrete syntax tree (CST):
//! every token keeps its absolute byte span, trivia — whitespace runs, `//` and `/* */`
//! comments, blank lines — is retained instead of discarded, and string/number content stays as
//! raw source bytes. Decoding (escape resolution, numeric conversion) belongs to a downstream
//! resolver or deserializer.
//!
//! The language is JSON5 as specified at <https://spec.json5.org/>, exactly: every JSON5
//! extension (unquoted `IdentifierName` member names, single-quoted strings, line continuations
//! and the extra escapes, hex numbers, `Infinity`/`NaN`, explicit `+`, leading/trailing decimal
//! points, trailing commas, comments, the expanded whitespace set with the byte-order mark as
//! whitespace) is a mandatory language feature, not an option — and nothing beyond JSON5 is
//! accepted, per the ecosystem's strict-by-default policy. The one caveat is Unicode identifier
//! classification, which approximates the ES 5.1 category sets using `std` (exact for ASCII and
//! `\uXXXX` escapes); see [`rules::ident`] for the precise deviations.
//!
//! The crate has three public layers, mirroring `strata-json`:
//!
//! - [`rules`] — the JSON5 token rules. The structural-punctuation and `true`/`false`/`null`
//!   rules are reused from `strata-json` (lifted into [`Json5Token`]); the shapes JSON5 widens
//!   or adds are JSON5-owned rules. Each rule is exported individually so supersets of JSON5
//!   can compose further.
//! - [`cst`] — the document shape: [`Document`], [`Value`], nodes, tokens, trivia, and the
//!   spec-mandated [`Warning`]s.
//! - [`parse`] — [`Json5Parser`], strict via [`Json5Parser::new`], with a builder for the
//!   spec-permitted nesting-depth limit and for replacing the lower layers.
//!
//! # Example
//!
//! ```
//! use strata_json5::{Json5Parser, Json5Token, Value};
//!
//! # fn main() -> Result<(), strata_json5::Json5Error> {
//! let document = Json5Parser::new().parse_str(
//!     "{\n  // package metadata\n  name: 'strata', versions: [0x1, .5, +Infinity,],\n}",
//! )?;
//!
//! let Value::Object(object) = document.root() else {
//!     panic!("expected an object");
//! };
//! let member = object.members.first().unwrap();
//!
//! // Unquoted member name, with its comment retained as leading trivia.
//! assert_eq!(member.name.kind, Json5Token::Identifier);
//! assert_eq!(document.token_bytes(&member.name), Some(&b"name"[..]));
//! assert_eq!(member.name.leading.len(), 3); // whitespace, comment, whitespace
//!
//! // Zero-copy: raw source bytes, quotes included, no decoding.
//! assert_eq!(document.token_bytes(&member.value.first_token()), Some(&b"'strata'"[..]));
//! # Ok(())
//! # }
//! ```

pub mod cst;
pub mod parse;
pub mod rules;

pub use cst::{
    ArrayNode, Document, Element, Member, ObjectNode, SyntaxToken, Trivia, TriviaKind, Value,
    Warning, WarningKind,
};
pub use parse::{DEFAULT_MAX_DEPTH, Json5Error, Json5ErrorKind, Json5Parser, Json5ParserBuilder};
pub use rules::{
    BlockCommentRule, IdentifierRule, Json5Token, JsonToken, LineCommentRule, LiteralRule,
    NumberRule, PunctRule, SharedRuleSet, StrictRuleSet, StringRule, WhitespaceRule,
    strict_rule_set, strict_scanner,
};
