async function thrower() { throw new TypeError('bad'); }
async function viaAwait() { try { await thrower(); } catch (e) { console.log('caught', e.name, e.message); return 'recovered'; } }
viaAwait().then((v) => console.log(v));
async function fin() { try { await null; return 'ret'; } finally { console.log('finally'); } }
fin().then(console.log);
Promise.reject(new Error('x')).finally(() => console.log('fin-cb')).catch((e) => console.log('after finally', e.message));
Promise.resolve(5).finally(() => 99).then((v) => console.log('value kept', v));
const p = new Promise(() => { throw new Error('in executor'); });
p.catch((e) => console.log(e.message));
new Promise((res, rej) => { res(1); rej(2); res(3); }).then((v) => console.log('first wins', v));
