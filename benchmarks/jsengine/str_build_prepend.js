// 2e4 `s = x + s` prepends; expected checksum: str_build_prepend 60000 97
let s = "";
for (let i = 0; i < 20000; i++) {
  s = ("ab" + (i % 10)) + s;
}
console.log("str_build_prepend " + s.length + " " + s.charCodeAt(0));
