process.on('unhandledRejection', (r, p) => { console.log('unhandled', r instanceof Error ? r.message : r, p instanceof Promise); });
Promise.reject(new Error('boom'));
Promise.reject('str');
const handled = Promise.reject(1);
handled.catch(() => console.log('late catch'));
process.on('exit', (c) => console.log('exit', c));
