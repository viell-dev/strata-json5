//! Source fidelity on the owned model: spans, raw bytes, trivia, and promotable handles.

use strata_json5::{Json5Error, Json5Number, Json5Resolver, Json5Value, TriviaKind};
use strata_lexer::Span;

#[test]
fn node_spans_address_the_raw_source() -> Result<(), Json5Error> {
    let input = "{a: [1, 'x']}";
    let document = Json5Resolver::new().resolve_str(input)?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    // The object's span covers the whole input; raw bytes round-trip.
    assert_eq!(object.meta().span(), Span::from_start_len(0, 13));
    assert_eq!(document.bytes(object.meta().span()), Some(input.as_bytes()));

    let [member] = object.members() else {
        panic!("expected one member");
    };
    // The unquoted name's span covers just the identifier — no quotes to include.
    assert_eq!(document.bytes(member.name_meta().span()), Some(&b"a"[..]));

    let Some(array) = member.value().as_array() else {
        panic!("expected an array");
    };
    assert_eq!(document.bytes(array.meta().span()), Some(&b"[1, 'x']"[..]));
    assert_eq!(
        array
            .get(1)
            .map(Json5Value::span)
            .and_then(|span| document.bytes(span)),
        Some(&b"'x'"[..])
    );

    Ok(())
}

#[test]
fn node_spans_exclude_leading_comments() -> Result<(), Json5Error> {
    let input = "/* head */ [1]";
    let document = Json5Resolver::new().resolve_str(input)?;
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };

    // The comment stays leading trivia; the array span starts at its `[`.
    assert_eq!(array.meta().span(), Span::from_start_len(11, 3));
    assert_eq!(document.bytes(array.meta().span()), Some(&b"[1]"[..]));

    Ok(())
}

#[test]
fn positions_track_lines_and_columns() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("{\n  // note\n  a: 1\n}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };
    let [member] = object.members() else {
        panic!("expected one member");
    };

    // The comment occupies line 2; the member name starts on line 3.
    assert_eq!(member.name_meta().start().line(), 3);
    assert_eq!(member.name_meta().start().column(), 3);
    assert_eq!(object.meta().start().line(), 1);
    assert_eq!(object.meta().end().line(), 4);

    Ok(())
}

#[test]
fn leading_trivia_carries_blank_line_fidelity() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("[1,\n\n\n2]")?;
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };
    let Some(second) = array.get(1) else {
        panic!("expected a second element");
    };
    let [trivia] = second.meta().leading() else {
        panic!("expected one leading trivia piece");
    };

    assert_eq!(trivia.kind(), TriviaKind::Whitespace);
    assert_eq!(trivia.line_breaks(), 3);
    assert_eq!(trivia.blank_lines(), 2);

    Ok(())
}

#[test]
fn trailing_commas_are_core_language() -> Result<(), Json5Error> {
    // No option to enable: JSON5 always tolerates one trailing comma.
    let document = Json5Resolver::new().resolve_str("[1, 2,]")?;
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };
    assert_eq!(array.len(), 2);

    let document = Json5Resolver::new().resolve_str("{a: 1, /* last */}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };
    assert_eq!(
        object.get("a").and_then(Json5Value::as_number),
        Some(Json5Number::U64(1))
    );

    // A comma with no preceding entry stays an error.
    assert!(Json5Resolver::new().resolve_str("[,]").is_err());
    assert!(Json5Resolver::new().resolve_str("{,}").is_err());

    Ok(())
}

#[test]
fn trailing_trivia_is_retained() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("1  \n")?;
    let [trailing] = document.trailing_trivia() else {
        panic!("expected one trailing trivia piece");
    };

    assert_eq!(trailing.kind(), TriviaKind::Whitespace);
    assert_eq!(trailing.span(), Span::from_start_len(1, 3));

    Ok(())
}

#[test]
fn source_refs_survive_store_reclamation() -> Result<(), Json5Error> {
    let mut document = Json5Resolver::new().resolve_str("['payload']")?;
    let span = {
        let Some(array) = document.root().as_array() else {
            panic!("expected an array");
        };
        let Some(element) = array.get(0) else {
            panic!("expected an element");
        };
        element.span()
    };

    let Some(source_ref) = document.source_ref(span) else {
        panic!("expected the span to be retained");
    };
    assert!(source_ref.is_loaned());

    // Owner-side reclamation promotes the handle instead of invalidating it.
    document.source_mut().retire_all();
    assert!(source_ref.is_owned());
    assert_eq!(source_ref.resolve(), b"'payload'".to_vec());

    // Plain byte lookups into the retired store no longer resolve...
    assert_eq!(document.bytes(span), None);
    // ...but the owned values are unaffected.
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };
    assert_eq!(array.get(0).and_then(Json5Value::as_str), Some("payload"));

    Ok(())
}

#[test]
fn resolving_a_reclaimed_document_reports_source_unavailable() {
    let parser = strata_json5_parser::Json5Parser::new();
    let Ok(mut parsed) = parser.parse_str("'text'") else {
        panic!("expected the parse to succeed");
    };
    parsed.source_mut().retire_all();

    let Err(error) = Json5Resolver::new().resolve_document(parsed) else {
        panic!("expected resolution to fail without retained source");
    };
    assert!(matches!(
        error.kind,
        strata_json5::Json5ErrorKind::SourceUnavailable { .. }
    ));
}
