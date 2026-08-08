//! The resolved document: owned values, warnings, plus the retained source.

use crate::value::Json5Value;
use strata_json5_parser::{Trivia, TriviaKind, Warning};
use strata_lexer::{SourceRef, SourceStore, Span};

/// A resolved JSON5 document: the owned root value, trailing trivia, the parser's spec-mandated
/// warnings, and the retained source.
///
/// The document keeps the [`SourceStore`] of the parse it was resolved from, so every
/// [`Meta`](crate::Meta) span in the model stays addressable: [`Json5Document::bytes`] borrows
/// raw source text zero-copy, [`Json5Document::comment_text`] extracts a comment's content, and
/// [`Json5Document::source_ref`] hands out promotable handles that survive store reclamation.
/// The warnings recorded during the parse (unescaped U+2028/U+2029 in strings — the parse
/// succeeded regardless) surface through [`Json5Document::warnings`].
#[derive(Debug)]
pub struct Json5Document {
    root: Json5Value,
    trailing: Vec<Trivia>,
    warnings: Vec<Warning>,
    source: SourceStore,
}

impl Json5Document {
    pub(crate) const fn new(
        root: Json5Value,
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

    /// The root value.
    #[inline]
    #[must_use]
    pub const fn root(&self) -> &Json5Value {
        &self.root
    }

    /// Trivia between the end of the root value and the end of input, comments included.
    #[inline]
    #[must_use]
    pub fn trailing_trivia(&self) -> &[Trivia] {
        &self.trailing
    }

    /// The spec-mandated warnings recorded during the parse (see
    /// [`WarningKind`](crate::WarningKind)), in source order. An empty slice means a
    /// warning-free document.
    #[inline]
    #[must_use]
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Borrows the raw source bytes covered by `span`, zero-copy.
    ///
    /// Returns `None` if the span is not (or no longer) backed by the retained source, e.g.
    /// after [`Json5Document::source_mut`] was used to retire buffers.
    #[must_use]
    pub fn bytes(&self, span: Span) -> Option<&[u8]> {
        self.source.bytes(span)
    }

    /// The content of a comment trivia piece, with its delimiters stripped.
    ///
    /// Line comments lose their `//` prefix (their span never includes the terminating line
    /// break); block comments lose both `/*` and `*/`. Interior whitespace is kept verbatim.
    ///
    /// Returns `None` when the piece is not a comment, when its span is no longer backed by the
    /// retained source, or when the bytes are not UTF-8 — the strict JSON5 rules validate
    /// comment encoding, so the latter is only reachable with a replaced rule set (via
    /// `with_rules`) that skips that validation.
    #[must_use]
    pub fn comment_text(&self, comment: &Trivia) -> Option<String> {
        let raw = match self.source.bytes(comment.span()) {
            Some(bytes) => std::borrow::Cow::Borrowed(bytes),
            None => std::borrow::Cow::Owned(self.source.copy_bytes(comment.span())?),
        };
        let content = match comment.kind() {
            TriviaKind::LineComment => raw.strip_prefix(b"//")?,
            TriviaKind::BlockComment => raw.strip_prefix(b"/*")?.strip_suffix(b"*/")?,
            TriviaKind::Whitespace => return None,
        };

        std::str::from_utf8(content).ok().map(str::to_owned)
    }

    /// Creates a promotable, owned-on-demand handle to the bytes covered by `span`.
    pub fn source_ref(&mut self, span: Span) -> Option<SourceRef> {
        self.source.source_ref(span)
    }

    /// The retained source store.
    #[inline]
    #[must_use]
    pub const fn source(&self) -> &SourceStore {
        &self.source
    }

    /// Mutable access to the retained source store, e.g. for owner-side reclamation
    /// (`maintain` / `retire_all`). The owned values stay intact either way; only raw-byte
    /// lookups through [`Json5Document::bytes`] (and thus
    /// [`Json5Document::comment_text`]) stop resolving for reclaimed spans.
    #[inline]
    #[must_use]
    pub const fn source_mut(&mut self) -> &mut SourceStore {
        &mut self.source
    }

    /// Decomposes the document into the root value, trailing trivia, recorded warnings, and
    /// retained source.
    #[must_use]
    pub fn into_parts(self) -> (Json5Value, Vec<Trivia>, Vec<Warning>, SourceStore) {
        (self.root, self.trailing, self.warnings, self.source)
    }
}
