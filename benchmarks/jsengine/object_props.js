// 1e6 property get/set cycles on one object
const o = { a: 0, b: 1, c: 2 };
for (let i = 0; i < 1000000; i++) {
  o.a = o.a + i;
  o.b = (o.b + o.c) % 1000003;
  o.c = o.c + 1;
}
console.log("object_props " + o.a + " " + o.b + " " + o.c);
