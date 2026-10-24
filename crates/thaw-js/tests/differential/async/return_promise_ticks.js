async function a() { return Promise.resolve('x'); }
async function b() { return 'y'; }
a().then((v) => console.log('a', v));
b().then((v) => console.log('b', v));
Promise.resolve().then(() => console.log('1')).then(() => console.log('2')).then(() => console.log('3')).then(() => console.log('4'));
