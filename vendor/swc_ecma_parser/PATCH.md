# Vendored swc_ecma_parser 45.1.2 (patched)

Upstream: crates.io `swc_ecma_parser` 45.1.2, wired in via `[patch.crates-io]` in the workspace
`Cargo.toml`. Only `Cargo.toml` (examples removed) and these source edits differ from upstream:

1. `src/parser/stmt.rs`: new `parse_if_clause`. In sloppy mode a plain `function` declaration as
   an `if`/`else` body is parsed and wrapped in a block (ECMA-262 Annex B.3.4). Strict mode,
   generators, async functions and other loop/label bodies keep the upstream error.
2. `src/lexer/mod.rs` (`read_keyword_with`): an escaped keyword (`bre\u0061k`) is lexed as an
   identifier (cooked text) instead of a hard lexer error, so it works as an IdentifierName (`o.break`, `{ if: 1 }`).
3. `src/parser/ident.rs` (`check_escaped_reserved`, called from `parse_ident`, `parse_binding_ident`, and
   the primary-expression identifier path in `expr.rs`) + `src/lexer/token.rs` (`Token::keyword_from_str`): an
   identifier token whose text is a reserved word in the current context is rejected with
   `EscapeInReservedWord`, so it stays illegal as a reference/binding.

Rebase: copy a new upstream `src/` over this directory (keep `Cargo.toml` minus its `[[example]]`
tables), re-apply the three edits (grep for `thaw patch`), bump the version in the workspace
`Cargo.toml` requirement if needed, and run `cargo test -p thaw-js --lib swc_parser_gap`.
