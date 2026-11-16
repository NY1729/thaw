//! `URL` and `URLSearchParams` under `util.inspect` (`Symbol.for('nodejs.util.inspect.custom')`): the
//! console.log text, nested values and the depth/breakLength options match Node 22 (expectations recorded
//! from `node --experimental-strip-types` on the same sources; each output line is one `it.out` entry).

use super::*;

/// Run `src` and return the console.log lines joined with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => std::mem::take(&mut it.out).join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

#[test]
fn url_and_search_params_console_log_match_node() {
    assert_eq!(
        one(
            r#"console.log(new URL("https://user:pw@example.com:8080/p/a/t/h?query=string&a=1&a=2#hash"))"#
        ),
        [
            "URL {",
            "  href: 'https://user:pw@example.com:8080/p/a/t/h?query=string&a=1&a=2#hash',",
            "  origin: 'https://example.com:8080',",
            "  protocol: 'https:',",
            "  username: 'user',",
            "  password: 'pw',",
            "  host: 'example.com:8080',",
            "  hostname: 'example.com',",
            "  port: '8080',",
            "  pathname: '/p/a/t/h',",
            "  search: '?query=string&a=1&a=2',",
            "  searchParams: URLSearchParams { 'query' => 'string', 'a' => '1', 'a' => '2' },",
            "  hash: '#hash'",
            "}",
        ]
        .join("|")
    );
    assert_eq!(
        one(r#"console.log(new URLSearchParams("a=1&b=2"), new URLSearchParams(""))"#),
        "URLSearchParams { 'a' => '1', 'b' => '2' } URLSearchParams {}"
    );
    assert_eq!(
        one(r#"console.log(new URLSearchParams("a=it's&b=\"x\"&c=%0A"))"#),
        r#"URLSearchParams { 'a' => "it's", 'b' => '"x"', 'c' => '\n' }"#
    );
}

#[test]
fn search_params_nested_in_an_object_match_node() {
    assert_eq!(
        one(r#"console.log({ p: new URLSearchParams("a=1&b=2") })"#),
        "{ p: URLSearchParams { 'a' => '1', 'b' => '2' } }"
    );
}

#[test]
fn url_and_search_params_custom_inspect_honour_depth_and_break_length() {
    assert_eq!(
        one(r#"const u = new URL("http://a/b?x=1#h"), p = new URLSearchParams("a=1&b=2"), s = Symbol.for("nodejs.util.inspect.custom");
console.log(u[s](0, { depth: 0 }));
console.log(u[s](1, { depth: 1 }));
console.log(p[s](-1, { stylize: (t) => t }));
console.log(p[s](1, { breakLength: 10 }));
console.log(p[s](2, { breakLength: 80 }));"#),
        [
            "URL {|  href: 'http://a/b?x=1#h',|  origin: 'http://a',|  protocol: 'http:',|  username: '',|  password: '',|  host: 'a',|  hostname: 'a',|  port: '',|  pathname: '/b',|  search: '?x=1',|  searchParams: [Object],|  hash: '#h'|}",
            "URL {|  href: 'http://a/b?x=1#h',|  origin: 'http://a',|  protocol: 'http:',|  username: '',|  password: '',|  host: 'a',|  hostname: 'a',|  port: '',|  pathname: '/b',|  search: '?x=1',|  searchParams: URLSearchParams { 'x' => '1' },|  hash: '#h'|}",
            "[Object]",
            "URLSearchParams {|  'a' => '1',|  'b' => '2' }",
            "URLSearchParams { 'a' => '1', 'b' => '2' }",
        ]
        .join("|")
    );
}
