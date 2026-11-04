// global regexp exec loop over ~20k lines; expected checksum: regexp_match 20000 <sum>
const lines = [];
for (let i = 0; i < 20000; i++) {
  lines.push("id " + i + " ref " + ((i * 7919) % 10000) + "-" + ((i % 9000) + 1000) + " end");
}
const text = lines.join("\n");
const re = /(\d+)-(\d+)/g;
let m;
let matches = 0;
let sum = 0;
while ((m = re.exec(text)) !== null) {
  matches++;
  sum += Number(m[1]) + Number(m[2]);
}
console.log("regexp_match " + matches + " " + sum);
