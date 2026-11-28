//! Gives every class its own private names.
//!
//! `#x` is stored under the key `@@priv:x`, so two classes of one hierarchy that both declare `#x`
//! (minified code reuses `#i`, `#e`, ... in a base class and its subclass) would share one slot. Each
//! class's declared private names get a per-class suffix before evaluation; references resolve
//! lexically to the innermost class that declares the name.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use swc_ecma_visit::{VisitMut, VisitMutWith};
use thaw_parser::ast as a;

/// Separates the source name from the class suffix in a renamed private name.
pub(crate) const SUFFIX_MARK: char = '\u{1}';

static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);

/// The private name as written in source (`#x`'s `x`).
pub(crate) fn source_name(name: &str) -> &str {
    name.split(SUFFIX_MARK).next().unwrap_or(name)
}

#[derive(Default)]
struct Uniquify {
    scopes: Vec<HashMap<String, String>>,
}

fn declared(class: &a::Class) -> Vec<String> {
    class
        .body
        .iter()
        .filter_map(|member| match member {
            a::ClassMember::PrivateMethod(m) => Some(m.key.name.to_string()),
            a::ClassMember::PrivateProp(p) => Some(p.key.name.to_string()),
            a::ClassMember::AutoAccessor(acc) => match &acc.key {
                a::Key::Private(p) => Some(p.name.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

impl VisitMut for Uniquify {
    fn visit_mut_class(&mut self, class: &mut a::Class) {
        // `extends <expr>` is evaluated outside the class's own private scope
        if let Some(super_class) = &mut class.super_class {
            super_class.visit_mut_with(self);
        }
        let names = declared(class);
        if names.is_empty() {
            let super_class = class.super_class.take();
            class.visit_mut_children_with(self);
            class.super_class = super_class;
            return;
        }
        let id = NEXT_CLASS.fetch_add(1, Ordering::Relaxed);
        self.scopes.push(
            names
                .into_iter()
                .map(|name| {
                    let renamed = format!("{name}{SUFFIX_MARK}{id}");
                    (name, renamed)
                })
                .collect(),
        );
        let super_class = class.super_class.take();
        class.visit_mut_children_with(self);
        class.super_class = super_class;
        self.scopes.pop();
    }

    fn visit_mut_private_name(&mut self, name: &mut a::PrivateName) {
        let current = name.name.to_string();
        if let Some(renamed) = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&current))
        {
            name.name = renamed.as_str().into();
        }
    }
}

pub(crate) fn uniquify_module(module: &mut a::Module) {
    module.visit_mut_with(&mut Uniquify::default());
}

pub(crate) fn uniquify_function(function: &mut a::Function) {
    function.visit_mut_with(&mut Uniquify::default());
}

#[cfg(test)]
mod tests {
    use crate::{Ctl, Interp};

    fn one(src: &str) -> String {
        let mut it = Interp::new();
        match it.run(src) {
            Ok(_) => it.out.join("|"),
            Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
            Err(_) => "stray completion".into(),
        }
    }

    // Node 22: `D.i D.x B.i B.x` for a base and a subclass that declare the same private names.
    #[test]
    fn a_subclass_private_name_does_not_shadow_the_base_class_one() {
        assert_eq!(
            one("class B { #i() { return 'B.i'; } #x = 'B.x'; base() { return this.#i() + this.#x; } }\n\
                 class D extends B { #i() { return 'D.i'; } #x = 'D.x'; der() { return this.#i() + this.#x; } }\n\
                 const d = new D(); console.log(d.der(), d.base(), (class { #m() { return 1; } static has(o) { return #m in o; } }).has({}));"),
            "D.iD.x B.iB.x false"
        );
    }

    #[test]
    fn nested_classes_resolve_private_names_lexically() {
        assert_eq!(
            one("class O { #a = 'oa'; make() { const o = this; return new (class { #b = 'ib'; get() { return o.#a + this.#b; } })().get(); } }\n\
                 console.log(new O().make());"),
            "oaib"
        );
    }
}
