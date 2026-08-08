# strata-json5

JSON5 ([json5.org](https://json5.org)) resolver for the Strata parser ecosystem: a read-only,
dynamic document model with owned values — decoded strings and converted numbers, including JSON5
value semantics (`Infinity`/`NaN`, hex numbers, unquoted member names) — that keeps source
fidelity available (spans, raw bytes, member order, duplicate members, and trivia), built on
`strata-json5-parser`'s CST.

## Status

This crate is in initial development. The public API is not settled yet.

## Usage

```rust
use strata_json5::{Json5Error, Json5Number, Json5Resolver, NameStyle};

fn main() -> Result<(), Json5Error> {
    let document = Json5Resolver::new().resolve_str(
        "{\n  // limits\n  max: 0x1F,\n  'label': 'caf\\u00e9',\n}",
    )?;

    let Some(object) = document.root().as_object() else {
        panic!("expected an object root");
    };

    // Owned, decoded values with JSON5 semantics: hex numbers, unquoted and
    // single-quoted member names, trailing commas — all mandatory, no options.
    assert_eq!(
        object.get("max").and_then(|v| v.as_number()),
        Some(Json5Number::U64(31))
    );
    assert_eq!(object.get("label").and_then(|v| v.as_str()), Some("café"));

    // Fidelity: name spellings and comments stay reachable.
    let [max, label] = object.members() else {
        panic!("expected two members");
    };
    assert_eq!(max.name_style(), NameStyle::Identifier);
    assert_eq!(label.name_style(), NameStyle::SingleQuoted);

    let Some(comment) = max.name_meta().comments().next() else {
        panic!("expected the comment before \"max\"");
    };
    assert_eq!(document.comment_text(comment).as_deref(), Some(" limits"));

    Ok(())
}
```

See [docs/strata-json5.md](docs/strata-json5.md) for the design reference, including the
duplicate-member and lone-surrogate policies and how spec-mandated warnings surface.

## Minimum Supported Rust Version

The MSRV is **1.87.0**.

## License

Licensed under the Mozilla Public License, Version 2.0 (see [LICENSE](LICENSE)
or <https://www.mozilla.org/en-US/MPL/2.0/>).

MPL-2.0 is a file-level weak copyleft license: its obligations attach to this crate's files (in
source and compiled form) and to modifications of them, not to your own source files that merely
use this crate. Distributing a binary that includes this crate obliges you to tell recipients
that the crate is included and how to obtain its source; for an unmodified crate, pointing them
at the exact crates.io release is a reasonable way to do that. Example material (documentation
code examples, `examples/`, integration tests, and benchmarks) is additionally licensed under
[MIT-0](LICENSE-MIT-0), so it can be copied freely. See
[LICENSE-CLARIFICATION.md](LICENSE-CLARIFICATION.md) for the maintainers' full statement of
intent regarding normal Rust usage (linking, monomorphization, macro expansion), binary
distribution, and generated code, and [CONTRIBUTING.md](CONTRIBUTING.md) for the contribution
policy.
