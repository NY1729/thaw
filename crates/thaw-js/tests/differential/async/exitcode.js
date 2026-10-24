process.exitCode = 5;
setTimeout(() => console.log('done'), 1);
process.on('beforeExit', () => console.log('beforeExit'));
