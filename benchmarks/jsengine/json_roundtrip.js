// ~1 MB JSON: stringify, parse, stringify again; expected checksum: json_roundtrip <len> true <score>
const rows = [];
for (let i = 0; i < 7000; i++) {
  rows.push({
    id: i,
    name: "item-" + i,
    price: ((i * 37) % 1000) / 10,
    tags: ["x" + (i % 7), "y" + (i % 11)],
    active: i % 3 === 0,
    meta: { created: 1700000000 + i, score: i % 97 },
  });
}
const text = JSON.stringify(rows);
const parsed = JSON.parse(text);
const again = JSON.stringify(parsed);
let score = 0;
for (const r of parsed) {
  score += r.meta.score;
}
console.log("json_roundtrip " + text.length + " " + (again === text) + " " + score);
