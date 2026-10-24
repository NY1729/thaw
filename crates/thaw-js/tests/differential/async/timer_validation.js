try { setTimeout('x'); } catch (e) { console.log(e.name, e.code, e.message); }
try { process.nextTick(1); } catch (e) { console.log(e.name, e.code); }
try { queueMicrotask(); } catch (e) { console.log(e.name, e.code); }
const t = setTimeout(() => {}, 1);
console.log(typeof t.ref, typeof t.unref, typeof t.hasRef, typeof t.refresh);
clearTimeout(undefined); clearTimeout(null); clearTimeout({});
