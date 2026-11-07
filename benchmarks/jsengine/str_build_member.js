// 2e4 `o.buf += x` appends on a property (not an identifier); expected checksum: str_build_member 60000 57
let o = { buf: "" };
for (let i = 0; i < 20000; i++) {
  o.buf += "ab" + (i % 10);
}
console.log("str_build_member " + o.buf.length + " " + o.buf.charCodeAt(o.buf.length - 1));
