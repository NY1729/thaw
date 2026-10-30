//! `class X extends EventEmitter` (from `node:events`): the base is a JavaScript-realm class
//! that AOT classes cannot extend, so a module that subclasses it gets a native TypeScript
//! `EventEmitter` declared in place of the import (see `event_emitter_base.ts`). Modules that
//! never subclass it keep the real `node:events` class.

use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast::{
    Decl, Expr, ImportSpecifier, MemberProp, Module, ModuleDecl, ModuleExportName, ModuleItem, Pat,
    Stmt,
};

const PRELUDE: &str = include_str!("event_emitter_base.ts");

fn is_events_module(specifier: &str) -> bool {
    matches!(specifier, "events" | "node:events")
}

struct ExtendsFinder<'a> {
    name: &'a str,
    found: bool,
}

impl Visit for ExtendsFinder<'_> {
    fn visit_class(&mut self, class: &thaw_parser::ast::Class) {
        if matches!(class.super_class.as_deref(), Some(Expr::Ident(base)) if base.sym.as_ref() == self.name)
        {
            self.found = true;
        }
        class.visit_children_with(self);
    }
}

enum Binding {
    /// `import { EventEmitter } from "node:events"` / `import EventEmitter from "events"`.
    Import {
        item: usize,
        specifier: usize,
        local: String,
    },
    /// `const EventEmitter = events.EventEmitter` (a destructured default/namespace import).
    Member {
        item: usize,
        declarator: usize,
        local: String,
    },
}

impl Binding {
    fn local(&self) -> &str {
        match self {
            Binding::Import { local, .. } | Binding::Member { local, .. } => local,
        }
    }
}

/// `const X = D.EventEmitter` where `D` is a default/namespace import of `node:events`.
fn member_binding(module: &Module) -> Option<Binding> {
    let mut roots = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if import.type_only || !import.src.value.as_str().is_some_and(is_events_module) {
            continue;
        }
        for specifier in &import.specifiers {
            match specifier {
                ImportSpecifier::Default(default) => roots.push(default.local.sym.to_string()),
                ImportSpecifier::Namespace(namespace) => {
                    roots.push(namespace.local.sym.to_string())
                }
                _ => {}
            }
        }
    }
    // `const alias = events;` (top-level destructuring is normalized through such aliases).
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(vars))) = item else {
            continue;
        };
        for declarator in &vars.decls {
            if let (Pat::Ident(name), Some(Expr::Ident(root))) =
                (&declarator.name, declarator.init.as_deref())
            {
                if roots.iter().any(|known| known == root.sym.as_ref()) {
                    roots.push(name.id.sym.to_string());
                }
            }
        }
    }
    for (item_index, item) in module.body.iter().enumerate() {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Var(vars))) = item else {
            continue;
        };
        for (declarator_index, declarator) in vars.decls.iter().enumerate() {
            let (Pat::Ident(name), Some(init)) = (&declarator.name, declarator.init.as_deref())
            else {
                continue;
            };
            let Expr::Member(member) = init else { continue };
            let (Expr::Ident(root), MemberProp::Ident(property)) =
                (member.obj.as_ref(), &member.prop)
            else {
                continue;
            };
            if property.sym.as_ref() == "EventEmitter"
                && roots.iter().any(|known| known == root.sym.as_ref())
            {
                return Some(Binding::Member {
                    item: item_index,
                    declarator: declarator_index,
                    local: name.id.sym.to_string(),
                });
            }
        }
    }
    None
}

/// The local name an `EventEmitter` import from `node:events` binds, with its specifier position.
fn event_emitter_bindings(module: &Module) -> Vec<Binding> {
    let mut found = Vec::new();
    for (item_index, item) in module.body.iter().enumerate() {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if import.type_only || !import.src.value.as_str().is_some_and(is_events_module) {
            continue;
        }
        for (specifier_index, specifier) in import.specifiers.iter().enumerate() {
            match specifier {
                ImportSpecifier::Named(named) if !named.is_type_only => {
                    let imported = match &named.imported {
                        Some(ModuleExportName::Ident(name)) => name.sym.to_string(),
                        Some(ModuleExportName::Str(name)) => {
                            name.value.to_string_lossy().into_owned()
                        }
                        None => named.local.sym.to_string(),
                    };
                    if imported == "EventEmitter" {
                        found.push(Binding::Import {
                            item: item_index,
                            specifier: specifier_index,
                            local: named.local.sym.to_string(),
                        });
                    }
                }
                ImportSpecifier::Default(default) => {
                    found.push(Binding::Import {
                        item: item_index,
                        specifier: specifier_index,
                        local: default.local.sym.to_string(),
                    });
                }
                _ => {}
            }
        }
    }
    found.extend(member_binding(module));
    found
}

/// Replaces the `EventEmitter` import of `module` with a native class when the module extends it.
pub(crate) fn inline_event_emitter_base(module: &mut Module) -> Result<(), String> {
    let Some(binding) = event_emitter_bindings(module).into_iter().find(|binding| {
        let mut finder = ExtendsFinder {
            name: binding.local(),
            found: false,
        };
        module.visit_with(&mut finder);
        finder.found
    }) else {
        return Ok(());
    };
    let local = binding.local().to_string();
    match binding {
        Binding::Import {
            item, specifier, ..
        } => {
            let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = &mut module.body[item] else {
                unreachable!("binding found in an import")
            };
            import.specifiers.remove(specifier);
            if import.specifiers.is_empty() {
                module.body.remove(item);
            }
        }
        Binding::Member {
            item, declarator, ..
        } => {
            let ModuleItem::Stmt(Stmt::Decl(Decl::Var(vars))) = &mut module.body[item] else {
                unreachable!("binding found in a variable declaration")
            };
            vars.decls.remove(declarator);
            if vars.decls.is_empty() {
                module.body.remove(item);
            }
            // The normalized destructuring temporaries (`const __thaw_top_destructure_0 = events;`)
            // would now reference the removed import.
            module.body.retain(|item| {
                !matches!(item, ModuleItem::Stmt(Stmt::Decl(Decl::Var(vars)))
                    if vars.decls.iter().all(|declarator| matches!(&declarator.name, Pat::Ident(name)
                        if name.id.sym.starts_with("__thaw_top_destructure_")
                            && matches!(declarator.init.as_deref(), Some(Expr::Ident(_))))))
            });
        }
    }
    let source = PRELUDE.replace("EventEmitter", &local);
    let (prelude, _) = thaw_parser::parse_typescript_with_shared_source_map_named(
        &source,
        thaw_parser::common::FileName::Custom("thaw:node-events-base".into()),
    )?;
    module.body.splice(0..0, prelude.body);
    Ok(())
}
