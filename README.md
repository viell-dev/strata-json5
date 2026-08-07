# strata-json5

JSON5 ([json5.org](https://json5.org)) parser for the Strata parser ecosystem.

`strata-json5` parses JSON5 as specified at [spec.json5.org](https://spec.json5.org/) — strictly,
with no extensions — into a lossless, zero-copy concrete syntax tree: comments and whitespace are
retained as trivia, tokens keep exact byte spans, and string/number content stays as raw source
bytes for a downstream resolver to decode. It is composed on top of
[`strata-json`](https://codeberg.org/Devious-Concepts/strata-json), reusing the token rules the
two languages share.

## Usage

```rust
use strata_json5::{Json5Parser, Json5Token, Value};

let document = Json5Parser::new().parse_str(
    "{\n  // metadata\n  name: 'strata', limit: 0xFF,\n}",
)?;

let Value::Object(object) = document.root() else {
    unreachable!();
};

// Unquoted member names, with the comment retained as leading trivia.
let member = object.members.first().unwrap();
assert_eq!(member.name.kind, Json5Token::Identifier);
assert_eq!(document.token_bytes(&member.name), Some(&b"name"[..]));

// Spec-mandated warnings (unescaped U+2028/U+2029 in strings) surface on the document.
assert!(document.warnings().is_empty());
```

The parser is strict by default and JSON5 has no optional features; the builder only exposes the
spec-permitted nesting-depth limit and hooks for replacing the lower layers. See
[`docs/strata-json5.md`](docs/strata-json5.md) for the API map, the composition pattern, and the
spec stances (including the Unicode identifier approximation).

## Status

This crate is in initial development. The public API is not settled yet.

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
