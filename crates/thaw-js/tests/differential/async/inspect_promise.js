console.log(Promise.resolve(1), Promise.reject(2).catch(() => {}) instanceof Promise);
const p = new Promise(() => {});
console.log(p, typeof Promise, Object.prototype.toString.call(p));
const r = Promise.reject(new Error('x')); r.catch(() => {});
console.log(Promise.resolve('s'));
