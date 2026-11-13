// util.format specifiers as console.log and the class print them. Each call is one printed line.
const out = [];
const sink = { write(s) { out.push(s); return true; }, on() { return this; }, once() { return this; },
  removeListener() { return this; }, emit() { return false; }, listenerCount() { return 1; } };
const c = new console.Console(sink);
c.log('%s|%d|%i|%f|%j|%o|%O|%c|%%', 'a', '7.5', '7.9', '3.25', { x: 1 }, { y: 1 }, { z: 2 }, 'css');
c.log('%d %s', 'x');
c.log('a%', 1);
c.log('%s', { a: 1 }, 'extra');
c.log(1n, null, undefined, [1, 'a'], { a: { b: 1 } });
c.log('%d %i %f', 1n, 3n, 'abc');
c.log('%s %s', 1n, null);
c.log('%j', { toJSON() { return 'custom'; } });
c.log('%s:%s', 'only');
c.log('%%s %s', 'after');
c.log('%i', -7.9, '%d');
c.log('%o', 'str');
c.log('%s', -0);
c.log(console.Console === console.Console, typeof console.Console);
console.log(JSON.stringify(out));
