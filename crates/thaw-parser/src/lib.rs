//! Thin wrapper around SWC's TypeScript parser. This crate's only job is
//! "TS source text -> `swc_ecma_ast::Module`" -- HIR lowering from that AST
//! lives in `thaw-hir`, not here.

use swc_common::errors::Handler;
use swc_common::sync::Lrc;
use swc_common::{FileName, SourceMap};
use swc_ecma_ast::{Module, ModuleItem};
use swc_ecma_parser::{lexer::Lexer, EsSyntax, Parser, StringInput, Syntax, TsSyntax};

pub use swc_common as common;
pub use swc_ecma_ast as ast;

thread_local! {
    static SHARED_SOURCE_MAP: Lrc<SourceMap> = Default::default();
}

/// The thread's shared `SourceMap`: every module parsed with
/// [`parse_typescript_with_shared_source_map_named`] appends its file here, so such a `Span`
/// resolves to its own file (positions never collide across those modules).
// ponytail: sources stay alive until the thread ends; fine for one-shot compiles.
pub fn shared_source_map() -> Lrc<SourceMap> {
    SHARED_SOURCE_MAP.with(Lrc::clone)
}

/// `(file name, 1-based line, 1-based column)` of `span`'s start, or `None` for a synthetic span
/// (or one that was not parsed into the shared map; a span from a private parse may resolve to an
/// unrelated file, so only lower modules whose parts all went through the shared map).
pub fn span_location(span: swc_common::Span) -> Option<(String, u32, u32)> {
    if span.lo.0 == 0 {
        return None;
    }
    let cm = shared_source_map();
    // A span from a private parse can land mid-character here; refuse it instead of tripping SWC's assertion.
    let file = cm.try_lookup_source_file(span.lo).ok()??;
    if !file
        .src
        .is_char_boundary((span.lo - file.start_pos).0 as usize)
    {
        return None;
    }
    let loc = cm.try_lookup_char_pos(span.lo).ok()?;
    Some((
        loc.file.name.to_string(),
        loc.line as u32,
        loc.col.0 as u32 + 1,
    ))
}

/// Parses a TypeScript source string into an SWC `Module`.
///
/// Errors are rendered to stderr via SWC's diagnostic emitter and also
/// returned as a joined string, since callers won't have a `Handler` of
/// their own at this stage of the pipeline.
pub fn parse_typescript(source: &str) -> Result<Module, String> {
    parse_typescript_with_source_map(source).map(|(module, _)| module)
}

/// Same as [`parse_typescript`], but also returns the `SourceMap` the
/// module was parsed against -- see
/// [`parse_javascript_with_source_map`]'s doc comment for why a caller
/// would need this (thaw-cli's cross-package name-collision call-site
/// rewrite parses the user's own `.ts` source this way, since it's real
/// TypeScript, not plain JS).
pub fn parse_typescript_with_source_map(source: &str) -> Result<(Module, Lrc<SourceMap>), String> {
    parse_typescript_with_source_map_named(source, FileName::Custom("input.ts".into()))
}

/// Parse TypeScript while retaining its real or synthetic source name in diagnostics.
pub fn parse_typescript_with_source_map_named(
    source: &str,
    filename: FileName,
) -> Result<(Module, Lrc<SourceMap>), String> {
    parse_typescript_source(source, filename, false, false)
}

/// Like [`parse_typescript_with_source_map_named`], but the file joins the thread's
/// [`shared_source_map`], so its spans stay resolvable by [`span_location`] next to those of every
/// other module parsed this way. Only the module bundler uses it: other callers index sources by
/// raw `BytePos`, which assumes each parse starts at position 1.
pub fn parse_typescript_with_shared_source_map_named(
    source: &str,
    filename: FileName,
) -> Result<(Module, Lrc<SourceMap>), String> {
    parse_typescript_source(source, filename, false, true)
}

/// Parses ambient declarations with SWC's declaration-file grammar.
pub fn parse_declarations(source: &str) -> Result<Module, String> {
    parse_declarations_with_source_map(source).map(|(module, _)| module)
}

pub fn parse_declarations_with_source_map(
    source: &str,
) -> Result<(Module, Lrc<SourceMap>), String> {
    parse_declarations_with_source_map_named(source, FileName::Custom("input.ts".into()))
}

/// Parse declarations while retaining their real or synthetic source name.
pub fn parse_declarations_with_source_map_named(
    source: &str,
    filename: FileName,
) -> Result<(Module, Lrc<SourceMap>), String> {
    parse_typescript_source(source, filename, true, false)
}

/// Parsed declaration files by (name, text), so each `.d.ts` is parsed once per build instead of
/// once per bridge pass. Callers get a clone, so they stay free to mutate their copy.
type DeclarationCache = Vec<(FileName, String, Module, Lrc<SourceMap>)>;

thread_local! {
    static DECLARATION_CACHE: std::cell::RefCell<(usize, DeclarationCache)> = const { std::cell::RefCell::new((0, Vec::new())) };
}

/// Cache budget in source bytes; the cache is dropped wholesale when exceeded.
const DECLARATION_CACHE_BYTES: usize = 64 << 20;

fn parse_typescript_source(
    source: &str,
    filename: FileName,
    declarations: bool,
    shared: bool,
) -> Result<(Module, Lrc<SourceMap>), String> {
    if !declarations || shared {
        return parse_typescript_uncached(source, filename, declarations, shared);
    }
    let hit = DECLARATION_CACHE.with(|cache| {
        cache
            .borrow()
            .1
            .iter()
            .find(|(name, text, ..)| *name == filename && text == source)
            .map(|(_, _, module, cm)| (module.clone(), cm.clone()))
    });
    if let Some(hit) = hit {
        return Ok(hit);
    }
    let parsed = parse_typescript_uncached(source, filename.clone(), declarations, shared)?;
    DECLARATION_CACHE.with(|cache| {
        let (bytes, entries) = &mut *cache.borrow_mut();
        if *bytes + source.len() > DECLARATION_CACHE_BYTES {
            entries.clear();
            *bytes = 0;
        }
        *bytes += source.len();
        entries.push((
            filename,
            source.to_string(),
            parsed.0.clone(),
            parsed.1.clone(),
        ));
    });
    Ok(parsed)
}

fn parse_typescript_uncached(
    source: &str,
    filename: FileName,
    declarations: bool,
    shared: bool,
) -> Result<(Module, Lrc<SourceMap>), String> {
    let cm = if shared {
        shared_source_map()
    } else {
        Default::default()
    };
    let handler = Handler::with_emitter_writer(Box::new(std::io::stderr()), Some(cm.clone()));

    let fm = cm.new_source_file(Lrc::new(filename), source.to_string());

    let syntax = Syntax::Typescript(TsSyntax {
        tsx: false,
        decorators: true,
        dts: declarations,
        ..Default::default()
    });

    let lexer = Lexer::new(syntax, Default::default(), StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);

    let module = parser.parse_module().map_err(|err| {
        err.into_diagnostic(&handler).emit();
        "failed to parse TypeScript source".to_string()
    })?;
    let errors = parser.take_errors();
    // Published `.d.ts` files are third-party text: recoverable grammar complaints that only
    // matter to executable code (execa's `arguments?: readonly string[]` parameter name under
    // strict mode) must not reject an otherwise parseable declaration.
    if !declarations && !errors.is_empty() {
        for error in errors {
            error.into_diagnostic(&handler).emit();
        }
        return Err("failed to parse TypeScript source".into());
    }
    Ok((module, cm))
}

/// Same as [`parse_typescript`], but for plain JS/ES syntax (no TS-specific
/// grammar). Kept around for parity with `Syntax::Es` and future use; the
/// real pipeline is TS-only.
pub fn parse_javascript(source: &str) -> Result<Module, String> {
    parse_javascript_with_source_map(source).map(|(module, _)| module)
}

/// Same as [`parse_javascript`], but also returns the `SourceMap` the
/// module was parsed against. A caller that needs to extract a node's
/// original source text (e.g. thaw-registry's ESM-to-CommonJS rewrite,
/// which keeps untouched statements verbatim rather than re-printing the
/// whole AST) needs the `SourceMap` to turn a `Span` back into text via
/// `SourceMap::span_to_snippet` -- raw `BytePos` arithmetic against the
/// input string isn't safe to do by hand (SWC's spans aren't simply
/// 0-based offsets into the source).
pub fn parse_javascript_with_source_map(source: &str) -> Result<(Module, Lrc<SourceMap>), String> {
    parse_javascript_with_source_map_named(source, FileName::Custom("input.js".into()))
}

/// Parse JavaScript while retaining its real or synthetic source name.
pub fn parse_javascript_with_source_map_named(
    source: &str,
    filename: FileName,
) -> Result<(Module, Lrc<SourceMap>), String> {
    let cm: Lrc<SourceMap> = Default::default();
    let handler = Handler::with_emitter_writer(Box::new(std::io::stderr()), Some(cm.clone()));
    let fm = cm.new_source_file(Lrc::new(filename), source.to_string());

    let syntax = Syntax::Es(EsSyntax::default());
    let lexer = Lexer::new(syntax, Default::default(), StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);

    let module = parser.parse_module().map_err(|err| {
        err.into_diagnostic(&handler).emit();
        "failed to parse JavaScript source".to_string()
    })?;
    let errors = parser.take_errors();
    if !errors.is_empty() {
        for error in errors {
            error.into_diagnostic(&handler).emit();
        }
        return Err("failed to parse JavaScript source".into());
    }
    Ok((module, cm))
}

/// Parse JavaScript as a sloppy-mode *script* (`with`, legacy octals, ... allowed unless the
/// text opts into `"use strict"`), for `eval` / `new Function` source.
pub fn parse_javascript_script(source: &str) -> Result<Module, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let handler = Handler::with_emitter_writer(Box::new(std::io::stderr()), Some(cm.clone()));
    let fm = cm.new_source_file(
        Lrc::new(FileName::Custom("input.js".into())),
        source.to_string(),
    );
    let lexer = Lexer::new(
        Syntax::Es(EsSyntax::default()),
        Default::default(),
        StringInput::from(&*fm),
        None,
    );
    let mut parser = Parser::new_from(lexer);
    let script = parser.parse_script().map_err(|err| {
        err.into_diagnostic(&handler).emit();
        "failed to parse JavaScript source".to_string()
    })?;
    if parser.take_errors().is_empty() {
        Ok(Module {
            span: script.span,
            body: script.body.into_iter().map(ModuleItem::Stmt).collect(),
            shebang: script.shebang,
        })
    } else {
        Err("failed to parse JavaScript source".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swc_common::Spanned;
    use swc_ecma_ast::{Decl, ModuleItem, Stmt};

    #[test]
    fn declaration_parse_cache_returns_independent_equal_modules() {
        let source = "export declare function f(x: number): number;";
        let name = FileName::Custom("cached.d.ts".into());
        let (mut first, _) =
            parse_declarations_with_source_map_named(source, name.clone()).unwrap();
        let (second, _) = parse_declarations_with_source_map_named(source, name.clone()).unwrap();
        assert_eq!(first.body.len(), 1);
        assert_eq!(first, second);
        first.body.clear();
        let (third, _) = parse_declarations_with_source_map_named(source, name).unwrap();
        assert_eq!(third, second);
        let (other, _) = parse_declarations_with_source_map_named(
            "export declare const y: string;",
            FileName::Custom("cached.d.ts".into()),
        )
        .unwrap();
        assert_ne!(other, second);
    }

    #[test]
    fn named_parsers_record_source_map_filename() {
        let path = std::path::PathBuf::from("src/diagnostic.ts");
        let filename = FileName::Real(path);
        let (ts, ts_map) =
            parse_typescript_with_source_map_named("export const x: number = 1;", filename.clone())
                .unwrap();
        assert_eq!(
            ts_map
                .lookup_char_pos(ts.body[0].span().lo)
                .file
                .name
                .as_ref(),
            &filename
        );
        let (dts, dts_map) =
            parse_declarations_with_source_map_named("export const x: number;", filename.clone())
                .unwrap();
        assert_eq!(
            dts_map
                .lookup_char_pos(dts.body[0].span().lo)
                .file
                .name
                .as_ref(),
            &filename
        );
        let (js, js_map) =
            parse_javascript_with_source_map_named("export const x = 1;", filename.clone())
                .unwrap();
        assert_eq!(
            js_map
                .lookup_char_pos(js.body[0].span().lo)
                .file
                .name
                .as_ref(),
            &filename
        );
    }

    #[test]
    fn source_only_wrappers_preserve_historical_filenames() {
        let (ts, ts_map) = parse_typescript_with_source_map("export const x: number = 1;").unwrap();
        assert_eq!(
            ts_map
                .lookup_char_pos(ts.body[0].span().lo)
                .file
                .name
                .as_ref(),
            &FileName::Custom("input.ts".into())
        );
        let (dts, dts_map) = parse_declarations_with_source_map("export const x: number;").unwrap();
        assert_eq!(
            dts_map
                .lookup_char_pos(dts.body[0].span().lo)
                .file
                .name
                .as_ref(),
            &FileName::Custom("input.ts".into())
        );
        let (js, js_map) = parse_javascript_with_source_map("export const x = 1;").unwrap();
        assert_eq!(
            js_map
                .lookup_char_pos(js.body[0].span().lo)
                .file
                .name
                .as_ref(),
            &FileName::Custom("input.js".into())
        );
    }

    #[test]
    fn parses_a_typed_function_declaration() {
        let module =
            parse_typescript("function add(a: number, b: number): number { return a + b; }")
                .unwrap();

        assert_eq!(module.body.len(), 1);
        match &module.body[0] {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                assert_eq!(fn_decl.ident.sym.as_str(), "add");
                assert_eq!(fn_decl.function.params.len(), 2);
                assert!(fn_decl.function.return_type.is_some());
            }
            other => panic!("expected a function declaration, got {other:?}"),
        }
    }

    #[test]
    fn parses_string_and_number_literals() {
        let module = parse_typescript(r#"console.log("Hello, Thaw!"); const n = 42;"#).unwrap();
        assert_eq!(module.body.len(), 2);
    }

    #[test]
    fn reports_a_syntax_error() {
        assert!(parse_typescript("function (").is_err());
    }

    #[test]
    fn rejects_recovered_parse_errors() {
        assert!(parse_typescript("function main(): void { const x; }").is_err());
        assert!(parse_typescript("const x = 0123;").is_err());
        assert!(parse_javascript("export const x;").is_err());
        assert!(parse_declarations("export const x: number;").is_ok());
        assert!(parse_declarations("export const x: ;").is_err());
    }
}
