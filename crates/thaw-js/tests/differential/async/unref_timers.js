const u = setTimeout(() => console.log('never'), 5000);
u.unref();
console.log(u.hasRef());
const r = setTimeout(() => console.log('runs'), 5);
console.log(r.hasRef());
setInterval(() => console.log('never iv'), 500).unref();
