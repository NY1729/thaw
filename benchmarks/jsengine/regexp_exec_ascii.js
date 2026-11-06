// global regexp exec loop over a ~127 KB ASCII subject; expected checksum: regexp_exec_ascii 20000 399960000
const parts = [];
for (let i = 0; i < 10000; i++) {
  parts.push("n" + i + " x" + (i * 7) + "\n");
}
const text = parts.join("");
const re = /\d+/g;
let m;
let matches = 0;
let sum = 0;
while ((m = re.exec(text)) !== null) {
  matches++;
  sum += Number(m[0]);
}
console.log("regexp_exec_ascii " + matches + " " + sum);
