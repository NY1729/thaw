// 1e5 Map set then get with numeric keys only (isolates integer-key hashing); expected checksum: map_num_keys 100000 9999900000
const m = new Map();
for (let i = 0; i < 100000; i++) {
  m.set(i, i * 2);
}
let got = 0;
for (let i = 0; i < 100000; i++) {
  got += m.get(i);
}
console.log("map_num_keys " + m.size + " " + got);
