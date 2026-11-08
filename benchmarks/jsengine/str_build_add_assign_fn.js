// 1e5 `s += x` appends (3 chars each) inside a function, called twice; expected checksum: str_build_add_assign_fn 300000 57
function main() {
  let s = "";
  for (let i = 0; i < 100000; i++) {
    s += "ab" + (i % 10);
  }
  return "str_build_add_assign_fn " + s.length + " " + s.charCodeAt(s.length - 1);
}
main();
console.log(main());
