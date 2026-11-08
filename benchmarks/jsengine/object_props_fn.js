// 1e6 property get/set cycles on one object inside a function, called twice; expected checksum: object_props_fn 499999500000 1 1000002
function main() {
  const o = { a: 0, b: 1, c: 2 };
  for (let i = 0; i < 1000000; i++) {
    o.a = o.a + i;
    o.b = (o.b + o.c) % 1000003;
    o.c = o.c + 1;
  }
  return "object_props_fn " + o.a + " " + o.b + " " + o.c;
}
main();
console.log(main());
