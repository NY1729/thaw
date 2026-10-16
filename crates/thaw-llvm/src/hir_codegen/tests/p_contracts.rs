// Wave P: Promise.allSettled per-status results and Promise.any errors. Expected output recorded from node 22.

#[test]
fn p_contract_allsettled_async_rejections_have_the_per_status_shape() {
    let source = r####"async function f(n: number): Promise<string> { await Promise.resolve(); if (n % 2 === 0) { throw new TypeError('even ' + n); } return 'odd ' + n; }
async function main(): Promise<void> {
  const rs = await Promise.allSettled([f(1), f(2), f(3), f(4)]);
  console.log(rs.length, rs.filter((x: any) => x.status === 'fulfilled').length);
  for (const x of rs) {
    if (x.status === 'rejected') { console.log('rejected', String(x.reason.message)); } else { console.log('fulfilled', x.value); }
  }
  console.log(Object.keys(rs[0]).join(','), Object.keys(rs[1]).join(','));
}
"####;
    assert_eq!(
        compile_and_run(source, "p_contract_allsettled_async_rejections_have_the_per_status_shape"),
        "4 2\nfulfilled odd 1\nrejected even 2\nfulfilled odd 3\nrejected even 4\nstatus,value status,reason\n"
    );
}

#[test]
fn p_contract_allsettled_primitive_and_object_reasons_and_empty_input() {
    let source = r####"async function main(): Promise<void> {
  const rs = await Promise.allSettled([Promise.reject(1), Promise.reject({ code: 5 }), Promise.reject(null), Promise.resolve(undefined), Promise.resolve('ok')]);
  console.log(JSON.stringify(rs));
  const empty = await Promise.allSettled([]);
  console.log(JSON.stringify(empty), empty.length);
}
"####;
    assert_eq!(
        compile_and_run(source, "p_contract_allsettled_primitive_and_object_reasons_and_empty_input"),
        "[{\"status\":\"rejected\",\"reason\":1},{\"status\":\"rejected\",\"reason\":{\"code\":5}},{\"status\":\"rejected\",\"reason\":null},{\"status\":\"fulfilled\"},{\"status\":\"fulfilled\",\"value\":\"ok\"}]\n[] 0\n"
    );
}

#[test]
fn p_contract_allsettled_array_values_spreads_and_plain_values() {
    let source = r####"async function f(n: number): Promise<number> { await Promise.resolve(); if (n === 2) { throw new Error('two'); } return n * 10; }
async function main(): Promise<void> {
  const ps = [f(1), f(2)];
  const a = await Promise.allSettled(ps);
  console.log(a.map((x: any) => x.status).join(','), a[0].value, a[1].reason.message);
  const b = await Promise.allSettled([...ps, f(3)]);
  console.log(b.length, b[2].status, b[2].value);
  const c = await Promise.allSettled([f(4), 'plain', Promise.resolve(true)]);
  console.log(JSON.stringify(c));
}
"####;
    assert_eq!(
        compile_and_run(source, "p_contract_allsettled_array_values_spreads_and_plain_values"),
        "fulfilled,rejected 10 two\n3 fulfilled 30\n[{\"status\":\"fulfilled\",\"value\":40},{\"status\":\"fulfilled\",\"value\":\"plain\"},{\"status\":\"fulfilled\",\"value\":true}]\n"
    );
}

#[test]
fn p_contract_promise_any_rejection_exposes_errors_with_real_errors() {
    let source = r####"async function f(n: number): Promise<number> { await Promise.resolve(); throw new Error('f' + n); }
async function main(): Promise<void> {
  try {
    await Promise.any([f(1), Promise.reject('b'), Promise.reject(7), Promise.reject(null)]);
  } catch (e: any) {
    console.log(e.name, e.message, e instanceof AggregateError);
    console.log(e.errors.length, e.errors[0].message, e.errors[1], e.errors[2], e.errors[3]);
  }
  const v = await Promise.any([Promise.reject(new Error('z')), Promise.resolve(5)]);
  console.log(v);
  try { await Promise.any([]); } catch (e: any) { console.log(e.name, e.errors.length); }
}
"####;
    assert_eq!(
        compile_and_run(source, "p_contract_promise_any_rejection_exposes_errors_with_real_errors"),
        "AggregateError All promises were rejected true\n4 f1 b 7 null\n5\nAggregateError 0\n"
    );
}
