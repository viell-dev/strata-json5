# strata-json5

`strata-json5` is a strict JSON5 parser for the Strata parser ecosystem, composed on top of
`strata-json` per that crate's documented superset pattern. It turns a byte stream into a
lossless, zero-copy concrete syntax tree (CST): tokens keep absolute byte spans, trivia
(whitespace runs, `//` and `/* */` comments, blank lines) is retained instead of discarded, and
string/number content stays as raw source bytes. Decoding — escape resolution, numeric
conversion, surrogate-pair combination — belongs to a downstream resolver or deserializer.

The language is JSON5 as specified at <https://spec.json5.org/>. Every JSON5 extension is a
mandatory language feature, not an option, and nothing beyond JSON5 is accepted (the spec's
permission to "accept non-JSON5 forms or extensions" is deliberately unexercised, per the
ecosystem's strict-by-default policy).

```text
reader -> lexer -> parser (this crate) -> resolver -> deserializer
```

- **Edition / MSRV:** Rust 2024, MSRV `1.87.0`.
- **License:** MPL-2.0.
- **Dependencies:** `strata-json` and `strata-lexer` only (transitively `strata-reader`,
  `strata-lease`).

## 1. Module and public-API map

| Module        | Public items |
|---------------|--------------|
| `rules`       | `Json5Token`, `WhitespaceRule`, `IdentifierRule`, `StringRule`, `NumberRule`, `LineCommentRule`, `BlockCommentRule`, `StrictRuleSet`, `strict_rule_set()`, `strict_scanner()`, `is_json5_whitespace()`; re-exports `JsonToken`, `PunctRule`, `LiteralRule`, `SharedRuleSet` from `strata-json` |
| `rules::ident`| `is_identifier_start()`, `is_identifier_continue()` (the Unicode approximation lives here) |
| `cst`         | `Document`, `Value`, `ArrayNode`, `Element`, `ObjectNode`, `Member`, `SyntaxToken`, `Trivia`, `TriviaKind`, `Warning`, `WarningKind` |
| `parse`       | `Json5Parser<M>`, `Json5ParserBuilder<M>`, `Json5Error`, `Json5ErrorKind`, `DEFAULT_MAX_DEPTH` |

Everything is re-exported at the crate root (except the `rules::ident` classifier functions).

## 2. Composition on strata-json

`Json5Token` wraps `JsonToken` where rules are reused: `Json5Token::Json(JsonToken)` plus the
JSON5-owned kinds `Identifier`, `Infinity`, `NaN`, `String`, `Number`, `Whitespace`,
`LineComment`, `BlockComment`, `Eof`.

- **Reused from `strata-json`, lifted with `map_kind`:** `PunctRule` (the six structural
  characters are identical) and `LiteralRule` (`true`/`false`/`null`, spelled exactly as in
  JSON). With the strict set these are the only reachable `Json(...)` values.
- **Replaced by one JSON5 rule (JSON's shape is a strict subset, so no layering):**
  - `WhitespaceRule` — JSON's four bytes ⊂ the JSON5 set (§4).
  - `StringRule` — every RFC 8259 string is a valid JSON5 string.
  - `NumberRule` — every RFC 8259 number is a valid JSON5 number.
- **Added:** `IdentifierRule`, `LineCommentRule`, `BlockCommentRule` (kept separate so dialects
  can pick either comment shape independently).
- **Dropped:** `BomRule` — U+FEFF is ordinary JSON5 whitespace anywhere in the input, so there
  is no BOM rule, no BOM policy, and no `allow_bom` option.

Two lexing choices worth knowing:

- `true`/`false`/`null` are also valid `IdentifierName`s, so the lifted `LiteralRule` and
  `IdentifierRule` tie on those exact words. `IdentifierRule` declares `priority() == -1`, so
  the literal wins the maximal-munch tie deterministically; longer words (`truex`) lex as
  identifiers by ordinary maximal munch.
- Bare `Infinity`/`NaN` lex through `IdentifierRule`, which classifies the exact raw words into
  the dedicated kinds `Json5Token::Infinity`/`NaN`. The number rule matches them only *after an
  explicit sign* (`+Infinity`, `-NaN`). Reason: they are simultaneously number values and valid
  member names; lexing the bare words as `Number` would force the parser to accept arbitrary
  numbers in name position. The parser accepts the word kinds in both positions.

`StrictRuleSet` is a named struct with a manual `MatcherSet` impl (fully monomorphic, `Clone` +
`Copy`) rather than a matcher tuple: the lexer's tuple impls stop at six elements and this
grammar has eight rules. Supersets of JSON5 compose exactly as this crate composes over JSON:
import the rule structs, lift with `MatcherExt::map_kind` into a wider kind (double-lifting the
reused JSON rules works transparently), add or omit rules, and feed a `MatcherVec` to
`SpecScanner::new(rules, Eof)`. `tests/composition.rs` is the worked example.

## 3. Token shapes (`rules`)

- `WhitespaceRule` — a run of: tab, LF, VT, FF, CR, space, NBSP, U+2028, U+2029, U+FEFF, and
  the Unicode `Zs` category (hardcoded small list: U+1680, U+2000–U+200A, U+202F, U+205F,
  U+3000). Line terminators are inter-token whitespace in JSON5.
- `IdentifierRule` — ES 5.1 `IdentifierName` with `\uXXXX` escapes validated per position
  (§6 for the Unicode approximation). Invalid escapes end the match, surfacing as an
  unrecognized byte at the backslash.
- `StringRule` — `'...'` or `"..."`. Escapes: `' " \ b f n r t v`, `\0` (no digit may follow),
  `\xHH`, `\uHHHH`, line continuations (`\` + LF/CR/CRLF/U+2028/U+2029), and any other
  non-digit character after `\` (ES 5.1 `NonEscapeCharacter`, covering JSON's `\/`). Unescaped
  LF/CR are lexical errors; unescaped U+2028/U+2029 and raw control characters are legal
  content. Lone surrogate escapes are lexically valid (resolver work), as in `strata-json`.
- `NumberRule` — optional `+`/`-`, then decimal (leading/trailing decimal point allowed, no
  leading zeros), `0x`/`0X` hex integer, or (signed only) `Infinity`/`NaN`. `08`, `1e`, `0x`
  are lexical errors at the offending byte; a bare sign or dot matches nothing. Hex floats are
  rejected at the parser: the token ends at `0x1` and the adjacent `.5` is a parse error.
- `LineCommentRule` — `//` to (excluding) a line terminator or EOF. `BlockCommentRule` —
  `/* ... */`, no nesting, unterminated is an error at the opening. Comment and string bodies
  are validated as UTF-8.

Lexical errors carry exact positions; inside multi-line tokens (strings with continuations,
block comments) positions are computed line-aware, mirroring the lexer core's byte-oriented
tracking (U+2028/U+2029 do not advance the diagnostic line number anywhere in the crate).

## 4. CST (`cst`)

The tree mirrors `strata-json`'s CST shape but is a distinct set of types, because upstream's
`SyntaxToken` is hardwired to `JsonToken` and its `TriviaKind` has no comment variants (making
those kind-generic upstream is a noted follow-up). Differences:

- `TriviaKind::{Whitespace, LineComment, BlockComment}` — comments are retained trivia attached
  to the following token; `Trivia::line_breaks()`/`blank_lines()` work as upstream.
- `Member::name` may be a `String`, `Identifier`, or word token (`Json(True/False/Null)`,
  `Infinity`, `NaN`); the kind records the written form.
- `Value::Number` covers finite numbers, hex, and `Infinity`/`NaN` (bare words keep their word
  kinds in the token).
- Trailing commas are retained verbatim: a last element/member with a trailing comma has
  `comma: Some(...)`.
- `Document` additionally carries `warnings()` (§5). There is no `bom()` accessor: a BOM is
  plain whitespace trivia.

Byte-for-byte reconstruction holds: concatenating every token's leading trivia and lexeme in
source order, plus `Document::trailing_trivia()`, reproduces the input exactly
(`tests/trivia_spans.rs` proves it over a kitchen-sink document).

## 5. Parser (`parse`) and spec stances

`Json5Parser::new()` / `default()` is strict JSON5 — and the only JSON5 there is. The builder
exposes `with_rules`, `with_lexer_config`, and `max_depth`; there are no feature toggles.

Spec latitude, and the stance taken on each:

- **Implementation limits** ("an implementation may set limits ... maximum depth of nesting"):
  `max_depth`, same default as `strata-json` (`DEFAULT_MAX_DEPTH` = 128, shared constant),
  counted per nested container and bounding recursion. No text-size/number/string limits beyond
  the lexer configuration's buffer capacities.
- **Duplicate names** ("the behavior of software that receives such an object is
  unpredictable"): the CST retains all members verbatim in source order — the fidelity goal —
  and duplicate policy belongs downstream. No option.
- **U+2028/U+2029 warnings** ("JSON5 parsers should produce a warning when they are found
  unescaped in strings"): the parse succeeds *and* a `Warning` with position and the offending
  character is recorded on `Document::warnings()`, in source order, escape-aware (a separator
  in a line continuation or written as `\u2028` does not warn). Always on; spec-mandated
  SHOULD, not an option.
- **Extensions** ("a JSON5 parser may accept non-JSON5 forms"): none accepted.
- **BOM**: whitespace per the spec's whitespace table, anywhere; no option (unlike JSON's
  `allow_bom`).

Errors are `Json5Error { position, kind }` with one-based line/column plus absolute byte,
mirroring `strata-json`: `Lex(LexError)`, `Expected`, `UnexpectedEof`, `TrailingContent`,
`DepthLimitExceeded` (no `UnexpectedBom` — the variant has no meaning in JSON5).

## 6. Unicode identifier approximation

ES 5.1 defines identifier characters by Unicode general category (`Lu Ll Lt Lm Lo Nl` to start;
plus `Mn Mc Nd Pc`, ZWNJ, ZWJ to continue). `std` does not expose general categories and the
crate adds no dependencies, so non-ASCII classification approximates; ASCII and `\uXXXX` escape
handling (including per-position validation of the escaped character) are exact. Deviations:

- **Start** uses `char::is_alphabetic` (`Alphabetic` = `Lu Ll Lt Lm Lo Nl` +
  `Other_Alphabetic`): accepts everything ES 5.1 accepts; over-accepts `Other_Alphabetic`
  (mostly combining marks, e.g. U+0903) that ES 5.1 allows only in continuation position.
- **Continue** uses `is_alphabetic || is_numeric`, plus a hardcoded `Pc` table and ZWNJ/ZWJ:
  over-accepts `No` (e.g. `²`); under-accepts `Mn`/`Mc` marks that are not `Other_Alphabetic`
  (e.g. combining U+0301 — precomposed `é` is fine).

The classifier is two functions (`rules::ident::is_identifier_start` / `is_identifier_continue`)
with the non-ASCII fallback isolated in two private functions, so an exact category table can
drop in without touching the rule.

## 7. Quality bar

Same as the ecosystem: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D
warnings` under the strict lint set in `Cargo.toml`, and `cargo test` all clean. Lint
exceptions use `#[expect(lint, reason = "…")]` on the smallest existing scope.

## 8. Limitations and follow-up

- No resolver: string unescaping (escapes, continuations, surrogate pairs), identifier-name
  decoding (`\u0061bc` vs `abc` equality), and number conversion (hex, `Infinity`/`NaN`) are
  future resolver/serde work.
- Unicode identifier classification is approximate outside ASCII (§6) pending an exact table.
- Upstream note: `strata-json`'s CST types would be reusable outright if they were generic
  over the token kind and trivia kind; today a superset crate must mirror them.
- The public API is not settled; this is an initial-development crate on an experimental
  branch.
