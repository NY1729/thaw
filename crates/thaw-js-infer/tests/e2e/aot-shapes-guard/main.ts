import { area, label, longest } from "aot-shapes";
// `Json` arguments prove nothing statically: the clones sit behind run-time guards, and the
// values that break the contract (a number inside the string array) take the original path
function main(): void {
  const ok: Json = JSON.parse('{"name":"n","tags":["x","y"]}');
  const bad: Json = JSON.parse('{"name":"n","tags":["x",2]}');
  console.log(label(ok), label(bad));
  const shape: Json = JSON.parse('{"w":3,"h":4}');
  const odd: Json = JSON.parse('{"w":"3","h":4}');
  console.log(area(shape), area(odd));
  const words: Json = JSON.parse('["a","ccc",7]');
  console.log(longest(words));
}
