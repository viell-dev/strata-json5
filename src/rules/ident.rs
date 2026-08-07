//! The JSON5 identifier rule: ECMAScript 5.1 `IdentifierName` as unquoted member names.
//!
//! # Unicode approximation
//!
//! ES 5.1 defines identifier characters by Unicode general category — `Lu Ll Lt Lm Lo Nl` for
//! `IdentifierStart` (plus `$`, `_`, and `\uXXXX` escapes) and additionally `Mn Mc Nd Pc`, ZWNJ,
//! and ZWJ for `IdentifierPart`. The Rust standard library does not expose general categories,
//! and this crate adds no external dependencies, so non-ASCII characters are classified with the
//! closest `std` approximation. ASCII and `\uXXXX` escape handling are exact. The precise
//! deviations, relative to ES 5.1:
//!
//! - **Start** uses [`char::is_alphabetic`] (the `Alphabetic` property = `Lu Ll Lt Lm Lo Nl` plus
//!   `Other_Alphabetic`). It accepts everything ES 5.1 accepts, and *over-accepts* the
//!   `Other_Alphabetic` characters (mostly combining marks such as U+0903), which ES 5.1 allows
//!   only in continuation position.
//! - **Continue** uses `is_alphabetic() || is_numeric()` plus the hardcoded `Pc` list and
//!   ZWNJ/ZWJ. [`char::is_numeric`] covers `Nd` and `Nl` but *over-accepts* `No` (e.g. `²`), and
//!   the alphabetic side *under-accepts* combining marks that are not `Other_Alphabetic` (e.g.
//!   U+0301 COMBINING ACUTE ACCENT), so `e` + U+0301 is rejected where ES 5.1 accepts it (the
//!   precomposed `é` is fine).
//!
//! The classifier is structured so an exact category table can drop in later: replace the bodies
//! of [`unicode_start_approximation`] and [`unicode_continue_approximation`].

use crate::rules::{CharAt, Json5Token, peek_char_at};
use strata_lexer::{LexError, Lookahead, Match, Matcher};

/// Unicode connector punctuation (`Pc`), hardcoded from Unicode 16.0 (the category is small and
/// stable): `_`, undertie, character tie, inverted undertie, presentation-form underscores.
const CONNECTOR_PUNCTUATION: [char; 10] = [
    '\u{005F}', '\u{203F}', '\u{2040}', '\u{2054}', '\u{FE33}', '\u{FE34}', '\u{FE4D}', '\u{FE4E}',
    '\u{FE4F}', '\u{FF3F}',
];

/// Returns whether `ch` may begin a JSON5 identifier (ES 5.1 `IdentifierStart`, approximated for
/// non-ASCII characters as described in the [module docs](self)).
pub fn is_identifier_start(ch: char) -> bool {
    match ch {
        'a'..='z' | 'A'..='Z' | '$' | '_' => true,
        _ if ch.is_ascii() => false,
        _ => unicode_start_approximation(ch),
    }
}

/// Returns whether `ch` may continue a JSON5 identifier (ES 5.1 `IdentifierPart`, approximated
/// for non-ASCII characters as described in the [module docs](self)).
pub fn is_identifier_continue(ch: char) -> bool {
    match ch {
        // ASCII, plus ZWNJ (U+200C) and ZWJ (U+200D).
        'a'..='z' | 'A'..='Z' | '0'..='9' | '$' | '_' | '\u{200C}' | '\u{200D}' => true,
        _ if ch.is_ascii() => false,
        _ => unicode_continue_approximation(ch),
    }
}

/// The non-ASCII `IdentifierStart` approximation (see the [module docs](self)); the drop-in
/// point for a future exact `Lu Ll Lt Lm Lo Nl` table.
fn unicode_start_approximation(ch: char) -> bool {
    ch.is_alphabetic()
}

/// The non-ASCII `IdentifierPart` approximation (see the [module docs](self)); the drop-in point
/// for a future exact `Lu Ll Lt Lm Lo Nl Mn Mc Nd Pc` table.
fn unicode_continue_approximation(ch: char) -> bool {
    ch.is_alphabetic() || ch.is_numeric() || CONNECTOR_PUNCTUATION.contains(&ch)
}

/// Matches an ECMAScript 5.1 `IdentifierName` and classifies the exact words `Infinity` and
/// `NaN` into their own kinds (they double as number values; everything else is
/// [`Json5Token::Identifier`]).
///
/// `\uXXXX` escapes are allowed anywhere in the name; the escaped character must be valid in its
/// position, exactly as ES 5.1 requires (so `\u0031` cannot begin an identifier). An invalid
/// escape or character simply ends the match — a backslash or stray character that no rule
/// covers then surfaces as an unrecognized byte at a precise position. The classified words
/// compare *raw source bytes*: an escaped spelling such as `Infinit\u0079` is an ordinary
/// identifier (valid as a member name, but not a number value — the JSON5 grammar requires the
/// literal characters).
///
/// `priority() == -1` lets the reused JSON literal rule win exact-length ties on
/// `true`/`false`/`null`, which are themselves valid `IdentifierName`s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentifierRule;

impl Matcher for IdentifierRule {
    type Kind = Json5Token;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<Json5Token>>, LexError> {
        let mut len: usize = 0;

        loop {
            let start_position = len == 0;
            match measure_identifier_char(look, len, start_position)? {
                Some(width) => len = len.saturating_add(width),
                None => break,
            }
        }

        if len == 0 {
            return Ok(None);
        }

        let kind = match look.peek(len)?.get(..len) {
            Some(b"Infinity") => Json5Token::Infinity,
            Some(b"NaN") => Json5Token::NaN,
            _ => Json5Token::Identifier,
        };

        Ok(Match::from_len(kind, len))
    }

    fn priority(&self) -> i32 {
        -1
    }
}

/// Measures one identifier character (raw or `\uXXXX`-escaped) at `offset`, returning its byte
/// width, or `None` if the identifier ends here.
fn measure_identifier_char(
    look: &mut Lookahead<'_>,
    offset: usize,
    start_position: bool,
) -> Result<Option<usize>, LexError> {
    let valid = |ch: char| {
        if start_position {
            is_identifier_start(ch)
        } else {
            is_identifier_continue(ch)
        }
    };

    match peek_char_at(look, offset)? {
        CharAt::Char { ch: '\\', .. } => match unicode_escape_value(look, offset)? {
            Some(ch) if valid(ch) => Ok(Some(6)),
            _ => Ok(None),
        },
        CharAt::Char { ch, width } if valid(ch) => Ok(Some(width)),
        _ => Ok(None),
    }
}

/// Decodes the `\uXXXX` escape at `offset` (pointing at the backslash), or `None` if the bytes
/// are not a well-formed escape or do not denote a Unicode scalar value.
fn unicode_escape_value(look: &mut Lookahead<'_>, offset: usize) -> Result<Option<char>, LexError> {
    let end = offset.saturating_add(6);
    let bytes = look.peek(end)?;
    let Some(escape) = bytes.get(offset..end) else {
        return Ok(None);
    };

    if escape.get(1) != Some(&b'u') {
        return Ok(None);
    }

    let mut value: u32 = 0;

    for &byte in escape.get(2..6).unwrap_or_default() {
        let Some(digit) = char::from(byte).to_digit(16) else {
            return Ok(None);
        };
        value = value.saturating_mul(16).saturating_add(digit);
    }

    Ok(char::from_u32(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::{lex, lex_error};
    use strata_lexer::{LexError as Error, LexErrorKind};

    fn ident_span(input: &str) -> Result<(Json5Token, u64, u64), Error> {
        let tokens = lex(input)?;
        tokens
            .first()
            .copied()
            .ok_or_else(|| Error::unrecognized(strata_lexer::Position::START))
    }

    #[test]
    fn ascii_identifiers_lex() -> Result<(), Error> {
        for input in ["abc", "$", "_", "$loc", "_x9", "A1_b$"] {
            let (kind, start, end) = ident_span(input)?;
            assert_eq!(kind, Json5Token::Identifier, "input {input:?}");
            assert_eq!(start, 0);
            assert_eq!(end, u64::try_from(input.len()).unwrap_or(u64::MAX));
        }

        Ok(())
    }

    #[test]
    fn digits_cannot_start_an_identifier() -> Result<(), Error> {
        // `9a` — the number rule matches `9`, then `a` is a separate identifier.
        let tokens = lex("9a")?;
        assert_eq!(
            tokens,
            vec![
                (Json5Token::Number, 0, 1),
                (Json5Token::Identifier, 1, 2),
                (Json5Token::Eof, 2, 2),
            ]
        );

        Ok(())
    }

    #[test]
    fn unicode_letters_lex() -> Result<(), Error> {
        for input in ["\u{00E9}t\u{00E9}", "\u{4E2D}\u{6587}", "\u{03B1}\u{03B2}"] {
            let (kind, _, end) = ident_span(input)?;
            assert_eq!(kind, Json5Token::Identifier, "input {input:?}");
            assert_eq!(end, u64::try_from(input.len()).unwrap_or(u64::MAX));
        }

        Ok(())
    }

    #[test]
    fn zwnj_and_zwj_continue_but_do_not_start() {
        let (kind, _, end) = ident_span("a\u{200C}b").unwrap_or((Json5Token::Eof, 0, 0));
        assert_eq!(kind, Json5Token::Identifier);
        assert_eq!(end, 5);

        let error = lex_error("\u{200C}");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
    }

    #[test]
    fn escapes_are_validated_per_position() -> Result<(), Error> {
        // \u0061 = 'a' — valid anywhere.
        assert_eq!(ident_span(r"\u0061bc")?, (Json5Token::Identifier, 0, 8));
        assert_eq!(ident_span(r"ab\u0063")?, (Json5Token::Identifier, 0, 8));
        // \u0031 = '1' — continuation only.
        assert_eq!(ident_span(r"a\u0031")?, (Json5Token::Identifier, 0, 7));
        let error = lex_error(r"\u0031ab");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 0);

        Ok(())
    }

    #[test]
    fn malformed_escapes_end_the_identifier() {
        // `ab\uZZZZ` — the identifier ends at `ab`; the backslash matches no rule.
        let error = lex_error(r"ab\uZZZZ");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 2);

        // A lone-surrogate escape does not denote a scalar value.
        let error = lex_error(r"a\uD800");
        assert_eq!(error.position.byte(), 1);
    }

    #[test]
    fn infinity_and_nan_words_classify() -> Result<(), Error> {
        assert_eq!(ident_span("Infinity")?, (Json5Token::Infinity, 0, 8));
        assert_eq!(ident_span("NaN")?, (Json5Token::NaN, 0, 3));
        // Longer words and escaped spellings are plain identifiers.
        assert_eq!(ident_span("Infinity2")?.0, Json5Token::Identifier);
        assert_eq!(ident_span(r"Infinit\u0079")?.0, Json5Token::Identifier);
        assert_eq!(ident_span("NaNa")?.0, Json5Token::Identifier);

        Ok(())
    }

    #[test]
    fn known_start_approximation_over_accepts_other_alphabetic() {
        // U+0903 DEVANAGARI SIGN VISARGA is Mc + Other_Alphabetic: ES 5.1 rejects it as a start
        // character, this approximation accepts it. Documented deviation.
        assert!(is_identifier_start('\u{0903}'));
    }

    #[test]
    fn known_continue_approximation_deviations() {
        // U+00B2 SUPERSCRIPT TWO is No: ES 5.1 rejects, `is_numeric` accepts. Documented.
        assert!(is_identifier_continue('\u{00B2}'));
        // U+0301 COMBINING ACUTE ACCENT is Mn but not Other_Alphabetic: ES 5.1 accepts, this
        // approximation rejects. Documented.
        assert!(!is_identifier_continue('\u{0301}'));
    }

    #[test]
    fn connector_punctuation_continues() -> Result<(), Error> {
        assert_eq!(ident_span("a\u{203F}b")?.0, Json5Token::Identifier);

        Ok(())
    }
}
