// Ranged Buffer.toString, utf16le, and a Buffer built from an array literal in an `any` slot.
// Expected output from node 22.

#[test]
fn buffer_ranged_to_string_and_utf16le_match_node() {
    let source = r#"
function main(): void {
  const s = Buffer.from("hello world");
  console.log(s.toString("utf8", 2, 7), s.toString("hex", 0, 3), s.toString("utf8", 6), s.toString("base64", 1, 4), s.toString("utf8", 3, 100), "[" + s.toString("utf8", 5, 2) + "]");
  console.log(s.toString("utf16le").length, Buffer.from("hi", "utf16le"), Buffer.from("hi", "ucs2").toString("ucs2"), Buffer.from("\u{1F600}x", "utf16le"), Buffer.from("hé", "utf16le").toString("utf16le"));
  const a = Buffer.from([97, 98, 99]);
  const wide: any = Buffer.from([1, 2, 3]);
  console.log(a.toString(), JSON.stringify(wide));
}
"#;
    let output = compile_and_run(source, "buffer_ranged_to_string_and_utf16le_match_node");
    assert_eq!(
        output,
        "llo w 68656c world ZWxs lo world []\n5 <Buffer 68 00 69 00> hi <Buffer 3d d8 00 de 78 00> hé\nabc {\"type\":\"Buffer\",\"data\":[1,2,3]}\n"
    );
}
