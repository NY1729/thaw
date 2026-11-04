// 1e5 Map set/get and Set add/has; expected checksum: map_set 100000 9999900000 50000 50000
const m = new Map();
for (let i = 0; i < 100000; i++) {
  m.set(i, i * 2);
}
let got = 0;
for (let i = 0; i < 100000; i++) {
  got += m.get(i);
}
const s = new Set();
for (let i = 0; i < 100000; i++) {
  s.add(i % 50000);
}
let hits = 0;
for (let i = 0; i < 100000; i++) {
  if (s.has(i)) hits++;
}
console.log("map_set " + m.size + " " + got + " " + s.size + " " + hits);
