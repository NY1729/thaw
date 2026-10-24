process.on('uncaughtException', (e, origin) => console.log('handled', e.message, origin));
setTimeout(() => { throw new Error('from timer'); }, 1);
setTimeout(() => console.log('still running'), 5);
