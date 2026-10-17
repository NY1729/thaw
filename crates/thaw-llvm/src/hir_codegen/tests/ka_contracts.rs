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

#[test]
fn native_class_methods_preserve_returned_collection_aliases() {
 assert_eq!(compile_and_run(r##"const sharedMap = new Map<string,number>();
const sharedSet = new Set<number>();
class Store {
  calls = 0;
  getMap(): Map<string,number> { this.calls += 1; return sharedMap; }
  getSet(): Set<number> { return sharedSet; }
  fresh(): Map<string,number> { return new Map<string,number>(); }
}
async function main():Promise<void> {
 const s=new Store(); const a:any=s;
 const m:any=a.getMap(); const t:any=a.getSet();
 m.set("a",1); t.add(2);
 console.log(sharedMap.get("a"),sharedSet.has(2),m===a.getMap(),t===a.getSet(),s.calls);
 sharedMap.set("b",3);sharedSet.add(4); console.log(m.get("b"),t.has(4));
 await new Promise<void>((resolve)=>setTimeout(resolve,1));m.set("c",5);console.log(sharedMap.get("c"));
 const x:any=a.fresh(); const y:any=a.fresh();x.set("x",7);console.log(x===y,y.has("x"));
 for(let i=0;i<40;i++){await new Promise<void>((resolve)=>setTimeout(resolve,1));}console.log(x.get("x"));
}"##,"native_class_methods_preserve_returned_collection_aliases"),"1 true true true 2\n3 true\n5\nfalse false\n7\n");
}

#[test]
fn native_any_collection_keys_preserve_identity_through_host_aliases() {
 assert_eq!(compile_and_run(r##"async function main():Promise<void> {
 const key:any={n:1}; const distinct:any={n:1}; const arr:any=[1];
 const m=new Map<any,number>();m.set(key,1);m.set(distinct,2);m.set(arr,3);
 const a:any=m;const it:any=a.keys();const got:any=it.next().value;
 console.log(got===key,a.get(key),a.get(got),a.get(distinct),a.get(arr));
 a.set(got,4);console.log(m.get(key),m.get(distinct),m.size);
 for(let i=0;i<40;i++){m.set("k"+i,i);}
 console.log(a.get(got),m.get(key),a.has(key),a.has(arr));
 console.log(a.delete(got),m.has(key),m.has(distinct));a.set(key,5);console.log(m.get(key));
 const si=new Set<any>();si.add(key);si.add(arr);const s:any=si;console.log(s.has(got),s.has(arr));s.delete(got);console.log(si.has(key));s.add(got);console.log(si.has(key));
 for(let i=0;i<40;i++){await new Promise<void>((resolve)=>setTimeout(resolve,1));}console.log(a.get(got),a.get(arr));
 const box:any={key:{n:2}};a.set(box.key,8);console.log(m.get(box.key),a.get(box.key));
 const p=new Map<any,number>();p.set(NaN,1);p.set(-0,2);p.set("x",3);p.set(true,4);p.set(null,5);p.set(undefined,6);p.set(1n,7);const q:any=p;
 console.log(q.get(NaN),q.get(0),q.get("x"),q.get(true),q.get(null),q.get(undefined),q.get(1n));
}"##,"native_any_collection_keys_preserve_identity_through_host_aliases"),"true 1 1 2 3\n4 2 3\n4 4 true true\ntrue false true\n5\ntrue true\nfalse\ntrue\n5 3\n8 8\n1 2 3 4 5 6 7\n");
}

#[test]
fn native_iterator_helpers_preserve_lazy_state_and_references() {
    assert_eq!(compile_and_run(r###"function make():any {const m=new Map<string,number>();m.set("a",1);m.set("b",2);const a:any=m;return a.values().map((v:any)=>v*2).take(2);}
async function main():Promise<void> {
 const m=new Map<number,number>();m.set(1,1);m.set(2,2);m.set(3,3);const a:any=m;let calls=0;
 const it:any=a.values().map((v:any)=>{calls+=1;return v*2;}).filter((v:any)=>v>2).drop(1).take(2).flatMap((v:any)=>[v,v+1]);
 console.log(calls);m.set(3,30);m.set(4,4);console.log(it.next().value,calls);m.set(5,5);console.log([...it].join(","),calls,it.next().done);
 const key:any={n:1};const other:any={n:2};const refs=new Map<any,number>();refs.set(key,1);refs.set(other,2);const b:any=refs;
 const found:any=b.keys().find((k:any)=>k.n===2);console.log(found===other);
 const reduced:any=b.keys().reduce((acc:any,k:any)=>k);console.log(reduced===other);
 const arr:any=b.keys().toArray();console.log(arr.length,arr[0]===key,arr[1]===other);
 const bad:any=a.values().map((v:any)=>{throw new Error("helper failed");});try {bad.next();}catch(e:any){console.log(e.message);}console.log(bad.next().done);
 const retained:any=make();for(let i=0;i<40;i++){await new Promise<void>((resolve)=>setTimeout(resolve,1));}console.log([...retained].join(","),retained.next().done);
}"###, "native_iterator_helpers_preserve_lazy_state_and_references"), "0\n60 3\n61,8,9 4 true\ntrue\ntrue\n2 true true\nhelper failed\ntrue\n2,4 true\n");
}

#[test]
fn host_iterator_roots_survive_chained_helpers() {
    assert_eq!(compile_and_run(r###"function main():void {const arr:JsValue=[1,2,3];const a:any=arr.values().map((v:any)=>v*2).drop(1).take(1);console.log(a.next().value,a.next().done);const box:any={m:new Map<string,number>(),s:new Set<number>([1,2])};const m:JsValue=box.m;m.set("a",1);m.set("b",2);const keys:any=m.keys().map((v:any)=>v+"!");console.log(keys.next().value,[...keys].join(","));const set:JsValue=box.s;const entries:any=set.entries().flatMap((e:any)=>e);console.log([...entries].join(","));}"###, "host_iterator_roots_survive_chained_helpers"), "4 true\na! b!\n1,1,2,2\n");
}

#[test]
fn native_class_getters_preserve_collection_identity() {
 assert_eq!(compile_and_run(r###"const sharedMap = new Map<string,number>();
const sharedSet = new Set<number>();
class Store {
  get map(): Map<string,number> { return sharedMap; }
  get set(): Set<number> { return sharedSet; }
  calls = 0;
  getMap(): Map<string,number> { this.calls += 1; return this.map; }
  getSet(): Set<number> { return this.set; }
  fresh(): Map<string,number> { return new Map<string,number>(); }
}
async function main():Promise<void> {
 const s=new Store(); const a:any=s;
 const m:any=a.getMap(); const t:any=a.getSet();
 m.set("a",1); t.add(2);
 console.log(s.map.get("a"),s.set.has(2),m===a.getMap(),t===a.getSet(),m===a.map,t===a.set,s.calls);
 s.map.set("b",3);s.set.add(4); console.log(m.get("b"),t.has(4));
 await new Promise<void>((resolve)=>setTimeout(resolve,1));m.set("c",5);console.log(s.map.get("c"));
 const x:any=a.fresh(); const y:any=a.fresh();x.set("x",7);console.log(x===y,y.has("x"));
}"###,"native_class_getters_preserve_collection_identity"),"1 true true true true true 2\n3 true\n5\nfalse false\n");
}

#[test]
fn native_class_collection_fields_remain_live() {
 assert_eq!(compile_and_run(r###"class Store {
  map: Map<string,number> = new Map<string, number>();
  set: Set<number> = new Set<number>();
  calls = 0;
  getMap(): Map<string,number> { this.calls += 1; return this.map; }
  getSet(): Set<number> { return this.set; }
  fresh(): Map<string,number> { return new Map<string,number>(); }
}
async function main():Promise<void> {
 const s=new Store(); const a:any=s;
 const m:any=a.getMap(); const t:any=a.getSet();
 m.set("a",1); t.add(2);
 console.log(s.map.get("a"),s.set.has(2),m===a.getMap(),t===a.getSet(),m===a.map,t===a.set,s.calls);
 s.map.set("b",3);s.set.add(4); console.log(m.get("b"),t.has(4));
 await new Promise<void>((resolve)=>setTimeout(resolve,1));m.set("c",5);console.log(s.map.get("c"));
 const x:any=a.fresh(); const y:any=a.fresh();x.set("x",7);console.log(x===y,y.has("x"));
}"###,"native_class_collection_fields_remain_live"),"1 true true true true true 2\n3 true\n5\nfalse false\n");
}

#[test]
fn native_collection_arguments_and_define_writes_remain_live() {
 assert_eq!(compile_and_run(r###"class Store {
 map:Map<string,number>=new Map<string,number>(); set:Set<number>=new Set<number>();
 replace(m:Map<string,number>,s:Set<number>):Map<string,number> { this.map=m;this.set=s;m.set("inside",7);s.add(3);return m; }
}
async function main():Promise<void>{
 const store=new Store();const a:any=store;const m=new Map<string,number>();const s=new Set<number>();const b:any=m;const c:any=s;
 const result:any=a.replace(b,c); console.log(store.map===m,store.set===s,result===b,a.map===b,a.set===c,m.get("inside"),s.has(3));
 const other=new Map<string,number>();const d:any=other;a.map=d;a.set=c;console.log(store.map===other,store.set===s);
 Object.defineProperty(a,"map",{value:b});console.log(store.map===m,a.map===b);
 for(let i=0;i<40;i++){await new Promise<void>((resolve)=>setTimeout(resolve,1));}a.map.set("kept",9);console.log(m.get("kept"));
}"###,"native_collection_arguments_and_define_writes_remain_live"),"true true true true true 7 true\ntrue true\ntrue true\n9\n");
}

#[test]
fn native_inherited_accessors_keep_receivers_and_descriptors() {
 assert_eq!(compile_and_run(r###"class Base { n=1;get value():number{return this.n;}set value(v:number){this.n=v;} }
class Derived extends Base { get twice():number{return this.n*2;} }
function main():void {const x=new Derived();const y=new Derived();y.n=5;const a:any=x;const b:any=y;console.log(a.value,a.twice);a.value=3;console.log(x.n,a.twice);const proto:any=Object.getPrototypeOf(a);const getter:any=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(proto),"value").get;console.log(getter.call(b));console.log(Object.keys(a).join(","),Object.prototype.hasOwnProperty.call(a,"value"));}"###,"native_inherited_accessors_keep_receivers_and_descriptors"),"1 2\n3 6\n5\nn false\n");
}

#[test]
fn native_inherited_accessors_keep_real_ancestor_overrides() {
 assert_eq!(compile_and_run(r###"class Base {n=2;get value():number{return this.n;}set value(v:number){this.n=v;}}
class Derived extends Base {ping():number{return 1;}}
function main():void { let seen=0;const ctor:any=Base;Object.defineProperty(ctor.prototype,"value",{get:()=>100,set:(v:number)=>{seen=v;},configurable:true});const x=new Derived();const a:any=x;console.log(a.value);a.value=4;console.log(x.n,a.value,a.ping(),seen); }"###,"native_inherited_accessors_keep_real_ancestor_overrides"),"100\n2 100 1 4\n");
}

#[test]
fn native_error_subclass_keeps_javascript_prototype() {
 assert_eq!(compile_and_run(r###"class Failure extends Error { constructor(){super("failure");} } function main():void {const cls:any=Failure;const e=new Failure();console.log(e instanceof Error,e.message,cls.prototype instanceof Error); }"###,"native_error_subclass_keeps_javascript_prototype"),"true failure true\n");
}

#[test]
fn native_collection_and_array_arguments_keep_escaped_owner() {
 assert_eq!(compile_and_run(r###"class Box {stored:Map<string,number>=new Map<string,number>();run(m:Map<string,number>,values:number[]):number {this.stored=m;m.set("sum",values[0]+values[1]);return m.size;}}
function make():any {const box=new Box();const a:any=box;const m=new Map<string,number>();const b:any=m;console.log(a.run(b,[2,3]),m.get("sum"));return a;}
async function main():Promise<void>{const a:any=make();for(let i=0;i<40;i++){await new Promise<void>((resolve)=>setTimeout(resolve,1));}console.log(a.stored.get("sum"));}"###,"native_collection_and_array_arguments_keep_escaped_owner"),"1 5\n5\n");
}

#[test]
fn native_collection_arguments_reject_incompatible_and_unbranded_values() {
 assert_eq!(compile_and_run(r###"class Store {calls=0;use(m:Map<string,number>,s:Set<number>):number {this.calls+=1;m.set("ok",1);s.add(2);return this.calls;}}
function main():void {const store=new Store();const a:any=store;const m=new Map<string,number>();const s=new Set<number>();const b:any=m;const c:any=s;
const wrong=new Map<string,string>();const wrongSet=new Set<string>();const d:any=wrong;const e:any=wrongSet;
try {a.use(d,c);console.log("accepted wrong map");}catch(error:any){console.log(error instanceof TypeError);}
try {a.use(b,e);console.log("accepted wrong set");}catch(error:any){console.log(error instanceof TypeError);}
const host:any=new Function("return new Map()")();try{a.use(host,c);console.log("accepted host");}catch(error:any){console.log(error instanceof TypeError);}
const fake:any={"__thaw_js_handle_id__":1};try{a.use(fake,c);console.log("accepted fake");}catch(error:any){console.log(error instanceof TypeError);}
console.log(store.calls,m.size,s.size);console.log(a.use(b,c),m.get("ok"),s.has(2));}"###,"native_collection_arguments_reject_incompatible_and_unbranded_values"),"true\ntrue\ntrue\ntrue\n0 0 0\n1 1 true\n");
}

#[test]
fn native_class_real_accessors_keep_collection_assignments() {
 assert_eq!(compile_and_run(r###"class Holder {map:Map<string,number>=new Map<string,number>();calls=0;get current():Map<string,number>{this.calls+=1;return this.map;}set current(v:Map<string,number>){this.calls+=1;this.map=v;}}
function main():void {const token:any=Holder;const h=new Holder();const a:any=h;const m=new Map<string,number>();const b:any=m;console.log(h.calls);a.current=b;console.log(a.current===b,h.map===m,h.calls);a.current.set("x",3);console.log(m.get("x"),h.calls);}"###,"native_class_real_accessors_keep_collection_assignments"),"0\ntrue true 2\n3 3\n");
}

#[test]
fn native_host_get_prototype_preserves_proxy_exceptions() {
 assert_eq!(compile_and_run(r###"function main():void {const proxy:any=new Function("return new Proxy({}, {getPrototypeOf(){throw new Error('prototype failed');}})")();try{Object.getPrototypeOf(proxy);console.log("accepted");}catch(e:any){console.log(e.message);} }"###,"native_host_get_prototype_preserves_proxy_exceptions"),"prototype failed\n");
}

#[test]
fn native_private_fields_and_accessors_keep_callback_layout() {
 assert_eq!(compile_and_run(r###"class Counter {#n=1;get value():number{return this.#n;}set value(v:number){this.#n=v;}read():number{return this.#n;}}function main():void {const c=new Counter();const a:any=c;console.log(a.value,a.read(),Object.keys(a).length);a.value=4;console.log(c.read(),a.value);}"###,"native_private_fields_and_accessors_keep_callback_layout"),"1 1 0\n4 4\n");
}

#[test]
fn ka_contract_dynamic_storage_1() {
    assert_eq!(compile_and_run(r##"function main(): void { const xs: {x: number}[] = []; const a: any = xs; const original: any = new Function("return {x: 1}")(); a[0] = original; console.log(a[0] === original, xs[0] === original); original.x = 7; console.log(xs[0].x); xs[0].x = 9; console.log(original.x); }"##, "ka_contract_dynamic_storage_1"), "true true\n7\n9\n");
}

#[test]
fn ka_contract_dynamic_storage_2() {
    assert_eq!(compile_and_run(r##"function identity(xs: {x: number}[]): {x: number}[] { return xs; } function main(): void { const xs: {x: number}[] = []; const holder = {xs: identity(xs)}; const read = () => holder.xs; const a: any = read(); const original: any = new Function("return {x: 1}")(); a[0] = original; original.x = 5; console.log(xs[0] === original, holder.xs[0] === original, read()[0].x); }"##, "ka_contract_dynamic_storage_2"), "true true 5\n");
}

#[test]
fn ka_contract_dynamic_storage_3() {
    assert_eq!(compile_and_run(r##"function main(): void { const xs: number[][] = []; const a: any = xs; a[0] = a; console.log(a[0] === a, xs[0] === xs, typeof xs[0]); }"##, "ka_contract_dynamic_storage_3"), "true true object\n");
}

#[test]
fn ka_contract_dynamic_storage_4() {
    assert_eq!(compile_and_run(r##"class C { x = 1; method(): number { return this.x; } } function main(): void { const xs: C[] = []; const a: any = xs; const plain: any = new Function("return {x: 7}")(); a[0] = plain; console.log(xs[0] === plain, xs[0] instanceof C, typeof xs[0].method); const real = new C(); a[1] = real; console.log(xs[1] === real, xs[1] instanceof C, xs[1].method()); }"##, "ka_contract_dynamic_storage_4"), "true false undefined\ntrue true 1\n");
}

#[test]
fn ka_contract_dynamic_class_storage_1() {
    assert_eq!(compile_and_run(r##"class Box { value: number = 1; read(): number { return this.value; } } function main(): void { const box = new Box(); const exposed: any = box; const original: any = new Function("return {x: 7}")(); exposed.value = original; console.log(box.read() === original, box.value === original, typeof box.value); }"##, "ka_contract_dynamic_class_storage_1"), "true true object\n");
}

#[test]
fn ka_contract_dynamic_class_storage_2() {
    assert_eq!(compile_and_run(r##"class Factory { last: number[] = []; fresh(): number[] { const result: number[] = [1]; this.last = result; return result; } optional(present: boolean): number[] | undefined { return present ? this.last : undefined; } pair(): [number[], number[]] { return [this.last, this.last]; } async delayed(): Promise<number[]> { await Promise.resolve(); return this.last; } } async function main(): Promise<void> { const factory = new Factory(); const exposed: any = factory; const result: any = exposed.fresh(); console.log(result === factory.last, exposed.optional(true) === result, exposed.optional(false) === undefined); const pair: any = exposed.pair(); console.log(pair[0] === result, pair[1] === result); const delayed: any = await exposed.delayed(); delayed[0] = 9; console.log(delayed === result, factory.last[0]); }"##, "ka_contract_dynamic_class_storage_2"), "true true true\ntrue true\ntrue 9\n");
}

#[test]
fn ka_contract_dynamic_class_storage_3() {
    assert_eq!(compile_and_run(r##"class Base { #secret: number = 3; read(): number { return this.#secret; } } class Derived extends Base { get doubled(): number { return this.read() * 2; } } function main(): void { const values: Base[] = []; const exposed: any = values; const real = new Derived(); const plain: any = new Function("return {}")(); exposed[0] = real; exposed[1] = plain; console.log(values[0] === real, values[0] instanceof Base, values[0] instanceof Derived, exposed[0].doubled, values[0].read()); try { exposed[0].read.call(plain); } catch (error) { console.log(error instanceof TypeError); } console.log(values[1] === plain, values[1] instanceof Base, typeof values[1].read); }"##, "ka_contract_dynamic_class_storage_3"), "true true true 6 3\ntrue\ntrue false undefined\n");
}

#[test]
fn ka_contract_dynamic_class_storage_4() {
    assert_eq!(compile_and_run(r##"function main(): void { const map = new Map<number, number>(); const set = new Set<number>(); const m: any = map; const s: any = set; const original: any = new Function("return {x: 7}")(); m.set("key", original); s.add(original); console.log(map.size, m.get("key") === original, s.has(original), set.size); original.x = 9; console.log(m.get("key").x); }"##, "ka_contract_dynamic_class_storage_4"), "1 true true 1\n9\n");
}

#[test]
fn ka_contract_dynamic_graph_1() {
    assert_eq!(compile_and_run(r##"function main(): void { const root: number[][] = []; const first: any = root; let cursor: any = first; for (let i = 0; i < 70; i++) { const next: any = []; cursor[0] = next; cursor = next; } cursor[0] = first; let read: any = root; for (let i = 0; i < 70; i++) read = read[0]; console.log(read === cursor, read[0] === root); const other: number[][] = []; const second: any = other; second[0] = first; first[1] = second; const identity: any = new Function("value", "return value"); console.log(first[1][0] === first, identity(first) === root); }"##, "ka_contract_dynamic_graph_1"), "true true\ntrue true\n");
}

#[test]
fn ka_contract_dynamic_graph_2() {
    assert_eq!(compile_and_run(r##"function main(): void { const root: {next: any} = {next: null}; let cursor: any = root; for (let i = 0; i < 70; i++) { const next: any = {next: null}; cursor.next = next; cursor = next; } cursor.next = root; cursor.value = 7; let read: any = root; for (let i = 0; i < 70; i++) read = read.next; console.log(read === cursor, read.next === root, read.value); }"##, "ka_contract_dynamic_graph_2"), "true true 7\n");
}

#[test]
fn ka_contract_dynamic_graph_3() {
    assert_eq!(compile_and_run(r##"async function main(): Promise<void> { const values: {x: number}[] = []; const exposed: any = values; const original: any = new Function("return {x: 1}")(); exposed[0] = original; const fail: any = new Function("value", "throw value"); for (let i = 0; i < 40; i++) { await Promise.resolve(); try { fail(original); } catch (error) { } original.x += 1; } console.log(values[0] === original, exposed[0] === original, values[0].x); }"##, "ka_contract_dynamic_graph_3"), "true true 41\n");
}

#[test]
fn ka_contract_dynamic_graph_4() {
    assert_eq!(compile_and_run(r##"async function main(): Promise<void> { const original: any = new Function("return {x: 1}")(); const sparse: any[] = [, original]; const exposed: any = sparse; console.log(0 in exposed, 1 in exposed, exposed[1] === original); const copied: any = [...exposed, await Promise.resolve(original)]; console.log(0 in copied, copied[1] === original, copied[2] === original); }"##, "ka_contract_dynamic_graph_4"), "false true true\ntrue true true\n");
}

#[test]
fn ka_contract_dynamic_graph_5() {
    assert_eq!(compile_and_run(r##"function make(xs: {x: number}[]): () => {x: number}[] { return () => xs; } function main(): void { const xs: {x: number}[] = []; const read = make(xs); const exposed: any = read(); const original: any = new Function("return {x: 7}")(); exposed[0] = original; console.log(xs[0] === original, read()[0] === original); }"##, "ka_contract_dynamic_graph_5"), "true true\n");
}

#[test]
fn ka_contract_dynamic_graph_6() {
    assert_eq!(compile_and_run(r##"class Plain { value: number = 1; } function main(): void { const source = new Plain(); const values: Plain[] = []; const exposed: any = values; exposed[0] = source; console.log(values[0] === source, values[0] instanceof Plain); const object: any = new Function("return {value: 2}")(); exposed[1] = object; console.log(values[1] === object, values[1] instanceof Plain); }"##, "ka_contract_dynamic_graph_6"), "true true\ntrue false\n");
}
