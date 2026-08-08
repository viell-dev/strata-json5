//! Owned-value resolution: scalars, JSON5 string decoding, and escape handling.

use strata_json5::{
    Json5Error, Json5ErrorKind, Json5Number, Json5Resolver, Json5Value, LoneSurrogates,
};

fn root(input: &str) -> Result<Json5Value, Json5Error> {
    Ok(Json5Resolver::new().resolve_str(input)?.into_parts().0)
}

#[test]
fn scalars_resolve_to_owned_values() -> Result<(), Json5Error> {
    assert!(root("null")?.is_null());
    assert_eq!(root("true")?.as_bool(), Some(true));
    assert_eq!(root("false")?.as_bool(), Some(false));
    assert_eq!(root("42")?.as_number(), Some(Json5Number::U64(42)));
    assert_eq!(root(r#""hello""#)?.as_str(), Some("hello"));

    Ok(())
}

#[test]
fn scalars_resolve_with_comments_around_them() -> Result<(), Json5Error> {
    assert_eq!(
        root("// leading\n42 /* trailing */")?.as_number(),
        Some(Json5Number::U64(42))
    );

    Ok(())
}

#[test]
fn single_quoted_strings_decode() -> Result<(), Json5Error> {
    assert_eq!(root("'hello'")?.as_str(), Some("hello"));
    assert_eq!(root("''")?.as_str(), Some(""));
    // The other quote kind is ordinary content.
    assert_eq!(root("'say \"hi\"'")?.as_str(), Some("say \"hi\""));
    assert_eq!(root("\"it's\"")?.as_str(), Some("it's"));

    Ok(())
}

#[test]
fn strings_decode_every_simple_escape() -> Result<(), Json5Error> {
    let value = root(r#""\"\\\/\b\f\n\r\t""#)?;
    assert_eq!(value.as_str(), Some("\"\\/\u{08}\u{0C}\n\r\t"));

    // The JSON5-only escapes: \' , \v, \0, and self-escaping non-escape characters.
    assert_eq!(root(r"'\''")?.as_str(), Some("'"));
    assert_eq!(root(r"'\v'")?.as_str(), Some("\u{0B}"));
    assert_eq!(root(r"'a\0b'")?.as_str(), Some("a\u{0}b"));
    assert_eq!(root(r"'\q'")?.as_str(), Some("q"));

    Ok(())
}

#[test]
fn hex_escapes_decode() -> Result<(), Json5Error> {
    assert_eq!(root(r"'\x41'")?.as_str(), Some("A"));
    assert_eq!(root(r"'\xe9'")?.as_str(), Some("\u{E9}"));

    Ok(())
}

#[test]
fn line_continuations_produce_no_output() -> Result<(), Json5Error> {
    for terminator in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
        let raw = format!("'a\\{terminator}b'");
        assert_eq!(
            root(&raw)?.as_str(),
            Some("ab"),
            "terminator {terminator:?}"
        );
    }

    Ok(())
}

#[test]
fn strings_decode_unicode_escapes_and_surrogate_pairs() -> Result<(), Json5Error> {
    assert_eq!(
        root("\"\\u0041\\u00e9\\u20AC\"")?.as_str(),
        Some("A\u{E9}\u{20AC}")
    );
    assert_eq!(root("\"\\uD83D\\uDE00\"")?.as_str(), Some("\u{1F600}"));
    assert_eq!(root("'\\uD83D\\uDE00'")?.as_str(), Some("\u{1F600}"));
    assert_eq!(root("\"\\u0000\"")?.as_str(), Some("\u{0}"));

    Ok(())
}

#[test]
fn surrogate_pairs_combine_across_a_line_continuation() -> Result<(), Json5Error> {
    // The continuation contributes nothing, leaving the code units adjacent.
    assert_eq!(root("'\\uD83D\\\n\\uDE00'")?.as_str(), Some("\u{1F600}"));

    Ok(())
}

#[test]
fn comment_delimiters_inside_strings_are_content() -> Result<(), Json5Error> {
    assert_eq!(
        root(r#""not // a comment""#)?.as_str(),
        Some("not // a comment")
    );
    assert_eq!(root("'nor /* this */'")?.as_str(), Some("nor /* this */"));

    Ok(())
}

#[test]
fn multibyte_content_survives_verbatim() -> Result<(), Json5Error> {
    assert_eq!(
        root("'caf\u{E9} \u{1F600}'")?.as_str(),
        Some("caf\u{E9} \u{1F600}")
    );

    Ok(())
}

#[test]
fn lone_surrogates_error_by_default() {
    let Err(error) = root(r"'\uD800'") else {
        panic!("expected a lone-surrogate error");
    };

    assert!(matches!(
        error.kind,
        Json5ErrorKind::LoneSurrogate { unit: 0xD800 }
    ));
    // Byte 0 is the quote, byte 1 the backslash of the lone escape.
    assert_eq!(error.position.byte(), 1);
}

#[test]
fn lone_surrogates_replace_under_policy() -> Result<(), Json5Error> {
    let resolver = Json5Resolver::builder()
        .lone_surrogates(LoneSurrogates::Replace)
        .build();
    let document = resolver.resolve_str(r"'a\uDC00b'")?;

    assert_eq!(document.root().as_str(), Some("a\u{FFFD}b"));

    Ok(())
}

#[test]
fn arrays_resolve_recursively() -> Result<(), Json5Error> {
    let value = root("[1, 'two', [true, null],]")?;
    let Some(array) = value.as_array() else {
        panic!("expected an array");
    };

    assert_eq!(array.len(), 3);
    assert_eq!(
        array.get(0).and_then(Json5Value::as_number),
        Some(Json5Number::U64(1))
    );
    assert_eq!(array.get(1).and_then(Json5Value::as_str), Some("two"));

    let Some(inner) = array.get(2).and_then(Json5Value::as_array) else {
        panic!("expected a nested array");
    };
    assert_eq!(inner.get(0).and_then(Json5Value::as_bool), Some(true));
    assert!(inner.get(1).is_some_and(Json5Value::is_null));
    assert!(!array.is_empty());

    Ok(())
}

#[test]
fn parse_errors_pass_through() {
    let Err(error) = root("[1 2]") else {
        panic!("expected a parse error");
    };

    assert!(matches!(error.kind, Json5ErrorKind::Parse(_)));
    assert!(error.to_string().contains("expected ',' or ']'"));
}
