//! The JSON5 number rule: decimal with optional leading/trailing decimal point, hexadecimal
//! integers, and signed `Infinity`/`NaN`.
//!
//! The JSON number grammar is a strict subset of this shape, so per the ecosystem's composition
//! guidance this crate carries one JSON5 rule instead of layering `strata-json`'s `NumberRule`
//! under extension rules.

use crate::rules::{Json5Token, offset_position};
use strata_lexer::{LexError, Lookahead, Match, Matcher};

/// Matches a JSON5 number token per the [spec] grammar:
///
/// ```text
/// JSON5Number         = [ "+" / "-" ] JSON5NumericLiteral
/// JSON5NumericLiteral = NumericLiteral / "Infinity" / "NaN"
/// NumericLiteral      = DecimalLiteral / HexIntegerLiteral            ; ES 5.1 §7.8.3
/// DecimalLiteral      = int "." [digits] [exp] / "." digits [exp] / int [exp]
/// HexIntegerLiteral   = ( "0x" / "0X" ) 1*HEXDIG
/// ```
///
/// Compared with JSON: an explicit leading `+` is allowed, the integer part may be empty when a
/// fraction follows (`.5`), the fraction may be empty (`1.`), hex integers exist, and the words
/// `Infinity`/`NaN` are numbers. *Bare* `Infinity`/`NaN` (no sign) are left to the identifier
/// rule, which classifies them as [`Json5Token::Infinity`]/[`Json5Token::NaN`]: they are also
/// valid member names, and lexing them as `Number` would force the parser to accept arbitrary
/// numbers in name position. With an explicit sign they are unambiguous and match here.
///
/// Still rejected, as in JSON: leading zeros (`08`; the ES 5.1 `DecimalIntegerLiteral` is `0` or
/// a non-zero digit followed by digits), an empty exponent (`1e`, `1e+`), and `0x` without hex
/// digits — all lexical errors at the offending byte. A bare sign, a bare `.`, or a sign
/// followed by nothing numeric simply does not match, and surfaces as an unrecognized byte.
/// Hex floats (`0x1.5`) are not merged: the rule measures `0x1` and the parser rejects the
/// adjacent `.5` number token.
///
/// [spec]: https://spec.json5.org/
#[expect(
    clippy::module_name_repetitions,
    reason = "rule struct names mirror strata-json's public naming convention"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NumberRule;

impl Matcher for NumberRule {
    type Kind = Json5Token;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<Json5Token>>, LexError> {
        if !matches!(look.peek_byte()?, Some(b'+' | b'-' | b'.' | b'0'..=b'9')) {
            return Ok(None);
        }

        let start = look.position();
        let mut index: usize = 0;

        if matches!(look.peek(1)?.first(), Some(b'+' | b'-')) {
            index = 1;
        }

        // Signed Infinity / NaN (bare words belong to the identifier rule).
        if index > 0 {
            if let Some(len) = measure_word(look, index, b"Infinity")? {
                return Ok(Match::from_len(Json5Token::Number, len));
            }
            if let Some(len) = measure_word(look, index, b"NaN")? {
                return Ok(Match::from_len(Json5Token::Number, len));
            }
        }

        // Integer part.
        let mut int_digits: usize = 0;
        match look.peek(index.saturating_add(1))?.get(index) {
            Some(b'0') => {
                index = index.saturating_add(1);
                int_digits = 1;

                match look.peek(index.saturating_add(1))?.get(index) {
                    // Hexadecimal integer.
                    Some(b'x' | b'X') => {
                        let hex_start = index.saturating_add(1);
                        let hex_end = digit_run(look, hex_start, u8::is_ascii_hexdigit)?;

                        if hex_end == hex_start {
                            return Err(LexError::unrecognized(offset_position(start, hex_start)));
                        }

                        return Ok(Match::from_len(Json5Token::Number, hex_end));
                    }
                    // A digit after a leading zero is a grammar error (e.g. `08`).
                    Some(b'0'..=b'9') => {
                        return Err(LexError::unrecognized(offset_position(start, index)));
                    }
                    _ => {}
                }
            }
            Some(b'1'..=b'9') => {
                let end = digit_run(look, index, u8::is_ascii_digit)?;
                int_digits = end.saturating_sub(index);
                index = end;
            }
            _ => {}
        }

        // Fraction: `.` may lead (`.5`) or trail (`1.`), but not both alone (`.`).
        let mut frac_digits: usize = 0;
        if look.peek(index.saturating_add(1))?.get(index) == Some(&b'.') {
            let frac_start = index.saturating_add(1);
            let frac_end = digit_run(look, frac_start, u8::is_ascii_digit)?;
            frac_digits = frac_end.saturating_sub(frac_start);
            index = frac_end;
        }

        if int_digits == 0 && frac_digits == 0 {
            // Bare `+`, `-`, `.`, `+.`, ... — nothing numeric: decline.
            return Ok(None);
        }

        // Optional exponent.
        if matches!(
            look.peek(index.saturating_add(1))?.get(index),
            Some(b'e' | b'E')
        ) {
            index = index.saturating_add(1);

            if matches!(
                look.peek(index.saturating_add(1))?.get(index),
                Some(b'+' | b'-')
            ) {
                index = index.saturating_add(1);
            }

            let exp_end = digit_run(look, index, u8::is_ascii_digit)?;

            if exp_end == index {
                return Err(LexError::unrecognized(offset_position(start, index)));
            }

            index = exp_end;
        }

        Ok(Match::from_len(Json5Token::Number, index))
    }
}

/// Measures the run of bytes satisfying `digit` starting at `from`, filling on demand.
fn digit_run(
    look: &mut Lookahead<'_>,
    from: usize,
    digit: impl Fn(&u8) -> bool,
) -> Result<usize, LexError> {
    let mut len = from;

    loop {
        let bytes = look.peek(len.saturating_add(1))?;

        match bytes.get(len) {
            Some(byte) if digit(byte) => len = len.saturating_add(1),
            _ => return Ok(len),
        }
    }
}

/// Measures `word` at `offset`, returning the total length if the bytes match exactly.
fn measure_word(
    look: &mut Lookahead<'_>,
    offset: usize,
    word: &[u8],
) -> Result<Option<usize>, LexError> {
    let end = offset.saturating_add(word.len());
    let bytes = look.peek(end)?;

    Ok((bytes.get(offset..end) == Some(word)).then_some(end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::{lex, lex_error};
    use strata_lexer::{LexError as Error, LexErrorKind};

    fn assert_whole_number(input: &str) {
        let end = u64::try_from(input.len()).unwrap_or(u64::MAX);
        match lex(input) {
            Ok(tokens) => assert_eq!(
                tokens,
                vec![(Json5Token::Number, 0, end), (Json5Token::Eof, end, end)],
                "input {input:?}"
            ),
            Err(error) => panic!("expected a number token for {input:?}, got {error}"),
        }
    }

    #[test]
    fn json_numbers_still_lex() {
        for input in ["0", "-0", "42", "-17", "3.14", "1e10", "2.5E-3", "1e+5"] {
            assert_whole_number(input);
        }
    }

    #[test]
    fn json5_decimal_extensions_lex() {
        for input in [
            "+1", "+0", ".5", "-.5", "+.5", "1.", "-1.", "5.e2", ".5e-1", "+1.5e3", "0.", ".0",
        ] {
            assert_whole_number(input);
        }
    }

    #[test]
    fn hex_integers_lex() {
        for input in [
            "0x0",
            "0xdecaf",
            "0XABCDEF",
            "-0x1F",
            "+0xff",
            "0x0123456789abcdefABCDEF",
        ] {
            assert_whole_number(input);
        }
    }

    #[test]
    fn signed_infinity_and_nan_lex_as_numbers() {
        for input in ["+Infinity", "-Infinity", "+NaN", "-NaN"] {
            assert_whole_number(input);
        }
    }

    #[test]
    fn bare_infinity_and_nan_are_word_tokens() -> Result<(), Error> {
        assert_eq!(
            lex("Infinity")?,
            vec![(Json5Token::Infinity, 0, 8), (Json5Token::Eof, 8, 8)]
        );
        assert_eq!(
            lex("NaN")?,
            vec![(Json5Token::NaN, 0, 3), (Json5Token::Eof, 3, 3)]
        );

        Ok(())
    }

    #[test]
    fn leading_zeros_are_rejected() {
        let error = lex_error("08");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 1);

        let error = lex_error("-01");
        assert_eq!(error.position.byte(), 2);
    }

    #[test]
    fn empty_exponents_are_rejected() {
        for input in ["1e", "1E", "1e+", "1e-", ".5e"] {
            let error = lex_error(input);
            assert!(
                matches!(error.kind, LexErrorKind::Unrecognized),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn hex_without_digits_is_rejected() {
        for input in ["0x", "0X", "-0x", "0xg"] {
            let error = lex_error(input);
            assert!(
                matches!(error.kind, LexErrorKind::Unrecognized),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn bare_signs_and_dots_do_not_match() {
        for input in ["+", "-", ".", "+.", "-."] {
            let error = lex_error(input);
            assert!(
                matches!(error.kind, LexErrorKind::Unrecognized),
                "input {input:?}"
            );
            assert_eq!(error.position.byte(), 0, "input {input:?}");
        }
    }

    #[test]
    fn double_signs_do_not_match() {
        let error = lex_error("+-1");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 0);
    }

    #[test]
    fn unsigned_infinity_words_do_not_match_here() {
        // `+Infinityz`: the number rule munches `+Infinity`; `z` is a separate identifier.
        let tokens = lex("+Infinityz").unwrap_or_default();
        assert_eq!(
            tokens,
            vec![
                (Json5Token::Number, 0, 9),
                (Json5Token::Identifier, 9, 10),
                (Json5Token::Eof, 10, 10),
            ]
        );
    }
}
