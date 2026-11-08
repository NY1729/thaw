// 1e6 plain data property reads inside a function, called twice; expected checksum: prop_get_fn 999985
function main() {
  const o = { a: 1, b: 2, c: 3 };
  let s = 0;
  for (let i = 0; i < 1000000; i++) {
    s = (s + o.a + o.b + o.c) % 1000003;
  }
  return "prop_get_fn " + s;
}
main();
console.log(main());
