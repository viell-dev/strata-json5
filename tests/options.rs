//! Option passthrough to the parser and resolver-level policies.

use strata_json5::{
    DuplicateKeys, Json5Error, Json5ErrorKind, Json5Number, Json5Resolver, LoneSurrogates,
    TriviaKind,
};

#[test]
fn a_byte_order_mark_is_ordinary_whitespace() -> Result<(), Json5Error> {
    // JSON5 classifies U+FEFF as whitespace, so there is no allow_bom option to reach for.
    let document = Json5Resolver::new().resolve_str("\u{FEFF}1")?;

    let [whitespace] = document.root().meta().leading() else {
        panic!("expected the BOM as one leading whitespace run");
    };
    assert_eq!(whitespace.kind(), TriviaKind::Whitespace);
    assert_eq!(document.root().as_number(), Some(Json5Number::U64(1)));

    Ok(())
}

#[test]
fn max_depth_passes_through() {
    let resolver = Json5Resolver::builder().max_depth(1).build();

    assert!(resolver.resolve_str("[1]").is_ok());

    let Err(error) = resolver.resolve_str("[[1]]") else {
        panic!("expected a depth error");
    };
    let Json5ErrorKind::Parse(parse) = &error.kind else {
        panic!("expected a parse error, got {:?}", error.kind);
    };
    assert!(matches!(
        parse.kind,
        strata_json5_parser::Json5ErrorKind::DepthLimitExceeded { limit: 1 }
    ));
}

#[test]
fn resolver_accessors_report_configuration() {
    let resolver = Json5Resolver::builder()
        .duplicate_keys(DuplicateKeys::Error)
        .lone_surrogates(LoneSurrogates::Replace)
        .max_depth(7)
        .build();

    assert_eq!(resolver.duplicate_keys(), DuplicateKeys::Error);
    assert_eq!(resolver.lone_surrogates(), LoneSurrogates::Replace);
    assert_eq!(resolver.parser().max_depth(), 7);
}

#[test]
fn new_matches_the_builder_defaults() {
    let resolver = Json5Resolver::new();
    let built = Json5Resolver::builder().build();

    assert_eq!(resolver.duplicate_keys(), built.duplicate_keys());
    assert_eq!(resolver.lone_surrogates(), built.lone_surrogates());
    assert_eq!(
        resolver.parser().max_depth(),
        strata_json5_parser::DEFAULT_MAX_DEPTH
    );
}

#[test]
fn resolve_document_accepts_an_external_parse() -> Result<(), Json5Error> {
    let parser = strata_json5_parser::Json5Parser::builder().build();
    let Ok(parsed) = parser.parse_str("[true,] // done") else {
        panic!("expected the parse to succeed");
    };

    let document = Json5Resolver::new().resolve_document(parsed)?;
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };
    assert_eq!(array.len(), 1);
    assert!(document.warnings().is_empty());

    Ok(())
}
