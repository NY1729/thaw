// Audit2 Node-API leftovers (N043/N044/N047). Expected output recorded from node 22
// (--experimental-strip-types); the `message` text of the DOMExceptions is not pinned.

#[test]
fn node_api_atob_btoa_throw_invalid_character_error() {
    let source = r##"function main(): void {
  const t = (f: () => unknown) => { try { console.log(f()); } catch (e: any) { console.log(e.name, e.code, e instanceof Error); } };
  t(() => atob("***"));
  t(() => atob("aGVsbG8"));
  t(() => atob(" aGVs bG8= "));
  t(() => atob("aGk=="));
  t(() => btoa("€"));
  t(() => btoa("é"));
}
"##;
    assert_eq!(
        compile_and_run(source, "node_api_atob_btoa_throw_invalid_character_error"),
        "InvalidCharacterError 5 true\nhello\nhello\nInvalidCharacterError 5 true\nInvalidCharacterError 5 true\n6Q==\n"
    );
}

#[test]
fn node_api_buffer_concat_empty_list_and_structured_clone_keeps_dates_and_rejects_functions() {
    let source = r##"function main(): void {
  const t = (f: () => unknown) => { try { console.log(f()); } catch (e: any) { console.log(e.name, e.code, e instanceof Error); } };
  console.log(Buffer.concat([]).length, Buffer.concat([], 3).length);
  const c = structuredClone({ d: new Date(5), n: 1 });
  console.log(c, c.d instanceof Date, c.d.getTime());
  t(() => structuredClone(() => 1));
  t(() => structuredClone({ f: () => 1 }));
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "node_api_buffer_concat_empty_list_and_structured_clone_keeps_dates_and_rejects_functions"
        ),
        "0 0\n{ d: 1970-01-01T00:00:00.005Z, n: 1 } true 5\nDataCloneError 25 true\nDataCloneError 25 true\n"
    );
}
