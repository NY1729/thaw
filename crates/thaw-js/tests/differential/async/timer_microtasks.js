setTimeout(() => { console.log('A'); Promise.resolve().then(() => console.log('A-micro')); process.nextTick(() => console.log('A-tick')); }, 1);
setTimeout(() => { console.log('B'); Promise.resolve().then(() => console.log('B-micro')); }, 1);
