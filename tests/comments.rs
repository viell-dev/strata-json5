//! Comment ergonomics: attachment to nodes, text extraction, and span fidelity.

use strata_json5::{Json5Error, Json5Number, Json5Resolver, TriviaKind};
use strata_lexer::Span;

#[test]
fn line_comments_attach_to_the_following_member() -> Result<(), Json5Error> {
    let document =
        Json5Resolver::new().resolve_str("{\n  // the crate name\n  name: 'strata'\n}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };
    let [member] = object.members() else {
        panic!("expected one member");
    };

    let comments: Vec<_> = member.name_meta().comments().collect();
    let [comment] = comments[..] else {
        panic!("expected one leading comment");
    };
    assert_eq!(comment.kind(), TriviaKind::LineComment);
    assert_eq!(
        document.comment_text(comment).as_deref(),
        Some(" the crate name")
    );

    Ok(())
}

#[test]
fn block_comment_text_loses_both_delimiters() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("/* note */ 1")?;

    let comments: Vec<_> = document.root().meta().comments().collect();
    let [comment] = comments[..] else {
        panic!("expected one leading comment");
    };
    assert_eq!(comment.kind(), TriviaKind::BlockComment);
    assert_eq!(document.comment_text(comment).as_deref(), Some(" note "));

    Ok(())
}

#[test]
fn block_comments_span_lines() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("/* a\nb */ true")?;

    let comments: Vec<_> = document.root().meta().comments().collect();
    let [comment] = comments[..] else {
        panic!("expected one leading comment");
    };
    assert_eq!(comment.line_breaks(), 1);
    assert_eq!(document.comment_text(comment).as_deref(), Some(" a\nb "));
    assert_eq!(document.root().as_bool(), Some(true));

    Ok(())
}

#[test]
fn comments_attach_inside_arrays_and_objects() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("[ // first\n1, /* second */ 2]")?;
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };

    let Some(first) = array.get(0) else {
        panic!("expected a first element");
    };
    let first_comments: Vec<_> = first.meta().comments().collect();
    let [first_comment] = first_comments[..] else {
        panic!("expected one comment before the first element");
    };
    assert_eq!(first_comment.kind(), TriviaKind::LineComment);
    assert_eq!(
        document.comment_text(first_comment).as_deref(),
        Some(" first")
    );

    let Some(second) = array.get(1) else {
        panic!("expected a second element");
    };
    let second_comments: Vec<_> = second.meta().comments().collect();
    let [second_comment] = second_comments[..] else {
        panic!("expected one comment before the second element");
    };
    assert_eq!(second_comment.kind(), TriviaKind::BlockComment);
    assert_eq!(
        document.comment_text(second_comment).as_deref(),
        Some(" second ")
    );
    assert_eq!(second.as_number(), Some(Json5Number::U64(2)));

    Ok(())
}

#[test]
fn trailing_comments_land_in_trailing_trivia() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("1 // done\n")?;

    let [space, comment, newline] = document.trailing_trivia() else {
        panic!("expected three trailing trivia pieces");
    };
    assert_eq!(space.kind(), TriviaKind::Whitespace);
    assert_eq!(comment.kind(), TriviaKind::LineComment);
    assert_eq!(newline.kind(), TriviaKind::Whitespace);

    // The line comment's span excludes the terminating line break.
    assert_eq!(comment.span(), Span::from_start_len(2, 7));
    assert_eq!(document.comment_text(comment).as_deref(), Some(" done"));

    Ok(())
}

#[test]
fn comment_spans_address_the_raw_source() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str("/* keep */ null")?;

    let comments: Vec<_> = document.root().meta().comments().collect();
    let [comment] = comments[..] else {
        panic!("expected one leading comment");
    };
    assert_eq!(comment.span(), Span::from_start_len(0, 10));
    assert_eq!(document.bytes(comment.span()), Some(&b"/* keep */"[..]));

    Ok(())
}

#[test]
fn comment_text_is_none_for_non_comments_and_reclaimed_spans() -> Result<(), Json5Error> {
    let mut document = Json5Resolver::new().resolve_str("1  // tail")?;

    let [whitespace, comment] = &document.trailing_trivia().to_vec()[..] else {
        panic!("expected two trailing trivia pieces");
    };
    assert_eq!(document.comment_text(whitespace), None);
    assert_eq!(document.comment_text(comment).as_deref(), Some(" tail"));

    // After owner-side reclamation the comment bytes are gone.
    document.source_mut().retire_all();
    assert_eq!(document.comment_text(comment), None);

    Ok(())
}
