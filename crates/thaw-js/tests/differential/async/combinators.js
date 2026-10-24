const d = (v, ms) => new Promise((r) => setTimeout(() => r(v), ms));
const f = (e, ms) => new Promise((_, r) => setTimeout(() => r(e), ms));
(async () => {
  console.log(await Promise.all([d(1, 20), d(2, 5), 3]));
  console.log(JSON.stringify(await Promise.allSettled([d(1, 5), f("no", 1)])));
  console.log(await Promise.race([d('slow', 30), d('fast', 2)]));
  console.log(await Promise.any([f('a', 1), d('ok', 5)]));
  try { await Promise.any([f('a', 1), f('b', 2)]); } catch (e) { console.log(e.name, e.message, e.errors); }
  try { await Promise.all([d(1, 1), f('boom', 2)]); } catch (e) { console.log('caught', e); }
  console.log(await Promise.all([]));
  console.log(await Promise.allSettled([]));
})();
