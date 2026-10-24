const log = [];
const sleep = (ms, v) => new Promise((r) => setTimeout(() => r(v), ms));
async function worker(name, ms) { for (let i = 0; i < 3; i++) { await sleep(ms); console.log(name, i); } return name; }
Promise.all([worker("A", 10), worker("B", 100)]).then((r) => console.log('done', r));
