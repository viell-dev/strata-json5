//! JSON5 (json5.org) resolver for the Strata parser ecosystem: a dynamic, owned document model
//! with source fidelity.
//!
//! The resolver is the consumer-facing layer of the JSON5 stack: it drives
//! `strata-json5-parser`, then turns the zero-copy CST into owned values with JSON5 value
//! semantics — strings unescaped (single- or double-quoted, line continuations, `\xHH` escapes,
//! UTF-16 surrogate-pair combination with a configurable lone-surrogate policy), numbers
//! converted integer-preserving (hex integers included; `Infinity`, `NaN`, and signed variants
//! become floats), and member names decoded whether they were written as strings or as
//! identifiers (`{"a": 1}`, `{'a': 1}`, and `{a: 1}` all expose the name `a`, with the raw
//! spelling reported as a [`NameStyle`]) — while keeping fidelity available on every node: byte
//! spans, positions, leading trivia (whitespace runs, blank lines, and the `//` and `/* */`
//! comments JSON5 admits), member order, and duplicate members. Comments surface on the model:
//! [`Meta::comments`] iterates a node's leading comments and [`Json5Document::comment_text`]
//! extracts a comment's content with its delimiters stripped. The parser's spec-mandated
//! warnings (unescaped U+2028/U+2029 in strings) carry through to
//! [`Json5Document::warnings`]. The raw source text behind any node stays reachable through
//! [`Json5Document::bytes`] and promotable [`source_ref`](Json5Document::source_ref) handles.
//!
//! [`Json5Resolver::new`] is the fully strict path — and strict JSON5 needs no tolerance
//! switches: every JSON5 extension (identifier names, single quotes, comments, trailing commas,
//! hex numbers, ...) is a mandatory language feature, and a byte-order mark is ordinary
//! whitespace. [`Json5Resolver::builder`] exposes the parser's one spec-permitted option
//! (`max_depth`), lower-layer replacement, and the two resolution policies: [`DuplicateKeys`]
//! (retain by default, `get` resolves to the last occurrence) and [`LoneSurrogates`] (reject by
//! default). The model is read-only; editing and writing back stay at the CST/source level of
//! the lower layers.

mod document;
mod error;
mod resolve;
mod value;

pub use document::Json5Document;
pub use error::{Json5Error, Json5ErrorKind};
pub use resolve::{DuplicateKeys, Json5Resolver, Json5ResolverBuilder};
pub use strata_json5_parser::decode::LoneSurrogates;
pub use strata_json5_parser::{Trivia, TriviaKind, Warning, WarningKind};
pub use value::{Json5Array, Json5Member, Json5Number, Json5Object, Json5Value, Meta, NameStyle};

/// Compile-checks the README examples alongside the doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
mod readme {}
