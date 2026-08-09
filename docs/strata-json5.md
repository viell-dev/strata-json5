# strata-json5

`strata-json5` is the JSON5 resolver for the Strata parser ecosystem — the plain, consumer-facing
crate of the JSON5 stack. It drives `strata-json5-parser` and turns the zero-copy CST into a
read-only, owned document model with JSON5 value semantics: strings unescaped (single or double
quoted, line continuations, `\x` escapes, surrogate pairs), numbers converted integer-preserving
(hex included) with `Infinity`/`NaN` support, and member names decoded uniformly across their
three spellings — while source fidelity stays available on every node and the parser's
spec-mandated warnings surface on the resolved document.

The crate sits at the resolver stage of the per-language stack:

```text
strata-json5-rules -> strata-json5-parser -> strata-json5 (this crate)
```

- **Edition / MSRV:** Rust 2024, MSRV `1.87.0`.
- **License:** MPL-2.0.
- **Dependencies:** `strata-json5-parser`, `strata-lexer`.

## 1. Module and public-API map

| Module     | Public items |
|------------|--------------|
| `resolve`  | `Json5Resolver<M>`, `Json5ResolverBuilder<M>`, `DuplicateKeys` |
| `value`    | `Json5Value`, `Json5Array`, `Json5Object`, `Json5Member`, `Json5Number`, `NumberFormat`, `Meta` |
| `document` | `Json5Document` |
| `error`    | `Json5Error`, `Json5ErrorKind` |

Everything is re-exported at the crate root, plus `NameStyle`, `LoneSurrogates`,
`Warning`/`WarningKind`, and the parser's `Trivia`/`TriviaKind`.

## 2. The resolver (`resolve`)

`Json5Resolver::new()` / `default()` is the strict path — and for JSON5 that already includes
every extension, because the spec mandates them all: comments, unquoted and single-quoted names,
trailing commas, hex numbers, `Infinity`/`NaN`, and U+FEFF as whitespace are not options.
Accordingly the builder exposes only `max_depth`, `with_rules` (kind `Json5Token`), and
`with_lexer_config` as parser passthroughs, plus the two resolution policies:

- `duplicate_keys(DuplicateKeys)` — `Retain` (default) or `Error` at the repeated name.
  Duplicate detection compares **decoded** names, so `{a: 1, "a": 2}` is a duplicate.
- `lone_surrogates(LoneSurrogates)` — `Error` (default) or `Replace` with U+FFFD.

`resolve(impl Read)`, `resolve_str`, `resolve_bytes`, and `resolve_document` mirror the JSON
resolver; a document whose retained source was reclaimed first yields `SourceUnavailable`.

## 3. The model (`value`, `document`)

- `Json5Value` — the same shape as the JSON resolver's model. `Json5Number` is the parser
  `decode::Number` re-exported: hex integers stay integer-preserving (`0x1F` → `U64(31)`,
  `-0x10` → `I64(-16)`, magnitudes beyond 64 bits round to `f64`); `Infinity`/`NaN` and their
  signed forms are `F64`; `-0` and `-0x0` keep their sign. The written form rides along as the
  parser's `NumberFormat` (`Integer`, `Decimal`, `Exponential`, `Hex`, `Infinity`, `NaN`),
  exposed by `Json5Value::number_format()`.
- `Json5Member` — the decoded name (identifier `\uXXXX` escapes resolved, so `{"a":1}`,
  `{a:1}`, and `{a:1}` all expose `"a"`), plus `name_style()` (`DoubleQuoted`, `SingleQuoted`,
  or `Identifier`) and `name_meta()` for the raw spelling. `Json5Object::get` resolves to the
  last occurrence of a decoded name; `get_all`/`members`/`duplicates()` expose full fidelity.
- `Meta` — span, positions, leading trivia, and `comments()` filtering the leading pieces.
  `Json5Document::comment_text(&Trivia)` strips `//` or `/* */` delimiters.
- `Json5Document` — root, trailing trivia, retained `SourceStore` (`bytes`, `source_ref`,
  `source_mut`), and `warnings()`: the parser's spec-mandated warnings (unescaped U+2028/U+2029
  in strings parse successfully but SHOULD warn) ride along on the resolved document;
  `into_parts()` returns them with the tree and store.

## 4. Errors

`Json5Error { position, kind }` with `Json5ErrorKind`: `Parse(parser error)`,
`LoneSurrogate { unit }`, `DuplicateMember { name }`, `InvalidLexeme` (decode failure, only
reachable with replaced rule sets), and `SourceUnavailable { span }`. Decode-failure positions
carry an exact byte offset; the line/column may understate for multi-line string lexemes, which
strict JSON5 can produce (line continuations, raw U+2028/U+2029) — unlike JSON, where lexemes
are single-line.

## 5. Design choices and open decisions

- **Read-only model**, like the sibling resolvers; editing stays at the CST/source level.
- **Name-style fidelity without name-model complexity.** The decoded name is the identity
  (lookups, duplicate detection); the spelling survives as `NameStyle` plus the raw bytes behind
  `name_meta()`.
- **Warnings surface, not errors.** The spec mandates accepting unescaped LS/PS and warning;
  the resolver forwards the parser's warnings untouched.
- **Last-occurrence `get`**, hex-overflow rounding, and `-0` sign preservation are shared,
  maintainer-unratified decisions of the resolver family, inherited from the shared decode
  layer and documented in each crate.

## 6. Quality bar

Same as the ecosystem: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D
warnings` under the strict lint set in `Cargo.toml`, and `cargo test` all clean. Lint exceptions
use `#[expect(lint, reason = "…")]` on the smallest existing scope.

## 7. Limitations and follow-up

- No serde glue: deserialization onto user structs is the planned `strata-json5-serde` crate.
- `Json5Object::get` is a linear scan, as in the sibling resolvers.
- The json5 parser's `TriviaKind` exposes comment classification only through the generic
  `TriviaClass` trait (JSONC's has an inherent `is_comment()` too); `Meta::comments` matches on
  the enum directly. Worth aligning when the API settles.
- The public API is not settled; this is an initial-development crate on an experimental branch.
- Package verification (`cargo package`) is blocked until the path dependencies are published.
