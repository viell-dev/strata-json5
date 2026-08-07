//! The JSON5 comment rules: `//` line comments and `/* ... */` block comments.
//!
//! Comments are the shapes JSON5 adds over JSON with no JSON counterpart at all. They are trivia:
//! the parser retains them on the following token instead of discarding them. The two rules are
//! separate zero-sized matchers so a superset of JSON5 (or a sibling dialect) can pick either
//! independently.

use crate::rules::{CharAt, Json5Token, offset_position, peek_char_at, position_at};
use strata_lexer::{LexError, Lookahead, Match, Matcher};

/// Matches a `//` line comment, up to but *excluding* the terminating line terminator (LF, CR,
/// U+2028, or U+2029) — the terminator is ordinary whitespace trivia — or to end of input
/// ([spec]: a single line comment ends with a `LineTerminator` or the end of the document).
///
/// The comment body must be valid UTF-8; an invalid byte is a lexical error at its position. A
/// lone `/` matches neither comment rule and surfaces as an unrecognized byte.
///
/// [spec]: https://spec.json5.org/
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineCommentRule;

impl Matcher for LineCommentRule {
    type Kind = Json5Token;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<Json5Token>>, LexError> {
        if look.peek(2)? != b"//" {
            return Ok(None);
        }

        let start = look.position();
        let mut index: usize = 2;

        loop {
            match peek_char_at(look, index)? {
                CharAt::End => return Ok(Match::from_len(Json5Token::LineComment, index)),
                CharAt::Invalid => {
                    // Line comments cannot contain line breaks, so the offset stays in-line.
                    return Err(LexError::unrecognized(offset_position(start, index)));
                }
                CharAt::Char { ch, width } => match ch {
                    '\n' | '\r' | '\u{2028}' | '\u{2029}' => {
                        return Ok(Match::from_len(Json5Token::LineComment, index));
                    }
                    _ => index = index.saturating_add(width),
                },
            }
        }
    }
}

/// Matches a `/* ... */` block comment, both delimiters included. Block comments do not nest
/// ([spec]); the first `*/` closes the comment.
///
/// The body may contain anything — line terminators included — but must be valid UTF-8; an
/// invalid byte is a lexical error at its (line-aware) position. A comment with no closing
/// `*/` is
/// [`LexErrorKind::UnterminatedDelimiter`](strata_lexer::LexErrorKind::UnterminatedDelimiter)
/// at the opening `/`.
///
/// [spec]: https://spec.json5.org/
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockCommentRule;

impl Matcher for BlockCommentRule {
    type Kind = Json5Token;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<Json5Token>>, LexError> {
        if look.peek(2)? != b"/*" {
            return Ok(None);
        }

        let start = look.position();
        let mut index: usize = 2;

        loop {
            match peek_char_at(look, index)? {
                CharAt::End => return Err(LexError::unterminated_delimiter(start)),
                CharAt::Invalid => {
                    return Err(LexError::unrecognized(position_at(look, start, index)));
                }
                CharAt::Char { ch, width } => {
                    if ch == '*' {
                        let after = index.saturating_add(1);
                        let bytes = look.peek(after.saturating_add(1))?;

                        if bytes.get(after) == Some(&b'/') {
                            return Ok(Match::from_len(
                                Json5Token::BlockComment,
                                after.saturating_add(1),
                            ));
                        }
                    }

                    index = index.saturating_add(width);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::{lex, lex_error};
    use strata_lexer::{LexError as Error, LexErrorKind};

    #[test]
    fn line_comment_runs_to_end_of_input() -> Result<(), Error> {
        assert_eq!(
            lex("// note")?,
            vec![(Json5Token::LineComment, 0, 7), (Json5Token::Eof, 7, 7)]
        );

        Ok(())
    }

    #[test]
    fn line_comment_excludes_every_terminator_kind() -> Result<(), Error> {
        for terminator in ["\n", "\r", "\u{2028}", "\u{2029}"] {
            let input = format!("//x{terminator}1");
            let tokens = lex(&input)?;
            assert_eq!(
                tokens.first().copied(),
                Some((Json5Token::LineComment, 0, 3)),
                "terminator {terminator:?}"
            );
            assert_eq!(
                tokens.get(1).map(|token| token.0),
                Some(Json5Token::Whitespace),
                "terminator {terminator:?}"
            );
        }

        Ok(())
    }

    #[test]
    fn empty_line_comment_lexes() -> Result<(), Error> {
        assert_eq!(
            lex("//")?,
            vec![(Json5Token::LineComment, 0, 2), (Json5Token::Eof, 2, 2)]
        );

        Ok(())
    }

    #[test]
    fn block_comment_lexes_across_lines() -> Result<(), Error> {
        let input = "/* a\n * b\n */";
        let end = u64::try_from(input.len()).unwrap_or(u64::MAX);
        assert_eq!(
            lex(input)?,
            vec![
                (Json5Token::BlockComment, 0, end),
                (Json5Token::Eof, end, end)
            ]
        );

        Ok(())
    }

    #[test]
    fn block_comments_do_not_nest() {
        // The first `*/` closes; the rest is separate (invalid) input.
        let error = lex_error("/* /* */ */");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 9);
    }

    #[test]
    fn unterminated_block_comment_is_rejected_at_the_opening() {
        for input in ["/*", "/* open", "/* almost *"] {
            let error = lex_error(input);
            assert!(
                matches!(error.kind, LexErrorKind::UnterminatedDelimiter),
                "input {input:?}"
            );
            assert_eq!(error.position.byte(), 0, "input {input:?}");
        }
    }

    #[test]
    fn lone_slash_is_unrecognized() {
        let error = lex_error("/");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte(), 0);
    }

    #[test]
    fn comments_may_contain_multibyte_content() -> Result<(), Error> {
        let input = "// caf\u{E9} \u{1F600}";
        let end = u64::try_from(input.len()).unwrap_or(u64::MAX);
        assert_eq!(
            lex(input)?,
            vec![
                (Json5Token::LineComment, 0, end),
                (Json5Token::Eof, end, end)
            ]
        );

        Ok(())
    }
}
