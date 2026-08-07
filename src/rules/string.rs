//! The JSON5 string rule: single- or double-quoted, with the ECMAScript 5.1 escape repertoire
//! and line continuations.
//!
//! The JSON5 string shape is a strict superset of the JSON one (every RFC 8259 string is a valid
//! JSON5 string), so per the ecosystem's composition guidance this crate carries one JSON5 rule
//! instead of layering `strata-json`'s `StringRule` under an extension rule.

use crate::rules::{CharAt, Json5Token, peek_char_at, position_at};
use strata_lexer::{LexError, Lookahead, Match, Matcher};

/// Matches a JSON5 string token, validating lexical shape.
///
/// The measured span covers both quotes. Compared with JSON ([spec] §Strings, ES 5.1 §7.8.4):
///
/// - Strings open and close with `'` or `"`; the other quote kind is ordinary content.
/// - Escapes: the single-escape characters `' " \ b f n r t v`, `\0` (not followed by a decimal
///   digit), `\xHH`, `\uHHHH`, and any other non-digit source character after the backslash
///   (a `NonEscapeCharacter`, included verbatim — this covers JSON's `\/`).
/// - A backslash before a line terminator sequence (LF, CR, CRLF, U+2028, U+2029) is a line
///   continuation; the pair stays in the lexeme (nothing is decoded here) but is grammatically an
///   escape.
/// - Unescaped LF and CR are lexical errors; unescaped U+2028/U+2029 are *legal* string content
///   (the parser records the spec-mandated warning). Raw control characters (tab, NUL, ...) are
///   legal content too — ES 5.1 string literals only exclude the quote, the backslash, and line
///   terminators.
///
/// Shape violations are lexical errors at the offending byte:
/// [`LexErrorKind::Unrecognized`](strata_lexer::LexErrorKind::Unrecognized) for a bad escape
/// (`\` + decimal digit 1–9, `\0` + digit, malformed `\xHH`/`\uHHHH` — reported at the
/// backslash), an unescaped line terminator, or invalid UTF-8. An unclosed string is
/// [`LexErrorKind::UnterminatedDelimiter`](strata_lexer::LexErrorKind::UnterminatedDelimiter)
/// at the opening quote. Escape *decoding* stays downstream at the resolver boundary; lone
/// surrogate `\uHHHH` escapes are lexically valid, as in `strata-json`.
///
/// [spec]: https://spec.json5.org/
#[expect(
    clippy::module_name_repetitions,
    reason = "rule struct names mirror strata-json's public naming convention"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StringRule;

impl Matcher for StringRule {
    type Kind = Json5Token;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<Json5Token>>, LexError> {
        let Some(quote @ (b'"' | b'\'')) = look.peek_byte()? else {
            return Ok(None);
        };

        let start = look.position();
        let mut index: usize = 1; // past the opening quote

        loop {
            match peek_char_at(look, index)? {
                CharAt::End => return Err(LexError::unterminated_delimiter(start)),
                CharAt::Invalid => {
                    return Err(LexError::unrecognized(position_at(look, start, index)));
                }
                CharAt::Char { ch, width } => match ch {
                    _ if ch == char::from(quote) => {
                        return Ok(Match::from_len(Json5Token::String, index.saturating_add(1)));
                    }
                    '\\' => index = measure_escape(look, start, index)?,
                    '\n' | '\r' => {
                        return Err(LexError::unrecognized(position_at(look, start, index)));
                    }
                    _ => index = index.saturating_add(width),
                },
            }
        }
    }
}

/// Measures the escape whose backslash sits at `index`, returning the index just past it.
///
/// Handles line continuations (including the two-byte CRLF sequence), `\0` with its
/// no-following-digit restriction, `\xHH`, `\uHHHH`, and the single-escape / non-escape
/// character productions (both consume exactly one source character).
fn measure_escape(
    look: &mut Lookahead<'_>,
    start: strata_lexer::Position,
    index: usize,
) -> Result<usize, LexError> {
    let after_backslash = index.saturating_add(1);

    match peek_char_at(look, after_backslash)? {
        CharAt::End => Err(LexError::unterminated_delimiter(start)),
        CharAt::Invalid => Err(LexError::unrecognized(position_at(
            look,
            start,
            after_backslash,
        ))),
        CharAt::Char { ch, width } => match ch {
            // LineContinuation: \ LineTerminatorSequence. CR followed by LF is one sequence.
            '\r' => {
                let after_cr = after_backslash.saturating_add(1);
                let bytes = look.peek(after_cr.saturating_add(1))?;

                if bytes.get(after_cr) == Some(&b'\n') {
                    Ok(after_cr.saturating_add(1))
                } else {
                    Ok(after_cr)
                }
            }
            // LineContinuation over a single-character terminator sequence. Measures the same as
            // the non-escape arm below, but it is a distinct grammar production.
            #[expect(
                clippy::match_same_arms,
                reason = "line continuation is a distinct production that happens to measure \
                          like a non-escape character"
            )]
            '\n' | '\u{2028}' | '\u{2029}' => Ok(after_backslash.saturating_add(width)),
            // \0 — the ES 5.1 EscapeSequence production `0 [lookahead not DecimalDigit]`.
            '0' => {
                let after_zero = after_backslash.saturating_add(1);
                let bytes = look.peek(after_zero.saturating_add(1))?;

                if matches!(bytes.get(after_zero), Some(b'0'..=b'9')) {
                    Err(LexError::unrecognized(position_at(look, start, index)))
                } else {
                    Ok(after_zero)
                }
            }
            // Decimal digits 1-9 are EscapeCharacters with no production: always invalid.
            '1'..='9' => Err(LexError::unrecognized(position_at(look, start, index))),
            'x' => measure_hex_escape(look, start, index, 2),
            'u' => measure_hex_escape(look, start, index, 4),
            // SingleEscapeCharacter or NonEscapeCharacter: one source character either way.
            _ => Ok(after_backslash.saturating_add(width)),
        },
    }
}

/// Measures a `\xHH` or `\uHHHH` escape (backslash at `index`, `digits` hex digits expected),
/// returning the index just past it.
fn measure_hex_escape(
    look: &mut Lookahead<'_>,
    start: strata_lexer::Position,
    index: usize,
    digits: usize,
) -> Result<usize, LexError> {
    let digits_start = index.saturating_add(2);
    let digits_end = digits_start.saturating_add(digits);
    let bytes = look.peek(digits_end)?;
    let valid = bytes
        .get(digits_start..digits_end)
        .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit));

    if valid {
        Ok(digits_end)
    } else {
        Err(LexError::unrecognized(position_at(look, start, index)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::{lex, lex_bytes, lex_error};
    use strata_lexer::LexErrorKind;

    fn assert_whole_string(input: &str) {
        let end = u64::try_from(input.len()).unwrap_or(u64::MAX);
        match lex(input) {
            Ok(tokens) => assert_eq!(
                tokens,
                vec![(Json5Token::String, 0, end), (Json5Token::Eof, end, end)],
                "input {input:?}"
            ),
            Err(error) => panic!("expected a string token for {input:?}, got {error}"),
        }
    }

    #[test]
    fn double_and_single_quoted_strings_lex() {
        assert_whole_string("\"hello\"");
        assert_whole_string("'hello'");
        assert_whole_string("''");
        assert_whole_string("\"\"");
        // The other quote kind is plain content.
        assert_whole_string("'say \"hi\"'");
        assert_whole_string("\"it's\"");
    }

    #[test]
    fn json_escapes_still_lex() {
        assert_whole_string("\"\\\"\\\\\\/\\b\\f\\n\\r\\t\\u0041\"");
    }

    #[test]
    fn json5_escapes_lex() {
        assert_whole_string("'\\''"); // escaped single quote
        assert_whole_string("'\\v'");
        assert_whole_string("'\\0'");
        assert_whole_string("'\\x41'");
        assert_whole_string("'\\uD83D\\uDE00'"); // surrogate pair, decoded downstream
        assert_whole_string("'\\q'"); // NonEscapeCharacter
        assert_whole_string("'\\\u{20AC}'"); // multibyte NonEscapeCharacter
    }

    #[test]
    fn line_continuations_lex_for_every_terminator() {
        assert_whole_string("'a\\\nb'");
        assert_whole_string("'a\\\rb'");
        assert_whole_string("'a\\\r\nb'");
        assert_whole_string("'a\\\u{2028}b'");
        assert_whole_string("'a\\\u{2029}b'");
    }

    #[test]
    fn crlf_continuation_is_one_sequence() {
        // If \<CR> ended the continuation, the LF would be an unescaped line terminator.
        assert_whole_string("'\\\r\n'");
    }

    #[test]
    fn raw_control_characters_are_legal_content() {
        assert_whole_string("'a\tb'");
        assert_whole_string("'a\u{0}b'");
        assert_whole_string("'a\u{7F}b'");
    }

    #[test]
    fn unescaped_line_and_paragraph_separators_are_legal() {
        assert_whole_string("'a\u{2028}b'");
        assert_whole_string("'a\u{2029}b'");
    }

    #[test]
    fn unescaped_line_terminators_are_rejected() {
        for (input, byte) in [("'a\nb'", 2), ("'a\rb'", 2)] {
            let error = lex_error(input);
            assert!(
                matches!(error.kind, LexErrorKind::Unrecognized),
                "input {input:?}"
            );
            assert_eq!(error.position.byte(), byte, "input {input:?}");
        }
    }

    #[test]
    fn digit_escapes_are_rejected_at_the_backslash() {
        let error = lex_error("'\\1'");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 1);

        let error = lex_error("'\\09'");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 1);
    }

    #[test]
    fn malformed_hex_escapes_are_rejected_at_the_backslash() {
        for input in ["'\\x4'", "'\\xg1'", "'\\u12'", "'\\u12G4'"] {
            let error = lex_error(input);
            assert!(
                matches!(error.kind, LexErrorKind::Unrecognized),
                "input {input:?}"
            );
            assert_eq!(error.position.byte(), 1, "input {input:?}");
        }
    }

    #[test]
    fn unterminated_strings_are_rejected_at_the_opening_quote() {
        for input in ["'open", "\"open", "'trailing\\"] {
            let error = lex_error(input);
            assert!(
                matches!(error.kind, LexErrorKind::UnterminatedDelimiter),
                "input {input:?}"
            );
            assert_eq!(error.position.byte(), 0, "input {input:?}");
        }
    }

    #[test]
    fn mismatched_quotes_do_not_close() {
        let error = lex_error("'ab\"");
        assert!(matches!(error.kind, LexErrorKind::UnterminatedDelimiter));
    }

    #[test]
    fn invalid_utf8_is_rejected_at_its_byte() {
        let Err(error) = lex_bytes(b"'a\xFFb'") else {
            panic!("expected a lexical error");
        };
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 2);
    }

    #[test]
    fn error_positions_after_continuations_are_line_aware() {
        // Bytes: 0 `'`, 1 `\`, 2 LF, 3 `a`, 4 LF — the unescaped LF at byte 4 is on line 2.
        let error = lex_error("'\\\na\nb'");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 4);
        assert_eq!(error.position.line(), 2);
        assert_eq!(error.position.column(), 2);
    }
}
