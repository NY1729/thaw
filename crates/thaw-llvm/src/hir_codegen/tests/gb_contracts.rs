// Wave GB: a Json array/object written through an `any` view into a typed native slot stays linked to
// that slot, so both sides observe one value. Expected output from node 22.

#[test]
fn gb_contract_json_array_written_through_view_stays_shared() {
    let source = r##"async function main(): Promise<void> {
  const nat: number[][] = [[1]];
  const v: any = nat;
  const inner: any = [5, 6];
  v[0] = inner;
  inner.push(7);
  console.log(nat[0].length, v[0].length, inner.length);
  inner[0] = 50;
  console.log(nat[0][0]);
  nat[0].push(9);
  console.log(inner.length, JSON.stringify(inner));
}
"##;
    assert_eq!(
        compile_and_run(source, "gb_contract_json_array_written_through_view_stays_shared"),
        "3 3 3\n50\n4 [50,6,7,9]\n"
    );
}

#[test]
fn gb_contract_json_arrays_of_primitives_stay_linked_both_ways() {
    let source = r##"async function main(): Promise<void> {
  const nat: number[][] = [[1]];
  const v: any = nat;
  const inner: any = [5, 6];
  v[0] = inner;
  inner.splice(0, 1);
  console.log(nat[0].length, nat[0][0]);
  inner.unshift(1, 2);
  console.log(nat[0].join(","));
  nat[0][1] = 99;
  console.log(inner[1], inner.length);
  nat[0].push(4);
  console.log(inner.join(","), v[0].length);
  inner.splice(2);
  console.log(nat[0].length);
  inner.fill(7);
  console.log(nat[0].join(","));

  const words: string[][] = [["a"]];
  const wv: any = words;
  const w: any = ["x", "y"];
  wv[0] = w;
  w.push("z");
  console.log(words[0].join("-"), words[0].length);
  words[0][0] = "q";
  console.log(w.join("-"));
  w[1] = "yy";
  console.log(words[0][1]);

  const flags: boolean[][] = [[true]];
  const fv: any = flags;
  const f: any = [false, true];
  fv[0] = f;
  f.push(false);
  console.log(flags[0].length, flags[0][0], flags[0][2]);
  flags[0][0] = true;
  console.log(f[0]);

  const parsed: number[][] = [];
  const pv: any = parsed;
  const j = JSON.parse("[1,2]");
  pv.push(j);
  j.push(3);
  console.log(parsed.length, parsed[0].length);

  const two: number[][] = [[0], [0]];
  const tv: any = two;
  const shared: any = [1];
  tv[0] = shared;
  tv[1] = shared;
  shared.push(2);
  console.log(two[0].length, two[1].length);

  await new Promise<void>((resolve) => setTimeout(resolve, 5));
  inner.push(8);
  console.log(nat[0].length, nat[0][nat[0].length - 1]);
  console.log(v[0] === inner, v[0] === v[0]);
}
"##;
    assert_eq!(
        compile_and_run(source, "gb_contract_json_arrays_of_primitives_stay_linked_both_ways"),
        "1 6\n1,2,6\n99 3\n1,99,6,4 4\n2\n7,7\nx-y-z 3\nq-y-z\nyy\n3 false false\ntrue\n1 3\n2 2\n3 8\ntrue true\n"
    );
}

#[test]
fn gb_contract_json_object_written_through_view_stays_shared() {
    let source = r##"interface P { n: number; s: string; ok: boolean }
async function main(): Promise<void> {
  const objs: P[] = [{ n: 1, s: "a", ok: false }];
  const v: any = objs;
  const j: any = { n: 2, s: "b", ok: true };
  v[0] = j;
  j.n = 9;
  j.s = "bb";
  j.ok = false;
  console.log(objs[0].n, objs[0].s, objs[0].ok);
  objs[0].n = 10;
  objs[0].s = "typed";
  console.log(j.n, j.s, JSON.stringify(j));
  v.push(j);
  j.n = 11;
  console.log(objs[1].n, objs[0].n, v[0] === j, v[1] === j);
  const parsed = JSON.parse('{"n":5,"s":"p","ok":true}');
  v[0] = parsed;
  parsed.n = 6;
  console.log(objs[0].n);
  await new Promise<void>((resolve) => setTimeout(resolve, 5));
  j.n = 12;
  console.log(objs[1].n);
}
"##;
    assert_eq!(
        compile_and_run(source, "gb_contract_json_object_written_through_view_stays_shared"),
        "9 bb false\n10 typed {\"n\":10,\"s\":\"typed\",\"ok\":false}\n11 11 true true\n6\n12\n"
    );
}

#[test]
fn gb_contract_linked_json_values_survive_arena_resets() {
    let source = r##"async function main(): Promise<void> {
  const words: string[][] = [["a"]];
  const wv: any = words;
  const w: any = ["x"];
  wv[0] = w;
  const objs: { n: number; s: string }[] = [{ n: 0, s: "z" }];
  const ov: any = objs;
  const o: any = { n: 1, s: "q" };
  ov[0] = o;
  let sum = 0;
  for (let i = 0; i < 200; i++) {
    w.push("item" + i);
    o.n = i;
    o.s = "s" + i;
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const junk: string[] = [];
    for (let k = 0; k < 200; k++) junk.push("junk" + k + i);
    sum += junk.length + words[0].length + objs[0].n;
    if (words[0][words[0].length - 1] !== "item" + i || objs[0].s !== "s" + i) {
      console.log("MISMATCH", i, words[0][words[0].length - 1], objs[0].s);
      return;
    }
  }
  console.log(words[0].length, objs[0].n, sum, w.length, o.s);
}
"##;
    assert_eq!(
        compile_and_run(source, "gb_contract_linked_json_values_survive_arena_resets"),
        "201 199 80200 201 s199\n"
    );
}
