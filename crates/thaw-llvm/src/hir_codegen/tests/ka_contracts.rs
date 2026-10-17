// Wave KA: a native Map/Set that becomes `any` is a live view. Expected output from node 22.

#[test]
fn ka_contract_native_map_and_set_alias_through_any() {
    let source = r##"async function main(): Promise<void> {
  const m = new Map<string, number>();
  m.set("a", 1);
  const a: any = m;
  console.log(a === m, m === a, a.size, a.get("a"), a.has("a"), a.has("z"));
  a.set("b", 2);
  console.log(m.size, m.get("b"));
  m.set("c", 3);
  console.log(a.size, a.get("c"));
  console.log(a.delete("a"), m.has("a"), a.delete("zz"));
  console.log([...a].length, [...a.keys()].join(","), [...a.values()].join(","));
  const seen: string[] = [];
  a.forEach((v: any, k: any) => { seen.push(k + "=" + v); });
  console.log(seen.join(","));
  for (const e of a) { console.log(e[0], e[1]); }
  console.log(a instanceof Map, String(a), JSON.stringify(a));
  console.log(a);
  a.clear();
  console.log(m.size, a.size);
  const s = new Set<number>();
  s.add(1);
  const b: any = s;
  b.add(2);
  s.add(3);
  console.log(b === s, b.size, s.size, b.has(3), s.has(2), [...b].join(","));
  console.log(b.delete(1), s.has(1));
  console.log(b);
}
"##;
    assert_eq!(
        compile_and_run(source, "ka_contract_native_map_and_set_alias_through_any"),
        "true true 1 1 true false\n2 2\n3 3\ntrue false false\n2 b,c 2,3\nb=2,c=3\nb 2\nc 3\ntrue [object Map] {}\nMap(2) { 'b' => 2, 'c' => 3 }\n0 0\ntrue 3 3 true true 1,2,3\ntrue false\nSet(2) { 2, 3 }\n"
    );
}

#[test]
fn ka_contract_map_and_set_views_cross_literals_functions_and_value_types() {
    let source = r##"class P { n = 1; bump(): number { this.n += 1; return this.n; } }
function touch(x: any): void {
  x.set("viaFn", 9);
}
function give(m: Map<string, number>): any {
  return m;
}
async function main(): Promise<void> {
  const m = new Map<string, number>();
  m.set("a", 1);
  const o: any = { m, tag: "t" };
  o.m.set("b", 2);
  console.log(m.size, o.m.size, o.m === m, o.m.get("b"));
  m.set("c", 3);
  console.log(o.m.has("c"), o.tag);
  touch(m);
  console.log(m.get("viaFn"));
  const g = give(m);
  g.set("fromReturn", 4);
  console.log(m.get("fromReturn"), g === m);
  const s = new Set<string>(["x"]);
  const list: any = [s, m];
  list[0].add("y");
  list[1].delete("a");
  console.log(s.size, s.has("y"), m.has("a"), list.length, list[0] === s);
  const nums = new Map<number, string>();
  nums.set(1, "one");
  const n: any = nums;
  n.set(2, "two");
  console.log(nums.get(2), n.get(1), n.get(3), n.has(2), n.size);
  const arrs = new Map<string, number[]>();
  arrs.set("k", [1, 2]);
  const ar: any = arrs;
  ar.get("k").push(3);
  console.log(arrs.get("k")!.length, ar.get("k")[2], ar.size);
  const bools = new Map<string, boolean>();
  const bo: any = bools;
  bo.set("t", true);
  bo.set("f", false);
  console.log(bools.get("t"), bools.get("f"), bo.get("f"), bo.has("f"));
  const objs = new Map<string, P>();
  objs.set("p", new P());
  const ob: any = objs;
  console.log(ob.get("p").bump(), objs.get("p")!.n);
  console.log(o);
  console.log(JSON.stringify(o.m), [...n.entries()].map((e: any) => e.join(":")).join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "ka_contract_map_and_set_views_cross_literals_functions_and_value_types"),
        "2 2 true 2\ntrue t\n9\n4 true\n2 true false 2 true\ntwo one undefined true 2\n3 3 1\ntrue false false true\n2 2\n{\n  m: Map(4) { 'b' => 2, 'c' => 3, 'viaFn' => 9, 'fromReturn' => 4 },\n  tag: 't'\n}\n{} 1:one,2:two\n"
    );
}

// Thaw-specific: a typed native table cannot hold a key or value of another type, so a mismatched
// write through the view throws a TypeError (Node would store it). Reads of other types answer
// absent. The view survives arena resets between timer turns.
#[test]
fn ka_contract_map_view_rejects_mismatched_writes_and_survives_arena_resets() {
    let source = r##"async function main(): Promise<void> {
  const m = new Map<number, string>();
  const a: any = m;
  try { a.set("x", "y"); console.log("stored"); } catch (e) { console.log(e instanceof TypeError); }
  try { a.set(1, 2); console.log("stored"); } catch (e) { console.log(e instanceof TypeError); }
  console.log(a.has("x"), a.get("x"), a.delete("x"), m.size);
  const big = new Map<string, number>();
  const v: any = big;
  for (let i = 0; i < 40; i++) {
    v.set("k" + i, i);
    await new Promise<void>((resolve) => setTimeout(resolve, 1));
  }
  console.log(big.size, v.size, v.get("k39"), v.get("k0"));
}
"##;
    assert_eq!(
        compile_and_run(source, "ka_contract_map_view_rejects_mismatched_writes_and_survives_arena_resets"),
        "true\ntrue\nfalse undefined false 0\n40 40 39 0\n"
    );
}

#[test]
fn ka_contract_collection_replacer_keeps_custom_properties() {
    assert_eq!(
        compile_and_run(r#"function main(): void { const native = new Map<string, number>(); native.set("a", 1); const m: any = native; m.extra = 7; m.missing = undefined; m.inf = Infinity; console.log(JSON.stringify(m, ["size", "extra", "missing", "inf"])); const set = new Set<number>([1, 2]); const s: any = set; s.extra = "kept"; console.log(JSON.stringify(s, ["extra", "size"])); }"#, "ka_contract_collection_replacer_keeps_custom_properties"),
        "{\"size\":1,\"extra\":7,\"inf\":null}\n{\"extra\":\"kept\",\"size\":2}\n"
    );
}

#[test]
fn native_collection_views_iterate_live_across_mutations() {
    assert_eq!(compile_and_run(r##"async function main(): Promise<void> {
 const m = new Map<string, number>(); m.set("a", 1); m.set("b", 2); m.set("c", 3);
 const a:any = m; const keys:any = a.keys(); const values:any = a.values(); const entries:any = a.entries();
 console.log(keys.next().value, values.next().value, JSON.stringify(entries.next().value));
 m.delete("b"); m.set("c", 30); m.set("d", 4);
 console.log(keys.next().value, values.next().value, JSON.stringify(entries.next().value));
 m.clear(); m.set("a", 10);
 console.log(keys.next().value, values.next().value, JSON.stringify(entries.next().value));
 console.log(keys.next().done); m.set("e", 5); console.log(keys.next().done);
 console.log(values.next().value, JSON.stringify(entries.next().value));
 const seen:string[] = [];
 a.forEach((v:any,k:any) => { seen.push(k + "=" + v); if(k === "a") { m.delete("e"); m.set("f", 6); } });
 console.log(seen.join(","));
 const s = new Set<number>([1, 2, 3]); const b:any = s; const si:any = b.values(); const se:any = b.entries();
 console.log(si.next().value, JSON.stringify(se.next().value));
 s.delete(2); s.delete(1); s.add(1); s.add(4);
 console.log(si.next().value, JSON.stringify(se.next().value));
 console.log(si.next().value, JSON.stringify(se.next().value));
 s.clear(); s.add(5);
 await new Promise<void>((resolve) => setTimeout(resolve, 1));
 console.log(si.next().value, JSON.stringify(se.next().value));
 const seenSet:number[] = [];
 b.forEach((v:any) => { seenSet.push(v); if(v === 5) { s.add(6); } }); console.log(seenSet.join(","));
}"##, "native_collection_views_iterate_live_across_mutations"), "a 1 [\"a\",1]\nc 30 [\"c\",30]\na 10 [\"a\",10]\ntrue\ntrue\n5 [\"e\",5]\na=10,f=6\n1 [1,1]\n3 [3,3]\n1 [1,1]\n5 [5,5]\n5,6\n");
}

#[test]
fn native_collection_iterator_next_validates_and_uses_its_receiver() {
    assert_eq!(compile_and_run(r##"function main():void { const m=new Map<string,number>(); m.set("a",1); const a:any=m; const it:any=a.keys(); const next:any=it.next; try { next(); console.log("accepted"); } catch(e:any) { console.log(e instanceof TypeError); } console.log(it.next().value); const other:any=a.keys(); console.log(next.call(other).value); const real:any=new Map([["x",1]]); const ri:any=real.keys(); console.log(next.call(ri).value); }"##, "native_collection_iterator_next_validates_and_uses_its_receiver"), "true\na\na\nx\n");
}

#[test]
fn collection_iterator_outlives_receiver_across_arena_resets() {
    assert_eq!(compile_and_run(r##"function make():any { const m=new Map<string,number>(); m.set("kept",42); const a:any=m; return a.entries(); }
async function main():Promise<void> { const it:any=make(); for(let i=0;i<40;i++){ await new Promise<void>((resolve)=>setTimeout(resolve,1)); } console.log(JSON.stringify(it.next().value),it.next().done); }"##, "collection_iterator_outlives_receiver_across_arena_resets"), "[\"kept\",42] true\n");
}

#[test]
fn native_collection_iterator_spread() {
    assert_eq!(compile_and_run(r##"function main():void { const m = new Map<string,number>(); m.set("a",1); m.set("b",2); const a:any=m; console.log([...a.keys()].join(","), [...a.values()].join(","), [...a].length); }"##, "native_collection_iterator_spread"), "a,b 1,2 2\n");
}
