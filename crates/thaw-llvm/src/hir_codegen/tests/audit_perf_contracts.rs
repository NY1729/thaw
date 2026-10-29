// Perf-audit contracts (G001 push growth, G002 string append, G013 comparator sort). Expected output from node 22.

#[test]
fn audit_perf_array_push_aliasing_and_growth_match_node() {
    let source = r#"
function fill(a: number[], n: number): void { for (let i = 0; i < n; i++) a.push(i); }
function main(): void {
  const a: number[] = [1];
  const b = a;
  a.push(2);
  b.push(3, 4);
  console.log(a.length, b.length, a.join(","), b.join(","));
  const c = a.slice();
  a.push(9);
  console.log(c.length, a.length, c.join(","), a.join(","));
  const big: number[] = [];
  fill(big, 1000);
  let sum = 0;
  for (let i = 0; i < big.length; i++) sum += big[i];
  console.log(big.length, sum, big[0], big[999]);
  const popped = big.pop();
  big.push(5, 6, 7);
  console.log(popped, big.length, big[998], big[999], big[1000], big[1001]);
  big.shift();
  big.push(1);
  console.log(big.length, big[0], big[big.length - 1]);
  const words: string[] = [];
  for (let i = 0; i < 50; i++) words.push("w" + i);
  console.log(words.length, words[0], words[49], words.slice(10, 13).join("|"));
  const objs: { id: number; tag: string }[] = [];
  for (let i = 0; i < 40; i++) objs.push({ id: i, tag: "t" + i });
  let ids = 0;
  for (const o of objs) ids += o.id;
  console.log(objs.length, ids, objs[39].tag);
  const flags: boolean[] = [];
  for (let i = 0; i < 9; i++) flags.push(i % 3 === 0);
  console.log(flags.filter((x) => x).length, flags.length);
  const grid: number[][] = [];
  for (let i = 0; i < 5; i++) { const row: number[] = []; for (let j = 0; j < 5; j++) row.push(i * j); grid.push(row); }
  console.log(grid.length, grid[4][4], grid.map((r) => r.length).join(","));
  const holder = { items: [] as number[] };
  for (let i = 0; i < 20; i++) holder.items.push(i * i);
  console.log(holder.items.length, holder.items[19]);
  const copy = [...holder.items];
  holder.items.push(1);
  console.log(copy.length, holder.items.length);
  const e: number[] = [];
  console.log(e.push(1), e.push(2, 3), e.length);
}
"#;
    let output = compile_and_run(source, "audit_perf_array_push_aliasing_and_growth_match_node");
    assert_eq!(output, "4 4 1,2,3,4 1,2,3,4\n4 5 1,2,3,4 1,2,3,4,9\n1000 499500 0 999\n999 1002 998 5 6 7\n1002 1 1\n50 w0 w49 w10|w11|w12\n40 780 t39\n3 9\n5 16 5,5,5,5,5\n20 361\n20 21\n1 3 3\n");
}

#[test]
fn audit_perf_string_append_aliasing_matches_node() {
    let source = r#"
function main(): void {
  let s = "";
  const snaps: string[] = [];
  for (let i = 0; i < 6; i++) { s += "x"; snaps.push(s); }
  console.log(snaps.join(","), s.length);
  let t = "";
  let kept = "";
  for (let i = 0; i < 10; i++) { t += i; if (i === 4) kept = t; }
  console.log(t, kept, kept.length);
  let u = "a";
  for (let i = 0; i < 4; i++) u += u;
  console.log(u.length, u);
  let m = "";
  for (let i = 0; i < 5; i++) m = m + "<" + i + ">";
  console.log(m);
  let z = "";
  for (let i = 0; i < 4; i++) z += "a\0b";
  console.log(z.length, z.split("\0").length);
  let w = "";
  const parts: string[] = [];
  let i = 0;
  while (i < 8) { w += String.fromCharCode(97 + i); if (w.length % 3 === 0) parts.push(w); i++; }
  console.log(w, parts.join("/"));
  let a = "";
  let b = "";
  for (let k = 0; k < 5; k++) { a += "p"; b = a; a += "q"; }
  console.log(a, b);
  let lng = "";
  for (let k = 0; k < 3000; k++) lng += "ab";
  console.log(lng.length, lng.slice(0, 6), lng.slice(-4));
  let nested = "";
  for (let p = 0; p < 3; p++) { for (let q = 0; q < 3; q++) nested += p * 3 + q; nested += "|"; }
  console.log(nested);
  let cap = "";
  const fns: (() => string)[] = [];
  for (let k = 0; k < 3; k++) { cap += k; fns.push(() => cap); }
  cap += "!";
  console.log(fns.map((f) => f()).join(","));
  let emo = "";
  for (let k = 0; k < 3; k++) emo += "é\u{1F600}";
  console.log(emo.length, emo);
}
"#;
    let output = compile_and_run(source, "audit_perf_string_append_aliasing_matches_node");
    assert_eq!(output, "x,xx,xxx,xxxx,xxxxx,xxxxxx 6\n0123456789 01234 5\n16 aaaaaaaaaaaaaaaa\n<0><1><2><3><4>\n12 5\nabcdefgh abc/abcdef\npqpqpqpqpq pqpqpqpqp\n6000 ababab abab\n012|345|678|\n012!,012!,012!\n9 é😀é😀é😀\n");
}

#[test]
fn audit_perf_comparator_sort_is_stable_and_correct_like_node() {
    let source = r#"
interface P { k: number; n: string }
function main(): void {
  const a: number[] = [];
  for (let i = 0; i < 200; i++) a.push((i * 37) % 101);
  a.sort((x, y) => y - x);
  console.log(a[0], a[1], a[199], a.length);
  const up = a.slice().sort((x, y) => x - y);
  console.log(up.slice(0, 8).join(","), up.slice(-3).join(","));
  const ps: P[] = [];
  for (let i = 0; i < 30; i++) ps.push({ k: i % 4, n: "n" + i });
  ps.sort((p, q) => p.k - q.k);
  console.log(ps.map((p) => p.n).join(" "));
  const ws = ["pear", "fig", "apple", "kiwi", "plum", "date", "banana"];
  ws.sort((x, y) => x.length - y.length);
  console.log(ws.join(","));
  const one = [5];
  one.sort((x, y) => x - y);
  const none: number[] = [];
  none.sort((x, y) => x - y);
  console.log(one.join(","), none.length);
  const same = [3, 3, 3, 1, 1, 2];
  console.log(same.sort((x, y) => x - y).join(","));
  const ts = [9, 8, 7, 1, 2, 3].toSorted((x, y) => x - y);
  console.log(ts.join(","));
  let calls = 0;
  const big: number[] = [];
  for (let i = 0; i < 3000; i++) big.push((i * 7919) % 3001);
  big.sort((x, y) => { calls++; return x - y; });
  let ok = true;
  for (let i = 1; i < big.length; i++) if (big[i - 1] > big[i]) ok = false;
  console.log(ok, calls < 60000);
  const r = [4, 2, 5, 1, 3];
  const same2 = r.sort((x, y) => y - x);
  console.log(same2 === r, r.join(","));
}
"#;
    let output = compile_and_run(source, "audit_perf_comparator_sort_is_stable_and_correct_like_node");
    assert_eq!(output, "100 100 0 200\n0,0,1,1,2,2,3,3 99,100,100\nn0 n4 n8 n12 n16 n20 n24 n28 n1 n5 n9 n13 n17 n21 n25 n29 n2 n6 n10 n14 n18 n22 n26 n3 n7 n11 n15 n19 n23 n27\nfig,pear,kiwi,plum,date,apple,banana\n5 0\n1,1,2,3,3,3\n1,2,3,7,8,9\ntrue true\ntrue 5,4,3,2,1\n");
}

#[test]
fn audit_perf_large_push_append_sort_stay_within_budget() {
    let source = r#"
interface Item { id: number; name: string }
function main(): void {
  const start = Date.now();
  const nums: number[] = [];
  for (let i = 0; i < 100000; i++) nums.push(i * 2);
  const strs: string[] = [];
  for (let i = 0; i < 20000; i++) strs.push("s" + i);
  const items: Item[] = [];
  for (let i = 0; i < 20000; i++) items.push({ id: i, name: "n" + i });
  let s = "";
  for (let i = 0; i < 100000; i++) s += "x";
  const desc: number[] = [];
  for (let i = 0; i < 20000; i++) desc.push((i * 7919) % 20011);
  desc.sort((x, y) => y - x);
  const elapsed = Date.now() - start;
  console.log(nums.length, strs.length, items.length, s.length, desc[0], desc[19999]);
  console.log("rss under 400MB:", process.memoryUsage().rss < 400 * 1024 * 1024);
  console.log("time under 20s:", elapsed < 20000);
}
"#;
    let output = compile_and_run(source, "audit_perf_large_push_append_sort_stay_within_budget");
    assert_eq!(output, "100000 20000 20000 100000 20010 0\nrss under 400MB: true\ntime under 20s: true\n");
}

