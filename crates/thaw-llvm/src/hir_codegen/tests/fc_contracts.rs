// Wave FC: `.call/.apply/.bind` and array callbacks on extracted methods, JS values adopted into
// typed native arrays through an `any` view. Expected output from node 22.

#[test]
fn fc_contract_extracted_method_call_apply_bind() {
    let source = r##"const o = { base: 10, inc(n: number): number { return this.base + n; } };
async function main(): Promise<void> {
  const f = o.inc;
  try { f(1); console.log("no throw"); } catch (e) { console.log("threw", (e as Error).name); }
  console.log(f.call({ base: 5 }, 1));
  console.log(f.apply({ base: 6 }, [1]));
  const g = f.bind({ base: 7 });
  console.log(g(1));
  console.log(o.inc.call({ base: 100 }, 1));
  const o2 = { base: 100, inc: o.inc };
  console.log(o2.inc(1), o.inc(1));
}
"##;
    assert_eq!(
        compile_and_run(source, "fc_contract_extracted_method_call_apply_bind"),
        "threw TypeError\n6\n7\n8\n101\n101 11\n"
    );
}

#[test]
fn fc_contract_object_method_as_array_callback() {
    let source = r##"const o = {
  k: 3,
  scale(n: number): number { return n * this.k; },
  pure(n: number): number { return n + 1; },
};
async function main(): Promise<void> {
  console.log([1, 2].map(o.pure).join(","));
  try { console.log([1, 2].map(o.scale).join(",")); } catch (e) { console.log("threw", (e as Error).name); }
  console.log([1, 2].map(o.scale, { k: 4 }).join(","));
  console.log([1, 2].map(o.scale.bind(o)).join(","));
  console.log([1, 2, 3].filter(o.pure).length);
}
"##;
    assert_eq!(
        compile_and_run(source, "fc_contract_object_method_as_array_callback"),
        "2,3\nthrew TypeError\n4,8\n3,6\n3\n"
    );
}

#[test]
fn fc_contract_js_value_adopted_into_native_class_array() {
    let source = r##"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
async function main(): Promise<void> {
  const nat: C[] = [new C()];
  const v: any = nat;
  const j: any = JSON.parse('{"n": 41}');
  v.push(j);
  console.log(nat.length, nat[1].n);
  v[0] = { n: 7 };
  console.log(nat[0].n);
}
"##;
    assert_eq!(
        compile_and_run(source, "fc_contract_js_value_adopted_into_native_class_array"),
        "2 41\n7\n"
    );
}

#[test]
fn fc_contract_foreign_object_in_native_class_array_keeps_identity() {
    let source = r##"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
function main(): void {
  const nat: C[] = [];
  const v: any = nat;
  const o: any = { n: 7 };
  v[0] = o;
  console.log(nat[0] === o, nat[0] instanceof C, nat[0].n);
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "fc_contract_foreign_object_in_native_class_array_keeps_identity"
        ),
        "true false 7\n"
    );
}
