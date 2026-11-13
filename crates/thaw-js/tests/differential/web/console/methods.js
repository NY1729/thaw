// Every Console method over a fake stream (Node needs listenerCount on it). Lines are printed as JSON.
const out = [];
function sink(tag) {
  return { write(s) { out.push(tag + ':' + s); return true; }, on() { return this; }, once() { return this; },
    removeListener() { return this; }, emit() { return false; }, listenerCount() { return 1; } };
}
const c = new console.Console(sink('O'), sink('E'));
c.log('value', 42, null, undefined, true);
c.info('info'); c.debug('debug');
c.warn('warn'); c.error('error', { a: 1 });
c.dir({ a: { b: 1 } });
c.dir('text');
c.dirxml('x', 2);
c.log();
c.log('');
c.log(-0, 1.5, 'a\nb');
c.assert(true, 'not printed');
c.assert(false);
c.assert(false, 'bad %s', 'x');
c.assert(0, { a: 1 });
c.assert(null, 'n', 2);
c.count(); c.count(); c.count('k'); c.count(null);
c.countReset(); c.count(); c.countReset('missing');
c.group('G1', 'x'); c.log('in'); c.group(); c.log('deeper\nmulti'); c.groupEnd(); c.groupEnd(); c.groupEnd(); c.log('out');
c.groupCollapsed('C'); c.log('cc'); c.groupEnd(); c.groupEnd(); c.log('extra end');
c.group(); c.warn('warned in group');
c.groupEnd();
c.time('t'); c.timeLog('t', 'extra'); c.timeEnd('t'); c.timeEnd('t'); c.timeLog('never');
c.clear();
const t = c.trace('tracing', 1);
console.log(JSON.stringify(out.map(s => (s.startsWith('E:Trace') ? s.split('\n')[0] + '\n' : s.replace(/\d+(\.\d+)?ms/, 'TIME')))));
