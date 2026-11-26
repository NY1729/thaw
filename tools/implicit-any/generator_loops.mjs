// Loops inside generators: loop temporaries stay captured by the generator closure.

function* gen(items) {
  for (const x of items) yield x * 2;
}
console.log([...gen([1, 2, 3])].join(','));

async function* agen(items) {
  for (const x of items) {
    yield x + 1;
  }
}
const seen = [];
for await (const v of agen([1, 2, 3])) seen.push(v);
console.log(seen.join(','));
