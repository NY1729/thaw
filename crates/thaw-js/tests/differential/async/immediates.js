setImmediate(() => { console.log('i1'); process.nextTick(() => console.log('tick-in-i1')); Promise.resolve().then(() => console.log('micro-in-i1')); });
setImmediate(() => console.log('i2'));
const c = setImmediate(() => console.log('cleared'));
clearImmediate(c);
setTimeout(() => {
  setImmediate(() => console.log('imm-from-timer'));
  setTimeout(() => console.log('timer-from-timer'), 0);
}, 5);
