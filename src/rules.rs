//! JSON5 token rules: composable matchers for the JSON5 lexical grammar.
//!
//! Each rule is a zero-sized [`Matcher`](strata_lexer::Matcher) recognizing one JSON5 token shape
//! from read-only lookahead, following the same measure-then-advance model as `strata-json`'s
//! rules. The set is composed on top of `strata-json` per its documented superset pattern:
//!
//! - **Reused from JSON, lifted with [`MapMatcher`]:** [`PunctRule`] (the six structural
//!   characters are identical in JSON5) and [`LiteralRule`] (`true`/`false`/`null` are spelled
//!   exactly as in JSON). Their [`JsonToken`] kinds are wrapped as [`Json5Token::Json`].
//! - **Replaced with JSON5 shapes:** whitespace (JSON5's set is a strict superset of JSON's four
//!   bytes), strings (single or double quotes, more escapes, line continuations), and numbers
//!   (signs, hex, `Infinity`/`NaN`, leading/trailing decimal points). Where the JSON rule's shape
//!   is a strict subset of the JSON5 shape, one JSON5 rule is preferred over layering the two.
//! - **Added:** identifiers ([`IdentifierRule`]) and comments ([`LineCommentRule`],
//!   [`BlockCommentRule`]).
//! - **Dropped:** `BomRule` — U+FEFF is ordinary JSON5 whitespace ([spec] Table 3), anywhere in
//!   the input, so [`WhitespaceRule`] covers it and no BOM policy option exists.
//!
//! `true`, `false`, and `null` are also valid ECMAScript 5.1 `IdentifierName`s, so on those exact
//! words the lifted [`LiteralRule`] and [`IdentifierRule`] both match with equal length.
//! [`IdentifierRule`] declares `priority() == -1` so the literal deterministically wins the
//! maximal-munch tie; any longer word (`truex`) still lexes as an identifier.
//!
//! [spec]: https://spec.json5.org/
//! [`MapMatcher`]: strata_lexer::MapMatcher

use strata_lexer::{
    Candidate, LexError, Lookahead, MapMatcher, Match, Matcher, MatcherExt, MatcherSet, Position,
    SpecScanner,
};

pub mod comment;
pub mod ident;
pub mod number;
pub mod string;

pub use comment::{BlockCommentRule, LineCommentRule};
pub use ident::IdentifierRule;
pub use number::NumberRule;
pub use string::StringRule;

// Re-exported so composing on this crate does not require a direct `strata-json` dependency.
pub use strata_json::{JsonToken, LiteralRule, PunctRule, SharedRuleSet};

/// Token kinds produced by the JSON5 token rules.
///
/// Kinds reused from `strata-json` are wrapped verbatim: with the strict rule set, the reachable
/// [`Json5Token::Json`] values are the six structural characters and the `true`/`false`/`null`
/// literals (the JSON whitespace, string, number, BOM, and EOF kinds are replaced by JSON5-owned
/// kinds and never produced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Json5Token {
    /// A token produced by a reused `strata-json` rule: `{` `}` `[` `]` `:` `,` and the
    /// `true`/`false`/`null` literals.
    Json(JsonToken),
    /// An ECMAScript 5.1 `IdentifierName` (other than the classified words below), usable as an
    /// unquoted object member name.
    Identifier,
    /// The word `Infinity`, spelled exactly and without escapes: a number value, and — being an
    /// `IdentifierName` — also a valid unquoted member name.
    Infinity,
    /// The word `NaN`, spelled exactly and without escapes: a number value, and — being an
    /// `IdentifierName` — also a valid unquoted member name.
    NaN,
    /// A complete single- or double-quoted string, span covering both quotes.
    String,
    /// A complete number: decimal (leading/trailing decimal point allowed), hexadecimal, or a
    /// signed `Infinity`/`NaN`, with an optional leading `+` or `-`.
    Number,
    /// A run of JSON5 whitespace, including line terminators and the byte-order mark.
    Whitespace,
    /// A `//` comment, excluding the terminating line terminator.
    LineComment,
    /// A `/* ... */` comment, including both delimiters.
    BlockComment,
    /// End of input.
    Eof,
}

impl Json5Token {
    /// Returns whether this kind is trivia — grammatically insignificant source content that the
    /// CST retains instead of discarding.
    pub const fn is_trivia(self) -> bool {
        match self {
            Self::Whitespace | Self::LineComment | Self::BlockComment => true,
            Self::Json(token) => token.is_trivia(),
            _ => false,
        }
    }

    /// A short human-readable description for diagnostics.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Json(token) => token.describe(),
            Self::Identifier => "an identifier",
            Self::Infinity => "'Infinity'",
            Self::NaN => "'NaN'",
            Self::String => "a string",
            Self::Number => "a number",
            Self::Whitespace => "whitespace",
            Self::LineComment => "a line comment",
            Self::BlockComment => "a block comment",
            Self::Eof => "end of input",
        }
    }
}

/// The kind-lifting function used for the reused `strata-json` rules.
type WrapJson = fn(JsonToken) -> Json5Token;

/// The strict JSON5 rule set: named, fully monomorphic fields instead of a matcher tuple (the
/// lexer's tuple `MatcherSet` impls stop at six elements, and this set has eight rules).
///
/// The reused JSON rules keep their validation; the JSON5-owned rules carry the shapes that
/// differ. Composing a superset of JSON5 works exactly like composing JSON5 over JSON: lift the
/// individual rule structs with [`MatcherExt::map_kind`] into the wider kind and add or replace
/// only what differs.
#[derive(Debug, Clone, Copy)]
pub struct StrictRuleSet {
    whitespace: WhitespaceRule,
    punct: MapMatcher<PunctRule, WrapJson>,
    literal: MapMatcher<LiteralRule, WrapJson>,
    identifier: IdentifierRule,
    string: StringRule,
    number: NumberRule,
    line_comment: LineCommentRule,
    block_comment: BlockCommentRule,
}

/// Returns the strict JSON5 rule set.
pub fn strict_rule_set() -> StrictRuleSet {
    let wrap: WrapJson = Json5Token::Json;

    StrictRuleSet {
        whitespace: WhitespaceRule,
        punct: PunctRule.map_kind(wrap),
        literal: LiteralRule.map_kind(wrap),
        identifier: IdentifierRule,
        string: StringRule,
        number: NumberRule,
        line_comment: LineCommentRule,
        block_comment: BlockCommentRule,
    }
}

/// Returns a [`SpecScanner`] over the strict rule set with [`Json5Token::Eof`] as its
/// end-of-input token.
pub fn strict_scanner() -> SpecScanner<StrictRuleSet, Json5Token> {
    SpecScanner::new(strict_rule_set(), Json5Token::Eof)
}

impl MatcherSet for StrictRuleSet {
    type Kind = Json5Token;

    fn measure(&self, look: &mut Lookahead<'_>) -> Result<Vec<Candidate<Self::Kind>>, LexError> {
        let mut candidates = Vec::new();

        measure_into(&self.whitespace, look, &mut candidates)?;
        measure_into(&self.punct, look, &mut candidates)?;
        measure_into(&self.literal, look, &mut candidates)?;
        measure_into(&self.identifier, look, &mut candidates)?;
        measure_into(&self.string, look, &mut candidates)?;
        measure_into(&self.number, look, &mut candidates)?;
        measure_into(&self.line_comment, look, &mut candidates)?;
        measure_into(&self.block_comment, look, &mut candidates)?;

        Ok(candidates)
    }
}

/// Measures one rule and records its candidate, mirroring the lexer's tuple `MatcherSet` impls.
fn measure_into<M: Matcher<Kind = Json5Token>>(
    rule: &M,
    look: &mut Lookahead<'_>,
    candidates: &mut Vec<Candidate<Json5Token>>,
) -> Result<(), LexError> {
    if let Some(found) = rule.try_match(look)? {
        candidates.push(Candidate::new(found, rule.priority()));
    }

    Ok(())
}

/// Returns whether `ch` is JSON5 whitespace ([spec] Table 3).
///
/// The set is: tab, line feed, vertical tab, form feed, carriage return, space, no-break space,
/// line separator, paragraph separator, the byte-order mark, and every character in the Unicode
/// `Zs` (Space Separator) category. The `Zs` members not already listed are hardcoded from
/// Unicode 16.0: U+1680, U+2000–U+200A, U+202F, U+205F, U+3000 (`Zs` has been stable for many
/// versions; U+0020 and U+00A0 are also `Zs` and appear in the explicit list).
///
/// [spec]: https://spec.json5.org/
pub const fn is_json5_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{FEFF}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// Matches a run of JSON5 whitespace: the [`is_json5_whitespace`] set, decoded from UTF-8.
///
/// Line terminators are included in the run — JSON5 treats them as inter-token whitespace — and
/// so is U+FEFF anywhere in the input: unlike JSON, JSON5 classifies the byte-order mark as
/// ordinary whitespace, so there is no BOM policy and no BOM rule. A byte sequence that is not
/// valid UTF-8 simply ends the run (and, matching no other rule, surfaces as an unrecognized
/// byte).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WhitespaceRule;

impl Matcher for WhitespaceRule {
    type Kind = Json5Token;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<Json5Token>>, LexError> {
        let mut len: usize = 0;

        loop {
            match peek_char_at(look, len)? {
                CharAt::Char { ch, width } if is_json5_whitespace(ch) => {
                    len = len.saturating_add(width);
                }
                _ => break,
            }
        }

        Ok(Match::from_len(Json5Token::Whitespace, len))
    }
}

/// The decoded character at a lookahead offset.
pub(crate) enum CharAt {
    /// A valid UTF-8 character and its encoded width in bytes.
    Char {
        /// The decoded character.
        ch: char,
        /// The number of bytes its UTF-8 encoding occupies.
        width: usize,
    },
    /// End of input.
    End,
    /// The bytes at the offset are not valid UTF-8 (including truncation at end of input).
    Invalid,
}

/// Decodes the UTF-8 character starting `offset` bytes past the cursor, filling on demand.
pub(crate) fn peek_char_at(look: &mut Lookahead<'_>, offset: usize) -> Result<CharAt, LexError> {
    let bytes = look.peek(offset.saturating_add(4))?;
    let Some(&first) = bytes.get(offset) else {
        return Ok(CharAt::End);
    };

    let width: usize = match first {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return Ok(CharAt::Invalid),
    };

    let Some(slice) = bytes.get(offset..offset.saturating_add(width)) else {
        return Ok(CharAt::Invalid);
    };

    match core::str::from_utf8(slice) {
        Ok(text) => match text.chars().next() {
            Some(ch) => Ok(CharAt::Char { ch, width }),
            None => Ok(CharAt::Invalid),
        },
        Err(_) => Ok(CharAt::Invalid),
    }
}

/// Returns `start` advanced by `offset` bytes on the same line.
///
/// Only valid when the bytes before `offset` contain no line breaks; the number rule qualifies
/// (numbers are ASCII digits, signs, dots, and letters). Rules whose tokens can span lines use
/// [`position_at`] instead.
pub(crate) fn offset_position(start: Position, offset: usize) -> Position {
    start.advanced_by(u64::try_from(offset).unwrap_or(u64::MAX))
}

/// Advances `position` over `bytes`, mirroring the lexer core's tracking exactly: the column
/// advances one per byte, `\r`, `\n`, and `\r\n` each count as one line break, and U+2028/U+2029
/// do **not** advance the line (the core is byte-oriented; line/column are diagnostics).
pub(crate) fn advance_position(position: Position, bytes: &[u8]) -> Position {
    let mut byte = position.byte();
    let mut line = position.line();
    let mut column = position.column();
    let mut previous_was_cr = false;

    for &b in bytes {
        byte = byte.saturating_add(1);

        match b {
            b'\r' => {
                line = line.saturating_add(1);
                column = 1;
                previous_was_cr = true;
            }
            b'\n' if previous_was_cr => {
                previous_was_cr = false;
            }
            b'\n' => {
                line = line.saturating_add(1);
                column = 1;
                previous_was_cr = false;
            }
            _ => {
                column = column.saturating_add(1);
                previous_was_cr = false;
            }
        }
    }

    // Line and column stay one-based above (they only grow or reset to 1), so construction
    // cannot fail; the fallback is unreachable.
    Position::new(byte, line, column).unwrap_or(position)
}

/// Computes the position `offset` bytes past `start`, line-aware, for in-token error reporting.
///
/// JSON5 strings and block comments can contain line breaks (continuations, multi-line
/// comments), so a same-line offset shortcut would report wrong lines; this walks the actual
/// bytes. Errors are reported on already-peeked spans, so the peek cannot fail in practice; if
/// it does, the byte offset is still advanced.
pub(crate) fn position_at(look: &mut Lookahead<'_>, start: Position, offset: usize) -> Position {
    match look.peek(offset) {
        Ok(bytes) => {
            let walked = bytes.get(..offset).unwrap_or(bytes);
            advance_position(start, walked)
        }
        Err(_) => start.advanced_by(u64::try_from(offset).unwrap_or(u64::MAX)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;
    use strata_lexer::{LexErrorKind, Lexer};

    pub(crate) fn lex_bytes(input: &[u8]) -> Result<Vec<(Json5Token, u64, u64)>, LexError> {
        let mut lexer = Lexer::new(Cursor::new(input.to_vec()), strict_scanner());
        let mut out = Vec::new();

        loop {
            let token = lexer.next_token()?;
            let done = token.kind == Json5Token::Eof;
            out.push((token.kind, token.span().start(), token.span().end()));

            if done {
                break;
            }
        }

        Ok(out)
    }

    pub(crate) fn lex(input: &str) -> Result<Vec<(Json5Token, u64, u64)>, LexError> {
        lex_bytes(input.as_bytes())
    }

    pub(crate) fn lex_error(input: &str) -> LexError {
        match lex(input) {
            Err(error) => error,
            Ok(tokens) => panic!("expected a lexical error, got {tokens:?}"),
        }
    }

    #[test]
    fn punctuation_lexes_through_the_reused_json_rule() -> Result<(), LexError> {
        assert_eq!(
            lex("{}[]:,")?,
            vec![
                (Json5Token::Json(JsonToken::LeftBrace), 0, 1),
                (Json5Token::Json(JsonToken::RightBrace), 1, 2),
                (Json5Token::Json(JsonToken::LeftBracket), 2, 3),
                (Json5Token::Json(JsonToken::RightBracket), 3, 4),
                (Json5Token::Json(JsonToken::Colon), 4, 5),
                (Json5Token::Json(JsonToken::Comma), 5, 6),
                (Json5Token::Eof, 6, 6),
            ]
        );

        Ok(())
    }

    #[test]
    fn literals_win_the_tie_against_the_identifier_rule() -> Result<(), LexError> {
        assert_eq!(
            lex("true")?,
            vec![
                (Json5Token::Json(JsonToken::True), 0, 4),
                (Json5Token::Eof, 4, 4),
            ]
        );
        assert_eq!(
            lex("null")?,
            vec![
                (Json5Token::Json(JsonToken::Null), 0, 4),
                (Json5Token::Eof, 4, 4),
            ]
        );

        Ok(())
    }

    #[test]
    fn longer_words_lex_as_identifiers_by_maximal_munch() -> Result<(), LexError> {
        assert_eq!(
            lex("truex")?,
            vec![(Json5Token::Identifier, 0, 5), (Json5Token::Eof, 5, 5)]
        );
        assert_eq!(
            lex("nullify")?,
            vec![(Json5Token::Identifier, 0, 7), (Json5Token::Eof, 7, 7)]
        );

        Ok(())
    }

    #[test]
    fn whitespace_run_covers_the_expanded_set() -> Result<(), LexError> {
        // Tab, LF, VT, FF, CR, space, NBSP, LS, PS, BOM, ogham space, en quad, thin space,
        // narrow NBSP, math space, ideographic space.
        let input = "\t\n\u{0B}\u{0C}\r \u{A0}\u{2028}\u{2029}\u{FEFF}\u{1680}\u{2000}\u{2009}\u{202F}\u{205F}\u{3000}";
        let end = u64::try_from(input.len()).unwrap_or(u64::MAX);
        assert_eq!(
            lex(input)?,
            vec![
                (Json5Token::Whitespace, 0, end),
                (Json5Token::Eof, end, end)
            ]
        );

        Ok(())
    }

    #[test]
    fn bom_is_whitespace_anywhere() -> Result<(), LexError> {
        assert_eq!(
            lex("\u{FEFF}1\u{FEFF}")?,
            vec![
                (Json5Token::Whitespace, 0, 3),
                (Json5Token::Number, 3, 4),
                (Json5Token::Whitespace, 4, 7),
                (Json5Token::Eof, 7, 7),
            ]
        );

        Ok(())
    }

    #[test]
    fn non_whitespace_unicode_is_not_whitespace() {
        // U+200B zero width space is Cf, not Zs, and not in Table 3.
        let error = lex_error("\u{200B}");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 0);
    }

    #[test]
    fn advance_position_mirrors_the_lexer_core() {
        let start = Position::START;
        let end = advance_position(start, b"a\r\nb");
        assert_eq!(end.byte(), 4);
        assert_eq!(end.line(), 2);
        assert_eq!(end.column(), 2);

        let end = advance_position(start, b"\r\r");
        assert_eq!(end.line(), 3);
        assert_eq!(end.column(), 1);
    }
}
