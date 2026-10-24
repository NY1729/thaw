Promise.resolve().then(() => {
  console.log('a');
  process.nextTick(() => console.log('tick'));
  Promise.resolve().then(() => console.log('b'));
}).then(() => console.log('c'));
process.nextTick(() => {
  console.log('t1');
  Promise.resolve().then(() => console.log('micro-from-tick'));
  process.nextTick(() => console.log('t2'));
});
