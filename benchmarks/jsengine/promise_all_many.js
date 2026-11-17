// Promise.all over 10k resolved promises, 10 rounds run back to back; expected checksum: promise_all_many 100000 499950000
let total = 0;
let count = 0;
function round(r) {
  if (r === 10) return Promise.resolve();
  const ps = [];
  for (let i = 0; i < 10000; i++) ps.push(new Promise((resolve) => resolve(i)));
  return Promise.all(ps).then((vals) => {
    for (const v of vals) {
      total += v;
      count++;
    }
    return round(r + 1);
  });
}
round(0).then(() => console.log("promise_all_many " + count + " " + total));
