const thenable = { then(res) { console.log('then called'); res(42); } };
Promise.resolve(thenable).then((v) => console.log('resolved', v));
(async () => { const v = await thenable; console.log('await', v); })();
Promise.resolve().then(() => console.log('tick1')).then(() => console.log('tick2')).then(() => console.log('tick3'));
