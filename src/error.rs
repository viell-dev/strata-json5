//! Resolver error types.

use std::error;
use std::fmt;
use strata_lexer::{Position, Span};

/// A resolve failure, carrying the position at which it was observed.
#[derive(Debug)]
pub struct Json5Error {
    /// Position of the offending byte or token start.
    pub position: Position,
    /// What went wrong.
    pub kind: Json5ErrorKind,
}

/// Categories of resolve failures.
#[derive(Debug)]
pub enum Json5ErrorKind {
    /// Parsing failed before resolution began.
    Parse(strata_json5_parser::Json5Error),
    /// A string contained a UTF-16 surrogate escape without its pair, under
    /// [`LoneSurrogates::Error`](strata_json5_parser::decode::LoneSurrogates::Error).
    LoneSurrogate {
        /// The unpaired code unit.
        unit: u16,
    },
    /// An object repeated a member name, under
    /// [`DuplicateKeys::Error`](crate::DuplicateKeys::Error).
    ///
    /// The position is the start of the repeated (later) member name. Names are compared
    /// decoded, so different spellings of one name — unquoted, quoted either way, or
    /// escape-carrying — all collide.
    DuplicateMember {
        /// The decoded member name.
        name: String,
    },
    /// A scalar or member-name lexeme failed to decode.
    ///
    /// Tokens produced by the strict JSON5 rules always decode; this is only reachable when a
    /// replaced rule set (via `with_rules`) admits token shapes the JSON5 grammar does not.
    InvalidLexeme {
        /// The decode failure, with its offset inside the lexeme.
        error: strata_json5_parser::decode::Error,
    },
    /// A span of the document was no longer backed by its retained source.
    ///
    /// This only happens when a caller reclaims the source store of a parsed document (e.g. via
    /// `source_mut().retire_all()`) before handing it to
    /// [`Json5Resolver::resolve_document`](crate::Json5Resolver::resolve_document).
    SourceUnavailable {
        /// The unavailable byte range.
        span: Span,
    },
}

impl fmt::Display for Json5Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            // The parse error's message already carries its position.
            Json5ErrorKind::Parse(error) => write!(formatter, "{error}"),
            Json5ErrorKind::LoneSurrogate { unit } => write!(
                formatter,
                "lone surrogate escape \\u{unit:04X} at line {}, column {} (byte {})",
                self.position.line(),
                self.position.column(),
                self.position.byte()
            ),
            Json5ErrorKind::DuplicateMember { name } => write!(
                formatter,
                "duplicate object member {name:?} at line {}, column {} (byte {})",
                self.position.line(),
                self.position.column(),
                self.position.byte()
            ),
            Json5ErrorKind::InvalidLexeme { error } => write!(
                formatter,
                "{error} (token at line {}, column {}, byte {})",
                self.position.line(),
                self.position.column(),
                self.position.byte()
            ),
            Json5ErrorKind::SourceUnavailable { span } => write!(
                formatter,
                "source bytes {}..{} are no longer retained",
                span.start(),
                span.end()
            ),
        }
    }
}

impl error::Error for Json5Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            Json5ErrorKind::Parse(error) => Some(error),
            _ => None,
        }
    }
}

impl From<strata_json5_parser::Json5Error> for Json5Error {
    fn from(error: strata_json5_parser::Json5Error) -> Self {
        Self {
            position: error.position,
            kind: Json5ErrorKind::Parse(error),
        }
    }
}
