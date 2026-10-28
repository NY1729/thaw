function tag(s, ...v){ return s.raw.join('|') + '#' + v.join(',') + '#' + s.length + Object.isFrozen(s); }
console.log(tag`a${1}b${2}c\n`, String.raw`x\ny${3}`);
function same(s){ return s; } const f = () => same`q`; console.log(f() === f(), same`q` === same`q`);
try { throw 1; } catch { console.log('opt'); } finally { console.log('fin'); }
const t = (s) => s[0]; console.log(t`\unicode and \u{55}`);
const obj = { m(s){ return this.n + s[0]; }, n: 'N' }; console.log(obj.m`x`);
console.log(`a${`b${'c'}`}d`);
