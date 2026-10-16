// Wave HB: nested Json containers written through an `any` view into typed native slots stay linked at every
// level. Expected output from node 22.

#[test]
fn hb_contract_nested_number_arrays_stay_linked_through_three_levels() {
    let source = r##"async function main(): Promise<void> {
  const nat: number[][][] = [[[1]]];
  const v: any = nat;
  const inner: any = [[5, 6], [7]];
  v[0] = inner;
  inner[0].push(8);
  console.log(nat[0][0].length, JSON.stringify(inner));
  nat[0][1].push(9);
  console.log(inner[1].length, v[0][1].join(","));
  inner.push([10]);
  console.log(nat[0].length, nat[0][2][0]);
  nat[0][2].push(11);
  console.log(inner[2].join(","));
  console.log(v[0] === inner, v[0][0] === inner[0]);
  nat[0][0] = [42, 43];
  console.log(inner[0].join(","), JSON.stringify(nat));
}
"##;
    assert_eq!(
        compile_and_run(source, "hb_contract_nested_number_arrays_stay_linked_through_three_levels"),
        "3 [[5,6,8],[7]]\n2 7,9\n3 10\n10,11\ntrue true\n42,43 [[[42,43],[7,9],[10,11]]]\n"
    );
}

#[test]
fn hb_contract_records_with_array_fields_stay_linked() {
    let source = r##"interface Row { n: number; tags: string[] }
async function main(): Promise<void> {
  const rows: Row[] = [{ n: 1, tags: ["a"] }];
  const v: any = rows;
  const row: any = JSON.parse('{"n":2,"tags":["x","y"]}');
  v[0] = row;
  row.tags.push("z");
  console.log(rows[0].tags.join(","), rows[0].n);
  rows[0].tags.push("w");
  console.log(row.tags.join(","));
  row.n = 5;
  console.log(rows[0].n);
  rows[0].tags[0] = "q";
  console.log(row.tags[0], v[0] === row);
  const extra: any = JSON.parse('{"n":3,"tags":[]}');
  v.push(extra);
  extra.tags.push("t");
  console.log(rows.length, rows[1].tags.join(","));
  rows[1].tags.push("u");
  console.log(extra.tags.join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "hb_contract_records_with_array_fields_stay_linked"),
        "x,y,z 2\nx,y,z,w\n5\nq true\n2 t\nt,u\n"
    );
}

#[test]
fn hb_contract_repeated_nested_edits_survive_arena_resets() {
    let source = r##"async function main(): Promise<void> {
  const grid: number[][][] = [[[0]], [[1]]];
  const v: any = grid;
  const a: any = [[1], [2, 3]];
  const b: any = [[4]];
  v[0] = a;
  v[1] = b;
  for (let i = 0; i < 3; i++) {
    a[0].push(10 + i);
    grid[1][0].push(20 + i);
  }
  console.log(JSON.stringify(grid), JSON.stringify(a), JSON.stringify(b));
  a.length = 1;
  console.log(grid[0].length, JSON.stringify(grid[0]));
}
"##;
    assert_eq!(
        compile_and_run(source, "hb_contract_repeated_nested_edits_survive_arena_resets"),
        "[[[1,10,11,12],[2,3]],[[4,20,21,22]]] [[1,10,11,12],[2,3]] [[4,20,21,22]]\n1 [[1,10,11,12]]\n"
    );
}
