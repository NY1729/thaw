// Buffer/Uint8Array mutators across receiver kinds. Expected output from node 22.

#[test]
fn bufmut_buffer_swap() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Buffer => Buffer.from(a);

  const b = MK([1, 2, 3, 4, 5, 6, 7, 8]);
  b.swap16(); console.log(b.toString("hex"));
  b.swap32(); console.log(b.toString("hex"));
  b.swap64(); console.log(b.toString("hex"));
  const odd = MK([1, 2, 3]);
  try { odd.swap16(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  try { odd.swap32(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  try { odd.swap64(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  console.log(odd.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_buffer_swap");
    assert_eq!(output, "0201040306050807\n0304010207080506\n0605080702010403\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 16-bits\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 32-bits\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 64-bits\n010203\n");
}

#[test]
fn bufmut_buffer_revsort() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Buffer => Buffer.from(a);

  const r = MK([1, 2, 3, 4]);
  r.reverse(); console.log(r.toString("hex"));
  const s = MK([30, 10, 20, 5]);
  s.sort(); console.log(s.toString("hex"));
  s.sort((x: number, y: number) => y - x); console.log(s.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_buffer_revsort");
    assert_eq!(output, "04030201\n050a141e\n1e140a05\n");
}

#[test]
fn bufmut_buffer_setcopy() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Buffer => Buffer.from(a);

  const t = MK([0, 0, 0, 0, 0, 0]);
  t.set([1, 2, 3], 2); console.log(t.toString("hex"));
  const c = MK([1, 2, 3, 4, 5]);
  c.copyWithin(0, 3); console.log(c.toString("hex"));
  c.fill(7, 1, 3); console.log(c.toString("hex"));
  console.log(c.indexOf(7), c.includes(5), c.includes(99));
}
"#;
    let output = compile_and_run(source, "bufmut_buffer_setcopy");
    assert_eq!(output, "000001020300\n0405030405\n0407070405\n1 true false\n");
}

#[test]
fn bufmut_u8_revsort() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Uint8Array => new Uint8Array(a);

  const r = MK([1, 2, 3, 4]);
  r.reverse(); console.log(Array.from(r).join(","));
  const s = MK([30, 10, 20, 5]);
  s.sort(); console.log(Array.from(s).join(","));
  s.sort((x: number, y: number) => y - x); console.log(Array.from(s).join(","));
}
"#;
    let output = compile_and_run(source, "bufmut_u8_revsort");
    assert_eq!(output, "4,3,2,1\n5,10,20,30\n30,20,10,5\n");
}

#[test]
fn bufmut_u8_setcopy() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Uint8Array => new Uint8Array(a);

  const t = MK([0, 0, 0, 0, 0, 0]);
  t.set([1, 2, 3], 2); console.log(Array.from(t).join(","));
  const c = MK([1, 2, 3, 4, 5]);
  c.copyWithin(0, 3); console.log(Array.from(c).join(","));
  c.fill(7, 1, 3); console.log(Array.from(c).join(","));
  console.log(c.indexOf(7), c.includes(5), c.includes(99));
}
"#;
    let output = compile_and_run(source, "bufmut_u8_setcopy");
    assert_eq!(output, "0,0,1,2,3,0\n4,5,3,4,5\n4,7,7,4,5\n1 true false\n");
}

#[test]
fn bufmut_any_swap() {
    let source = r#"function main(): void {
  const MK = (a: number[]): any => Buffer.from(a);

  const b = MK([1, 2, 3, 4, 5, 6, 7, 8]);
  b.swap16(); console.log(b.toString("hex"));
  b.swap32(); console.log(b.toString("hex"));
  b.swap64(); console.log(b.toString("hex"));
  const odd = MK([1, 2, 3]);
  try { odd.swap16(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  try { odd.swap32(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  try { odd.swap64(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  console.log(odd.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_any_swap");
    assert_eq!(output, "0201040306050807\n0304010207080506\n0605080702010403\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 16-bits\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 32-bits\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 64-bits\n010203\n");
}

#[test]
fn bufmut_any_revsort() {
    let source = r#"function main(): void {
  const MK = (a: number[]): any => Buffer.from(a);

  const r = MK([1, 2, 3, 4]);
  r.reverse(); console.log(r.toString("hex"));
  const s = MK([30, 10, 20, 5]);
  s.sort(); console.log(s.toString("hex"));
  s.sort((x: number, y: number) => y - x); console.log(s.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_any_revsort");
    assert_eq!(output, "04030201\n050a141e\n1e140a05\n");
}

#[test]
fn bufmut_any_setcopy() {
    let source = r#"function main(): void {
  const MK = (a: number[]): any => Buffer.from(a);

  const t = MK([0, 0, 0, 0, 0, 0]);
  t.set([1, 2, 3], 2); console.log(t.toString("hex"));
  const c = MK([1, 2, 3, 4, 5]);
  c.copyWithin(0, 3); console.log(c.toString("hex"));
  c.fill(7, 1, 3); console.log(c.toString("hex"));
  console.log(c.indexOf(7), c.includes(5), c.includes(99));
}
"#;
    let output = compile_and_run(source, "bufmut_any_setcopy");
    assert_eq!(output, "000001020300\n0405030405\n0407070405\n1 true false\n");
}

#[test]
fn bufmut_union_swap() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Buffer | null => Buffer.from(a);

  const b: Buffer | null = MK([1, 2, 3, 4, 5, 6, 7, 8]);
  if (b === null) return;
  b.swap16(); console.log(b.toString("hex"));
  b.swap32(); console.log(b.toString("hex"));
  b.swap64(); console.log(b.toString("hex"));
  const odd: Buffer | null = MK([1, 2, 3]);
  if (odd === null) return;
  try { odd.swap16(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  try { odd.swap32(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  try { odd.swap64(); } catch (e: any) { console.log(e.name, e.code, e.message); }
  console.log(odd.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_union_swap");
    assert_eq!(output, "0201040306050807\n0304010207080506\n0605080702010403\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 16-bits\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 32-bits\nRangeError ERR_INVALID_BUFFER_SIZE Buffer size must be a multiple of 64-bits\n010203\n");
}

#[test]
fn bufmut_union_revsort() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Buffer | null => Buffer.from(a);

  const r: Buffer | null = MK([1, 2, 3, 4]);
  if (r === null) return;
  r.reverse(); console.log(r.toString("hex"));
  const s: Buffer | null = MK([30, 10, 20, 5]);
  if (s === null) return;
  s.sort(); console.log(s.toString("hex"));
  s.sort((x: number, y: number) => y - x); console.log(s.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_union_revsort");
    assert_eq!(output, "04030201\n050a141e\n1e140a05\n");
}

#[test]
fn bufmut_union_setcopy() {
    let source = r#"function main(): void {
  const MK = (a: number[]): Buffer | null => Buffer.from(a);

  const t: Buffer | null = MK([0, 0, 0, 0, 0, 0]);
  if (t === null) return;
  t.set([1, 2, 3], 2); console.log(t.toString("hex"));
  const c: Buffer | null = MK([1, 2, 3, 4, 5]);
  if (c === null) return;
  c.copyWithin(0, 3); console.log(c.toString("hex"));
  c.fill(7, 1, 3); console.log(c.toString("hex"));
  console.log(c.indexOf(7), c.includes(5), c.includes(99));
}
"#;
    let output = compile_and_run(source, "bufmut_union_setcopy");
    assert_eq!(output, "000001020300\n0405030405\n0407070405\n1 true false\n");
}

#[test]
fn bufmut_anyu8_revsort() {
    let source = r#"function main(): void {
  const MK = (a: number[]): any => new Uint8Array(a);

  const r = MK([1, 2, 3, 4]);
  r.reverse(); console.log(Array.from(r).join(","));
  const s = MK([30, 10, 20, 5]);
  s.sort(); console.log(Array.from(s).join(","));
  s.sort((x: number, y: number) => y - x); console.log(Array.from(s).join(","));
}
"#;
    let output = compile_and_run(source, "bufmut_anyu8_revsort");
    assert_eq!(output, "4,3,2,1\n5,10,20,30\n30,20,10,5\n");
}

#[test]
fn bufmut_anyu8_setcopy() {
    let source = r#"function main(): void {
  const MK = (a: number[]): any => new Uint8Array(a);

  const t = MK([0, 0, 0, 0, 0, 0]);
  t.set([1, 2, 3], 2); console.log(Array.from(t).join(","));
  const c = MK([1, 2, 3, 4, 5]);
  c.copyWithin(0, 3); console.log(Array.from(c).join(","));
  c.fill(7, 1, 3); console.log(Array.from(c).join(","));
  console.log(c.indexOf(7), c.includes(5), c.includes(99));
}
"#;
    let output = compile_and_run(source, "bufmut_anyu8_setcopy");
    assert_eq!(output, "0,0,1,2,3,0\n4,5,3,4,5\n4,7,7,4,5\n1 true false\n");
}

#[test]
fn bufmut_captured_all() {
    let source = r#"function main(): void {
  const b = Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]);
  const f = (): void => { b.swap16(); b.swap32(); b.reverse(); };
  f();
  console.log(b.toString("hex"));
  const a: any = Buffer.from([4, 3, 2, 1, 9, 8, 7, 6]);
  const g = (): void => { a.swap32(); a.sort(); a.set([9], 0); a.copyWithin(1, 5); a.fill(1, 6); };
  g();
  console.log(a.toString("hex"), a.indexOf(9), a.includes(6));
  let u: Buffer | null = Buffer.from([3, 1, 2]);
  const h = (): void => { if (u !== null) { u.sort(); u.reverse(); } };
  h();
  console.log(u!.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_captured_all");
    assert_eq!(output, "0605080702010403\n0907080906070101 0 true\n030201\n");
}

#[test]
fn bufmut_u8_no_swap_and_delete() {
    let source = r#"function main(): void {
  "use strict";
  const u: any = new Uint8Array(4);
  try { u.swap16(); } catch (e: any) { console.log(e.name); }
  const b: any = Buffer.from([1, 2, 3]);
  try { delete b[0]; console.log("deleted"); } catch (e: any) { console.log(e.name, e.message); }
  console.log(b.toString("hex"));
}
"#;
    let output = compile_and_run(source, "bufmut_u8_no_swap_and_delete");
    assert_eq!(output, "TypeError\nTypeError Cannot delete property '0' of [object Uint8Array]\n010203\n");
}
