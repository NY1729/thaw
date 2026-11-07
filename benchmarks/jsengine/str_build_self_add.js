// 2e4 `s = s + x` appends (the non-compound spelling); expected checksum: str_build_self_add 60000 57
let s = "";
for (let i = 0; i < 20000; i++) {
  s = s + ("ab" + (i % 10));
}
console.log("str_build_self_add " + s.length + " " + s.charCodeAt(s.length - 1));
