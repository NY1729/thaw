// Wave FA: `.raw` through an `any` tag parameter, and extracted methods. Expected output from node 22.

#[test]
fn fa_contract_any_tag_parameter_keeps_raw() {
    let source = r##"const seen: any[] = [];
function tag(s: any, ...v: any[]) { seen.push(s); return s.raw.length + ":" + s.raw[1] + ":" + s.length; }
function site() { return tag`a${1}\n${2}b`; }
async function main(): Promise<void> {
  console.log(site());
  console.log(site());
  console.log(seen[0] === seen[1], seen[0].raw === seen[1].raw);
  const plain: any = ["x", "y"];
  console.log(plain.raw === undefined);
}
"##;
    assert_eq!(
        compile_and_run(source, "fa_contract_any_tag_parameter_keeps_raw"),
        "3:\\n:3\n3:\\n:3\ntrue true\ntrue\n"
    );
}

#[test]
fn fa_contract_extracted_this_method_throws_when_called() {
    let source = r##"const o = { n: 1, inc() { this.n++; return this.n; }, pure() { return 7; } };
async function main(): Promise<void> {
  const g = o.pure;
  console.log(g());
  const f = o.inc;
  console.log("extracted", typeof f);
  try { f(); console.log("called"); } catch (e) { console.log("threw", (e as Error).name); }
  console.log(o.n, o.inc());
}
"##;
    assert_eq!(
        compile_and_run(source, "fa_contract_extracted_this_method_throws_when_called"),
        "7\nextracted function\nthrew TypeError\n1 2\n"
    );
}

#[test]
fn fa_contract_top_level_class_method_value_is_a_function() {
    let source = r##"class C { n = 1; bump() { this.n++; return this.n; } id() { return 5; } }
const c = new C();
const i = c.id;
const h = c.bump;
console.log(i());
try { console.log(h()); } catch (e) { console.log("threw", (e as Error).name); }
console.log(c.n, c.bump());
"##;
    assert_eq!(
        compile_and_run(source, "fa_contract_top_level_class_method_value_is_a_function"),
        "5\nthrew TypeError\n1 2\n"
    );
}
