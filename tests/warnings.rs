//! Spec-mandated warnings: unescaped U+2028/U+2029 in strings surface on the resolved document.

use strata_json5::{Json5Error, Json5Resolver, WarningKind};

#[test]
fn unescaped_line_separators_resolve_with_a_warning() -> Result<(), Json5Error> {
    // Legal JSON5 string content: the resolve succeeds and the warning is recorded.
    let document = Json5Resolver::new().resolve_str("'a\u{2028}b'")?;

    assert_eq!(document.root().as_str(), Some("a\u{2028}b"));

    let [warning] = document.warnings() else {
        panic!("expected exactly one warning");
    };
    assert_eq!(
        warning.kind,
        WarningKind::UnescapedLineSeparator {
            separator: '\u{2028}',
        }
    );
    // Byte 0 is the quote, byte 1 the `a`, byte 2 the separator.
    assert_eq!(warning.position.byte(), 2);

    Ok(())
}

#[test]
fn both_separators_warn_in_source_order() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("['\u{2028}', '\u{2029}']")?;

    let separators: Vec<char> = document
        .warnings()
        .iter()
        .map(|warning| match warning.kind {
            WarningKind::UnescapedLineSeparator { separator } => separator,
        })
        .collect();
    assert_eq!(separators, vec!['\u{2028}', '\u{2029}']);

    Ok(())
}

#[test]
fn escaped_separators_do_not_warn() -> Result<(), Json5Error> {
    // A backslash before the separator is a line continuation, not an unescaped occurrence.
    let document = Json5Resolver::new().resolve_str("'a\\\u{2028}b'")?;

    assert_eq!(document.root().as_str(), Some("ab"));
    assert!(document.warnings().is_empty());

    Ok(())
}

#[test]
fn clean_documents_carry_no_warnings() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("{a: [1, 'two']}")?;

    assert!(document.warnings().is_empty());

    Ok(())
}

#[test]
fn warnings_survive_into_parts() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("'\u{2029}'")?;
    let (root, _trailing, warnings, _source) = document.into_parts();

    assert_eq!(root.as_str(), Some("\u{2029}"));
    assert_eq!(warnings.len(), 1);

    Ok(())
}

#[test]
fn warnings_carry_through_resolve_document() -> Result<(), Json5Error> {
    let parser = strata_json5_parser::Json5Parser::new();
    let Ok(parsed) = parser.parse_str("'\u{2028}'") else {
        panic!("expected the parse to succeed");
    };
    assert_eq!(parsed.warnings().len(), 1);

    let document = Json5Resolver::new().resolve_document(parsed)?;
    assert_eq!(document.warnings().len(), 1);

    Ok(())
}
