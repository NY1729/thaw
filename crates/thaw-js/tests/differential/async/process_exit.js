process.on('exit', (c) => console.log('exit handler', c));
setTimeout(() => console.log('never'), 10);
console.log('a');
process.exitCode = 3;
Promise.resolve().then(() => { console.log('b'); process.exit(7); });
