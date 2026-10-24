console.log('before');
process.on('exit', (c) => console.log('exit code', c));
Promise.reject(new Error('fatal'));
setTimeout(() => console.log('should not run'), 10);
