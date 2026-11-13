// Constructor shapes, option handling, own properties, and the errors for a missing stream.
const out = [];
function sink(tag, failing) {
  return { write(s) { if (failing) throw new Error('boom'); out.push(tag + ':' + s); return true; },
    on() { return this; }, once() { return this; }, removeListener() { return this; }, emit() { return false; }, listenerCount() { return 1; } };
}
const one = new console.Console(sink('O'));
one.log('one'); one.error('error goes to stdout');
const two = new console.Console(sink('O'), sink('E'));
two.log('two'); two.error('two error');
const opts = new console.Console({ stdout: sink('O'), stderr: sink('E'), groupIndentation: 4 });
opts.group('g'); opts.log('x\ny'); opts.groupEnd(); opts.log('after');
const onlyOut = new console.Console({ stdout: sink('O') });
onlyOut.warn('warn to stdout');
const failing = new console.Console({ stdout: sink('O', true) });
failing.log('swallowed');
out.push('still running');
const strict = new console.Console({ stdout: sink('O', true), ignoreErrors: false });
try { strict.log('x'); } catch (e) { out.push('thrown: ' + e.message); }
const badErr = (f) => { try { f(); return 'ok'; } catch (e) { return e.name + ' ' + e.code; } };
out.push(badErr(() => new console.Console()));
out.push(badErr(() => new console.Console({})));
out.push(badErr(() => new console.Console(sink('O'), 5)));
out.push(typeof console.Console, console.Console.name, console.Console.length);
out.push(Object.keys(one).join(','));
out.push(one instanceof console.Console, typeof one.Console);
const { log } = one;
log('detached');
out.push(Object.getOwnPropertyDescriptor(console, 'Console').enumerable);
out.push(Object.getOwnPropertyDescriptor(console, 'Console').writable);
console.log(JSON.stringify(out));
