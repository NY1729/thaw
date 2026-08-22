//! Thin wrapper around SWC's TypeScript parser. This crate's only job is
//! "TS source text -> `swc_ecma_ast::Module`" -- HIR lowering from that AST
//! lives in `thaw-hir`, not here.

use swc_common::errors::Handler;
use swc_common::sync::Lrc;
use swc_common::{FileName, SourceMap};
use swc_ecma_ast::Module;
use swc_ecma_parser::{lexer::Lexer, EsSyntax, Parser, StringInput, Syntax, TsSyntax};

pub use swc_ecma_ast as ast;

/// Parses a TypeScript source string into an SWC `Module`.
///
/// Errors are rendered to stderr via SWC's diagnostic emitter and also
/// returned as a joined string, since callers won't have a `Handler` of
/// their own at this stage of the pipeline.
pub fn parse_typescript(source: &str) -> Result<Module, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let handler = Handler::with_emitter_writer(Box::new(std::io::stderr()), Some(cm.clone()));

    let fm = cm.new_source_file(Lrc::new(FileName::Custom("input.ts".into())), source.to_string());

    let syntax = Syntax::Typescript(TsSyntax {
        tsx: false,
        decorators: false,
        ..Default::default()
    });

    let lexer = Lexer::new(syntax, Default::default(), StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);

    for err in parser.take_errors() {
        err.into_diagnostic(&handler).emit();
    }

    parser
        .parse_module()
        .map_err(|err| {
            err.into_diagnostic(&handler).emit();
            "failed to parse TypeScript source".to_string()
        })
}

/// Same as [`parse_typescript`], but for plain JS/ES syntax (no TS-specific
/// grammar). Kept around for parity with `Syntax::Es` and future use; the
/// real pipeline is TS-only.
pub fn parse_javascript(source: &str) -> Result<Module, String> {
    let cm: Lrc<SourceMap> = Default::default();
    let handler = Handler::with_emitter_writer(Box::new(std::io::stderr()), Some(cm.clone()));
    let fm = cm.new_source_file(Lrc::new(FileName::Custom("input.js".into())), source.to_string());

    let syntax = Syntax::Es(EsSyntax::default());
    let lexer = Lexer::new(syntax, Default::default(), StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);

    for err in parser.take_errors() {
        err.into_diagnostic(&handler).emit();
    }

    parser.parse_module().map_err(|err| {
        err.into_diagnostic(&handler).emit();
        "failed to parse JavaScript source".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use swc_ecma_ast::{Decl, ModuleItem, Stmt};

    #[test]
    fn parses_a_typed_function_declaration() {
        let module = parse_typescript(
            "function add(a: number, b: number): number { return a + b; }",
        )
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
}
