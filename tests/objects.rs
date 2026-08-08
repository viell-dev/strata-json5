//! Object semantics: name styles and decoding, source order, duplicates, and the policy.

use strata_json5::{
    DuplicateKeys, Json5Error, Json5ErrorKind, Json5Member, Json5Number, Json5Resolver, Json5Value,
    NameStyle,
};

fn object(input: &str) -> Result<strata_json5::Json5Document, Json5Error> {
    Json5Resolver::new().resolve_str(input)
}

#[test]
fn members_keep_source_order_across_name_styles() -> Result<(), Json5Error> {
    let document = object("{z: 1, 'a': 2, \"m\": 3}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    let names: Vec<&str> = object.members().iter().map(Json5Member::name).collect();
    assert_eq!(names, vec!["z", "a", "m"]);
    assert_eq!(object.len(), 3);
    assert!(!object.is_empty());
    assert!(object.contains("a"));
    assert!(!object.contains("q"));

    Ok(())
}

#[test]
fn every_name_style_exposes_the_same_decoded_name() -> Result<(), Json5Error> {
    for input in ["{\"a\": 1}", "{'a': 1}", "{a: 1}", "{\\u0061: 1}"] {
        let document = object(input)?;
        let Some(object) = document.root().as_object() else {
            panic!("expected an object for {input:?}");
        };

        let [member] = object.members() else {
            panic!("expected one member for {input:?}");
        };
        assert_eq!(member.name(), "a", "input {input:?}");
        assert_eq!(
            object.get("a").and_then(Json5Value::as_number),
            Some(Json5Number::U64(1)),
            "input {input:?}"
        );
    }

    Ok(())
}

#[test]
fn name_styles_are_reported() -> Result<(), Json5Error> {
    let document = object("{a: 1, 'b': 2, \"c\": 3}")?;
    let Some(mixed) = document.root().as_object() else {
        panic!("expected an object");
    };

    let styles: Vec<NameStyle> = mixed
        .members()
        .iter()
        .map(Json5Member::name_style)
        .collect();
    assert_eq!(
        styles,
        vec![
            NameStyle::Identifier,
            NameStyle::SingleQuoted,
            NameStyle::DoubleQuoted,
        ]
    );
    // Identifier escapes stay identifiers stylistically; the decoded name loses the escape.
    let escaped = object("{\\u0061: 1}")?;
    let Some(escaped_object) = escaped.root().as_object() else {
        panic!("expected an object");
    };
    let [member] = escaped_object.members() else {
        panic!("expected one member");
    };
    assert_eq!(member.name_style(), NameStyle::Identifier);
    assert_eq!(member.name(), "a");

    Ok(())
}

#[test]
fn reserved_words_are_valid_member_names() -> Result<(), Json5Error> {
    let document = object("{true: 1, null: 2, Infinity: 3, NaN: 4}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    let names: Vec<&str> = object.members().iter().map(Json5Member::name).collect();
    assert_eq!(names, vec!["true", "null", "Infinity", "NaN"]);
    assert!(
        object
            .members()
            .iter()
            .all(|member| member.name_style() == NameStyle::Identifier)
    );

    Ok(())
}

#[test]
fn get_resolves_to_the_last_occurrence() -> Result<(), Json5Error> {
    let document = object("{a: 1, b: 2, a: 3}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    assert_eq!(
        object.get("a").and_then(Json5Value::as_number),
        Some(Json5Number::U64(3))
    );
    assert_eq!(object.len(), 3);

    let all: Vec<Option<Json5Number>> = object
        .get_all("a")
        .map(|member| member.value().as_number())
        .collect();
    assert_eq!(
        all,
        vec![Some(Json5Number::U64(1)), Some(Json5Number::U64(3))]
    );

    Ok(())
}

#[test]
fn get_spans_name_styles() -> Result<(), Json5Error> {
    // The identifier and quoted spellings are the same name; get resolves to the last.
    let document = object("{a: 1, 'a': 2}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    assert_eq!(
        object.get("a").and_then(Json5Value::as_number),
        Some(Json5Number::U64(2))
    );
    assert_eq!(object.get_all("a").count(), 2);

    Ok(())
}

#[test]
fn duplicates_reports_shadowed_members() -> Result<(), Json5Error> {
    let document = object("{a: 1, b: 2, 'a': 3, \"b\": 4, c: 5}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    let shadowed: Vec<(&str, Option<Json5Number>)> = object
        .duplicates()
        .map(|member| (member.name(), member.value().as_number()))
        .collect();
    assert_eq!(
        shadowed,
        vec![
            ("a", Some(Json5Number::U64(1))),
            ("b", Some(Json5Number::U64(2))),
        ]
    );

    Ok(())
}

#[test]
fn duplicate_keys_error_policy_rejects_at_the_repeated_name() {
    let resolver = Json5Resolver::builder()
        .duplicate_keys(DuplicateKeys::Error)
        .build();
    let Err(error) = resolver.resolve_str("{a: 1, a: 2}") else {
        panic!("expected a duplicate-member error");
    };

    let Json5ErrorKind::DuplicateMember { name } = &error.kind else {
        panic!("expected DuplicateMember, got {:?}", error.kind);
    };
    assert_eq!(name, "a");
    // The repeated (second) name token starts at byte 7.
    assert_eq!(error.position.byte(), 7);
}

#[test]
fn duplicate_detection_uses_decoded_names_across_styles() {
    let resolver = Json5Resolver::builder()
        .duplicate_keys(DuplicateKeys::Error)
        .build();

    for input in [
        "{a: 1, 'a': 2}",
        "{'a': 1, \"a\": 2}",
        "{\"a\": 1, a: 2}",
        "{a: 1, \\u0061: 2}",
        "{a: 1, \"\\u0061\": 2}",
    ] {
        let Err(error) = resolver.resolve_str(input) else {
            panic!("expected a duplicate-member error for {input:?}");
        };
        assert!(
            matches!(
                error.kind,
                Json5ErrorKind::DuplicateMember { ref name } if name == "a"
            ),
            "input {input:?}"
        );
    }
}

#[test]
fn commented_members_resolve_like_plain_ones() -> Result<(), Json5Error> {
    let document = object("{\n  // first\n  a: 1,\n  /* second */ 'b': 2,\n}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    let names: Vec<&str> = object.members().iter().map(Json5Member::name).collect();
    assert_eq!(names, vec!["a", "b"]);
    assert_eq!(
        object.get("b").and_then(Json5Value::as_number),
        Some(Json5Number::U64(2))
    );

    Ok(())
}

#[test]
fn nested_objects_resolve() -> Result<(), Json5Error> {
    let document = object("{outer: {inner: [1]}}")?;
    let Some(outer) = document.root().as_object() else {
        panic!("expected an object");
    };
    let Some(inner) = outer.get("outer").and_then(Json5Value::as_object) else {
        panic!("expected a nested object");
    };
    let Some(array) = inner.get("inner").and_then(Json5Value::as_array) else {
        panic!("expected a nested array");
    };

    assert_eq!(array.len(), 1);

    Ok(())
}
