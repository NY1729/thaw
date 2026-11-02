// fx16 round-6 diff contracts; expected output recorded from Node 22.

#[test]
fn fx16_top_level_string_append_is_linear_and_keeps_aliases() {
    let source = r#"let s = "";
for (let i = 0; i < 300000; i++) { s += "x"; }
const t = s;
s += "y";
console.log(s.length, t.length, t === s.slice(0, -1));
"#;
    assert_eq!(
        compile_and_run(source, "fx16_top_level_string_append"),
        "300001 300000 true\n"
    );
}

#[test]
fn fx16_top_level_string_global_read_by_function_keeps_snapshots() {
    let source = r#"let g = "";
const seen: string[] = [];
function peek(): string { return g; }
for (let i = 0; i < 4; i++) { g += "x"; seen.push(peek()); }
console.log(seen.join(","), g);
"#;
    assert_eq!(
        compile_and_run(source, "fx16_top_level_global_fn_read"),
        "x,xx,xxx,xxxx xxxx\n"
    );
}

#[test]
fn fx16_top_level_string_global_read_by_arrow_keeps_snapshots() {
    let source = r#"let h = "";
const snap = () => h;
const keep: string[] = [];
for (let i = 0; i < 5; i++) { h += "y"; keep.push(snap()); }
console.log(keep.join(","), h);
"#;
    assert_eq!(
        compile_and_run(source, "fx16_top_level_global_arrow_read"),
        "y,yy,yyy,yyyy,yyyyy yyyyy\n"
    );
}

#[test]
fn fx16_realm_global_function_names_and_lengths_match_node() {
    let source = r#"function main(): void {
  const g: any = globalThis;
  console.log(["queueMicrotask", "structuredClone", "atob", "btoa", "setTimeout", "setInterval", "setImmediate", "clearTimeout", "clearInterval", "clearImmediate"].map((k) => k + ":" + g[k].name + "/" + g[k].length).join(" "));
  console.log(process.emitWarning.length, queueMicrotask.name);
}
"#;
    assert_eq!(
        compile_and_run(source, "fx16_realm_global_names"),
        "queueMicrotask:queueMicrotask/1 structuredClone:structuredClone/2 atob:atob/1 btoa:btoa/1 setTimeout:setTimeout/2 setInterval:setInterval/2 setImmediate:setImmediate/1 clearTimeout:clearTimeout/1 clearInterval:clearInterval/1 clearImmediate:clearImmediate/1\n4 queueMicrotask\n"
    );
}

#[test]
fn fx16_computed_key_object_methods_are_named_after_their_keys() {
    let source = r#"const k = "dyn";
const o = { [k + "1"]: 1, [`t${2}`]: 2, f() { return this; } };
console.log(o, o.f() === o);
console.log({ [k]() { return 1; }, y: 2 });
const q = { ["x" + 1]() { return 1; }, async am() { return 3; } };
console.log(q.x1.name, q.am.name);
"#;
    assert_eq!(
        compile_and_run(source, "fx16_computed_key_method_names"),
        "{ dyn1: 1, t2: 2, f: [Function: f] } true\n{ dyn: [Function: dyn], y: 2 }\nx1 am\n"
    );
}

#[test]
fn fx16_url_and_search_params_inspect_like_node() {
    let source = r#"function main(): void {
  const u = new URL("https://user:pw@example.com:8080/p/a/t/h?query=string&a=1&a=2#hash");
  console.log(u);
  console.log(new URLSearchParams("a=1&b=2"), new URLSearchParams(""));
  console.log(new URL("http://a.com/x?y=1"));
}
"#;
    assert_eq!(
        compile_and_run(source, "fx16_url_inspect"),
        "URL {\n  href: 'https://user:pw@example.com:8080/p/a/t/h?query=string&a=1&a=2#hash',\n  origin: 'https://example.com:8080',\n  protocol: 'https:',\n  username: 'user',\n  password: 'pw',\n  host: 'example.com:8080',\n  hostname: 'example.com',\n  port: '8080',\n  pathname: '/p/a/t/h',\n  search: '?query=string&a=1&a=2',\n  searchParams: URLSearchParams { 'query' => 'string', 'a' => '1', 'a' => '2' },\n  hash: '#hash'\n}\nURLSearchParams { 'a' => '1', 'b' => '2' } URLSearchParams {}\nURL {\n  href: 'http://a.com/x?y=1',\n  origin: 'http://a.com',\n  protocol: 'http:',\n  username: '',\n  password: '',\n  host: 'a.com',\n  hostname: 'a.com',\n  port: '',\n  pathname: '/x',\n  search: '?y=1',\n  searchParams: URLSearchParams { 'y' => '1' },\n  hash: ''\n}\n"
    );
}
