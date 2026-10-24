let p = Promise.resolve(0);
for (let i = 0; i < 200; i++) p = p.then((v) => v + 1);
p.then((v) => console.log('chain', v));
(async () => { let s = 0; for (let i = 0; i < 200; i++) s += await i; console.log('sum', s); })();
