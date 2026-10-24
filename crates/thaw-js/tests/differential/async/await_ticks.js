async function f1() { console.log('f1 start'); await f2(); console.log('f1 end'); }
async function f2() { console.log('f2'); }
f1();
new Promise((res) => { console.log('p'); res(); }).then(() => console.log('then1')).then(() => console.log('then2')).then(() => console.log('then3'));
console.log('end');
