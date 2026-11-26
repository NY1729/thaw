//! Map/Set `delete` leaves a tombstone and compacts later: live iterators, `forEach` and re-adds must see the same
//! sequence as before (the expected string is what Node prints for the same program).
use super::*;

const PROG: &str = r#"const out = [];
const m = new Map();
for (let i = 0; i < 100; i++) m.set('k' + i, i);
const it = m.entries();
for (let i = 0; i < 10; i++) it.next();
for (let i = 0; i < 70; i++) m.delete('k' + i);          // compacts while `it` is live
out.push(m.size, JSON.stringify(it.next().value), m.has('k5'), m.get('k80'));
m.set('k5', 'again'); m.delete('k99'); m.set('k99', 'last');
out.push([...m.keys()].slice(-3).join(), m.size);
const seen = [];
m.forEach((v, k) => { seen.push(k); if (k === 'k75') { for (let i = 76; i < 90; i++) m.delete('k' + i); m.set('new', 1); } });
out.push(seen.join(), m.size);
m.clear(); m.set(NaN, 1); m.set(-0, 2); out.push(m.get(NaN), m.get(0), m.size);
const s = new Set();
for (let i = 0; i < 64; i++) s.add(i);
const si = s.values(); si.next(); si.next(); si.next();
for (let i = 0; i < 60; i++) s.delete(i);
out.push(s.size, si.next().value, [...s].join());
s.add(3); s.add(63); s.add(undefined); s.delete(undefined); s.add(undefined);
out.push([...s].join(), s.has(3), s.has(undefined), s.size);
const big = new Set(); for (let i = 0; i < 1000; i++) big.add(i); for (let i = 0; i < 1000; i += 2) big.delete(i);
let t = 0; for (const v of big) t += v; out.push(big.size, t, big.has(500), big.has(501));
const fi = new Set(); for (let i = 1; i <= 20; i++) fi.add(i);
const log = []; fi.forEach(v => { log.push(v); if (v < 5) fi.delete(v + 1); if (v === 6) { for (let k = 7; k < 20; k++) fi.delete(k); } });
out.push(log.join(), fi.size);
out.join('|');
"#;

#[test]
fn map_set_delete_with_live_cursors_matches_node() {
    let mut it = Interp::new();
    let v = match it.run(PROG) {
        Ok(v) => v,
        Err(_) => panic!("program threw"),
    };
    assert_eq!(
        &*it.to_str(&v),
        r#"30|["k70",70]|false|80|k98,k5,k99|31|k70,k71,k72,k73,k74,k75,k90,k91,k92,k93,k94,k95,k96,k97,k98,k5,k99,new|18|1|2|2|4|60|60,61,62,63|60,61,62,63,3,|true|true|6|500|250000|false|true|1,3,5,6,20|5"#
    );
}
