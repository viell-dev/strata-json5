//! Superset composition: a miniature dialect built from this crate's public token rules,
//! proving that a superset of JSON5 can reuse them exactly as this crate reuses
//! `strata-json`'s — import the rule structs, lift them with `map_kind` into a wider kind, and
//! add what differs (here, a `#` line comment).

use std::io::Cursor;
use strata_json5::rules::{
    IdentifierRule, Json5Token, JsonToken, LiteralRule, NumberRule, PunctRule, StringRule,
    WhitespaceRule,
};
use strata_lexer::{
    LexError, Lexer, Lookahead, Match, Matcher, MatcherExt, MatcherVec, SpecScanner,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialectToken {
    Json5(Json5Token),
    HashComment,
    Eof,
}

/// A `#` line comment rule — the one shape this dialect adds over JSON5.
#[derive(Debug, Clone, Copy, Default)]
struct HashCommentRule;

impl Matcher for HashCommentRule {
    type Kind = DialectToken;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<DialectToken>>, LexError> {
        if look.peek_byte()? != Some(b'#') {
            return Ok(None);
        }

        let mut len: usize = 1;

        loop {
            let bytes = look.peek(len.saturating_add(1))?;

            match bytes.get(len) {
                Some(b'\n' | b'\r') | None => {
                    return Ok(Some(Match::new(DialectToken::HashComment, len)));
                }
                Some(_) => len = len.saturating_add(1),
            }
        }
    }
}

fn dialect_rules() -> MatcherVec<DialectToken> {
    let wrap: fn(Json5Token) -> DialectToken = DialectToken::Json5;

    vec![
        HashCommentRule.boxed(),
        WhitespaceRule.map_kind(wrap).boxed(),
        // Rules this crate itself lifted from strata-json compose again transparently.
        PunctRule.map_kind(Json5Token::Json).map_kind(wrap).boxed(),
        LiteralRule
            .map_kind(Json5Token::Json)
            .map_kind(wrap)
            .boxed(),
        IdentifierRule.map_kind(wrap).boxed(),
        StringRule.map_kind(wrap).boxed(),
        NumberRule.map_kind(wrap).boxed(),
        // The JSON5 comment rules are deliberately omitted: this dialect uses `#` only.
    ]
}

fn lex_kinds(input: &str) -> Result<Vec<DialectToken>, LexError> {
    let scanner = SpecScanner::new(dialect_rules(), DialectToken::Eof);
    let mut lexer = Lexer::new(Cursor::new(input.as_bytes().to_vec()), scanner);
    let mut kinds = Vec::new();

    loop {
        let token = lexer.next_token()?;
        let done = token.kind == DialectToken::Eof;
        kinds.push(token.kind);

        if done {
            return Ok(kinds);
        }
    }
}

#[test]
fn reused_json5_rules_lex_under_the_wider_kind() -> Result<(), LexError> {
    let kinds = lex_kinds("{key: 0x1} # note")?;

    assert_eq!(
        kinds,
        vec![
            DialectToken::Json5(Json5Token::Json(JsonToken::LeftBrace)),
            DialectToken::Json5(Json5Token::Identifier),
            DialectToken::Json5(Json5Token::Json(JsonToken::Colon)),
            DialectToken::Json5(Json5Token::Whitespace),
            DialectToken::Json5(Json5Token::Number),
            DialectToken::Json5(Json5Token::Json(JsonToken::RightBrace)),
            DialectToken::Json5(Json5Token::Whitespace),
            DialectToken::HashComment,
            DialectToken::Eof,
        ]
    );

    Ok(())
}

#[test]
fn omitted_rules_leave_the_shape_unknown() {
    // `//` comments were not composed in, so the dialect simply does not know the shape.
    let Err(error) = lex_kinds("1 // gone") else {
        panic!("expected the omitted comment rule to leave `//` unrecognized");
    };
    assert_eq!(error.position.byte, 2);
}

#[test]
fn reused_rules_keep_their_validation() {
    // The lifted number rule still rejects leading zeros inside the dialect.
    let Err(error) = lex_kinds("08") else {
        panic!("expected the reused number rule to keep rejecting leading zeros");
    };
    assert_eq!(error.position.byte, 1);

    // The lifted literal/identifier pair keeps its priority tie-break.
    let kinds = lex_kinds("true").unwrap_or_default();
    assert_eq!(
        kinds,
        vec![
            DialectToken::Json5(Json5Token::Json(JsonToken::True)),
            DialectToken::Eof,
        ]
    );
}
