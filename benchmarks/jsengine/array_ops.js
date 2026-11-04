// 1e6 push, then map and reduce; expected checksum: array_ops 1000000 999999000000
const a = [];
for (let i = 0; i < 1000000; i++) {
  a.push(i);
}
const m = a.map((x) => x * 2);
const total = m.reduce((acc, x) => acc + x, 0);
console.log("array_ops " + a.length + " " + total);
