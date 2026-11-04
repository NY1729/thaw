// 1e5 string concatenations, then repeated indexOf scans; expected checksum: string_ops 300000 10000
let s = "";
for (let i = 0; i < 100000; i++) {
  s += "ab" + (i % 10);
}
let count = 0;
let pos = s.indexOf("b5");
while (pos !== -1) {
  count++;
  pos = s.indexOf("b5", pos + 1);
}
console.log("string_ops " + s.length + " " + count);
