// 1e5 `s += x` appends (3 chars each), no reads in the loop; expected checksum: str_build_add_assign 300000 57
let s = "";
for (let i = 0; i < 100000; i++) {
  s += "ab" + (i % 10);
}
console.log("str_build_add_assign " + s.length + " " + s.charCodeAt(s.length - 1));
