//! Named-export link check of a native ES-module graph (ES `ResolveExport`): a static `import { x }` /
//! `export { x } from` whose target is an ES module that provides no `x` (or two different bindings through
//! `export *`) is a `SyntaxError` at link time, before any module body runs. Input is the registry's export
//! graph of each module (`local`, `indirect`, `stars`, `requests`); a target without a graph (CommonJS, a
//! builtin, a peer) or a star that cannot be followed is "may provide": never an error.

use serde_json::{Map, Value};
use std::collections::HashSet;

/// A named request that cannot be satisfied.
pub(crate) enum LinkError {
    Missing,
    Ambiguous,
}

/// The terminal binding of an export: the module `key` that declares `binding` (`*namespace*` for a namespace
/// export of `key`, which then has no `export` name), reached under the name `export` there.
#[derive(Clone)]
pub(crate) struct Bound {
    pub key: String,
    binding: String,
    pub export: Option<String>,
}

// two exports are one binding when they name the same declaration, however they are reached
impl PartialEq for Bound {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.binding == other.binding
    }
}

#[derive(PartialEq, Clone)]
enum Found {
    None,
    Ambiguous,
    /// Something this check cannot see (opaque module): assume it provides the name.
    Opaque,
    Bind(Bound),
}

struct Graphs<'a, F: Fn(&str, &str) -> Option<String>> {
    graphs: &'a Map<String, Value>,
    /// The module key `spec` of module `key` resolves to.
    target: F,
}

impl<F: Fn(&str, &str) -> Option<String>> Graphs<'_, F> {
    fn resolve(&self, key: &str, name: &str, visited: &mut HashSet<(String, String)>) -> Found {
        let Some(graph) = self.graphs.get(key) else {
            return Found::Opaque;
        };
        if !visited.insert((key.to_string(), name.to_string())) {
            return Found::None;
        }
        if let Some(binding) = graph.get("local").and_then(|l| l.get(name)) {
            return Found::Bind(Bound {
                key: key.to_string(),
                binding: binding.as_str().unwrap_or(name).to_string(),
                export: Some(name.to_string()),
            });
        }
        if let Some(Value::Array(entry)) = graph.get("indirect").and_then(|i| i.get(name)) {
            let Some(spec) = entry.first().and_then(Value::as_str) else {
                return Found::Opaque;
            };
            let Some(next) = (self.target)(key, spec) else {
                return Found::Opaque;
            };
            return match entry.get(1).and_then(Value::as_str) {
                None | Some("*") => Found::Bind(Bound {
                    key: next,
                    binding: "*namespace*".into(),
                    export: None,
                }),
                Some(remote) => self.resolve(&next, remote, visited),
            };
        }
        if name == "default" {
            return Found::None;
        }
        let (mut found, mut opaque) = (None, false);
        for spec in graph
            .get("stars")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let Some(next) = (self.target)(key, spec) else {
                opaque = true;
                continue;
            };
            match self.resolve(&next, name, visited) {
                Found::Ambiguous => return Found::Ambiguous,
                Found::Opaque => opaque = true,
                Found::None => {}
                bind => match &found {
                    Some(prior) if *prior != bind => return Found::Ambiguous,
                    _ => found = Some(bind),
                },
            }
        }
        // an opaque star may provide the name too (then it is ambiguous): only the runtime knows
        match found {
            _ if opaque => Found::Opaque,
            Some(bind) => bind,
            None => Found::None,
        }
    }
}

/// The names `export *` gives module `key` that resolve statically to one binding (`default`, its own exports,
/// ambiguous names and names behind an opaque module are left out), with what each resolves to.
pub(crate) fn star_bindings(
    graphs: &Map<String, Value>,
    key: &str,
    target: impl Fn(&str, &str) -> Option<String>,
) -> Vec<(String, Bound)> {
    let g = Graphs { graphs, target };
    // every name a star target exports, depth first (a cycle ends at a visited module)
    fn names(
        g: &Graphs<impl Fn(&str, &str) -> Option<String>>,
        key: &str,
        seen: &mut HashSet<String>,
        out: &mut Vec<String>,
    ) {
        let Some(graph) = g.graphs.get(key) else {
            return;
        };
        if !seen.insert(key.to_string()) {
            return;
        }
        for table in ["local", "indirect"] {
            for name in graph
                .get(table)
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|o| o.keys())
            {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
        }
        for spec in graph
            .get("stars")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(next) = (g.target)(key, spec) {
                names(g, &next, seen, out);
            }
        }
    }
    let Some(graph) = graphs.get(key) else {
        return Vec::new();
    };
    let own = |name: &str| {
        ["local", "indirect"]
            .iter()
            .any(|t| graph.get(*t).and_then(|o| o.get(name)).is_some())
    };
    let mut candidates = Vec::new();
    for spec in graph
        .get("stars")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if let Some(next) = (g.target)(key, spec) {
            names(&g, &next, &mut HashSet::new(), &mut candidates);
        }
    }
    let mut out = Vec::new();
    for name in candidates {
        if name == "default"
            || name == "__esModule"
            || own(&name)
            || out.iter().any(|(n, _)| *n == name)
        {
            continue;
        }
        if let Found::Bind(bound) = g.resolve(key, &name, &mut HashSet::new()) {
            out.push((name, bound));
        }
    }
    out
}

/// The first unsatisfiable named request of module `key`, as `(specifier, requested name, error)`.
pub(crate) fn check_requests(
    graphs: &Map<String, Value>,
    key: &str,
    target: impl Fn(&str, &str) -> Option<String>,
) -> Option<(String, String, LinkError)> {
    let g = Graphs { graphs, target };
    let requests = graphs.get(key)?.get("requests")?.as_array()?;
    for request in requests {
        let (Some(spec), Some(name)) = (
            request.get(0).and_then(Value::as_str),
            request.get(1).and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(next) = (g.target)(key, spec) else {
            continue;
        };
        let mut visited = HashSet::new();
        match g.resolve(&next, name, &mut visited) {
            Found::None => return Some((spec.into(), name.into(), LinkError::Missing)),
            Found::Ambiguous => return Some((spec.into(), name.into(), LinkError::Ambiguous)),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(graphs: &str, key: &str) -> Option<(String, String, bool)> {
        let Value::Object(graphs) = serde_json::from_str::<Value>(graphs).unwrap() else {
            panic!()
        };
        check_requests(&graphs, key, |_, spec| {
            Some(spec.trim_start_matches("./").into())
        })
        .map(|(s, n, e)| (s, n, matches!(e, LinkError::Ambiguous)))
    }

    #[test]
    fn missing_local_indirect_star_and_ambiguous_names() {
        let g = r#"{
          "a": {"requests": [["./b","x"],["./b","gone"]]},
          "b": {"local": {"x": "x"}, "indirect": {}, "stars": []}}"#;
        assert_eq!(run(g, "a"), Some(("./b".into(), "gone".into(), false)));
        // the same binding through two stars is one binding; two different ones are ambiguous
        let g = r#"{
          "a": {"requests": [["./m","d"]]},
          "m": {"local": {}, "indirect": {}, "stars": ["./l","./r"]},
          "l": {"local": {}, "indirect": {"d": ["./p","d"]}, "stars": []},
          "r": {"local": {}, "indirect": {"d": ["./p","d"]}, "stars": []},
          "p": {"local": {"d": "d"}, "indirect": {}, "stars": []}}"#;
        assert_eq!(run(g, "a"), None);
        let g = g.replace(
            r#""r": {"local": {}, "indirect": {"d": ["./p","d"]}"#,
            r#""r": {"local": {"d": "d"}, "indirect": {}"#,
        );
        assert_eq!(run(&g, "a"), Some(("./m".into(), "d".into(), true)));
    }

    #[test]
    fn opaque_targets_and_stars_may_provide_any_name() {
        let g = r#"{
          "a": {"requests": [["./cjs","x"],["./m","y"],["./m","default"]]},
          "m": {"local": {}, "indirect": {}, "stars": ["./cjs"]}}"#;
        // a CommonJS target is not checked; a star over one may provide `y`, but never `default`
        assert_eq!(run(g, "a"), Some(("./m".into(), "default".into(), false)));
    }

    #[test]
    fn star_bindings_resolve_through_reexports_to_one_terminal_binding() {
        let Value::Object(graphs) = serde_json::from_str::<Value>(
            r#"{
          "i": {"local": {}, "indirect": {}, "stars": ["./l","./r"]},
          "l": {"local": {}, "indirect": {"ns": ["./b", "ns"]}, "stars": []},
          "r": {"local": {}, "indirect": {"ns": ["./b", "ns"], "k": ["./p","k"]}, "stars": []},
          "b": {"local": {}, "indirect": {"ns": ["./p", null]}, "stars": []},
          "p": {"local": {"k": "k", "default": "x"}, "indirect": {}, "stars": []}}"#,
        )
        .unwrap() else {
            panic!()
        };
        let got = star_bindings(&graphs, "i", |_, s| Some(s.trim_start_matches("./").into()));
        let got: Vec<_> = got
            .iter()
            .map(|(n, b)| (n.as_str(), b.key.as_str(), b.export.as_deref()))
            .collect();
        // the diamond `ns` is one namespace binding of p; `k` is p's own; `default` is never a star name
        assert_eq!(got, vec![("ns", "p", None), ("k", "p", Some("k"))]);
    }

    #[test]
    fn a_name_beside_an_opaque_star_is_left_to_the_runtime() {
        let Value::Object(graphs) = serde_json::from_str::<Value>(
            r#"{
          "m": {"local": {}, "indirect": {}, "stars": ["./cjs","./e"]},
          "e": {"local": {"a": "a"}, "indirect": {}, "stars": []}}"#,
        )
        .unwrap() else {
            panic!()
        };
        // `a` may also come from the CommonJS star (then it is ambiguous): no static binding
        assert!(star_bindings(&graphs, "m", |_, s| (s == "./e").then(|| "e".into())).is_empty());
    }

    #[test]
    fn a_cycle_through_stars_ends_as_missing() {
        let g = r#"{
          "a": {"requests": [["./m","x"]]},
          "m": {"local": {}, "indirect": {}, "stars": ["./n"]},
          "n": {"local": {}, "indirect": {}, "stars": ["./m"]}}"#;
        assert_eq!(run(g, "a"), Some(("./m".into(), "x".into(), false)));
    }
}
