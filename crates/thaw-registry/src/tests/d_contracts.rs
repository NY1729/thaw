// Behaviour contracts for assert / events / stream / diagnostics_channel / v8 / zlib / vm / readline / punycode / buffer / child_process.
// Each case runs the same JavaScript body through the bundled Thaw built-in; the expected JSON was recorded by running the identical body under real Node.js 22.

#[test]
fn d_assert_deep() {
    // expected: node22
    assert_eq!(
        p2c_run("d_assert_deep", r#####"const assert = require('node:assert');
const r = (f) => { try { f(); return 'eq'; } catch (e) { return e.code || e.name; } };
const out = {};
out.sparseLen = r(() => assert.deepStrictEqual([], new Array(3)));
out.sparseTail = r(() => assert.deepStrictEqual([1, , ,], [1, ,]));
out.mapDiff = r(() => assert.deepStrictEqual(new Map([[1, 2]]), new Map([[1, 3]])));
out.mapSame = r(() => assert.deepStrictEqual(new Map([[1, { a: 1 }]]), new Map([[1, { a: 1 }]])));
out.setDiff = r(() => assert.deepStrictEqual(new Set([1]), new Set([2])));
out.setSame = r(() => assert.deepStrictEqual(new Set([1, { a: 1 }]), new Set([{ a: 1 }, 1])));
out.setSize = r(() => assert.deepStrictEqual(new Set([1]), new Set([1, 2])));
out.errMsg = r(() => assert.deepStrictEqual(new Error('a'), new Error('b')));
out.errSame = r(() => assert.deepStrictEqual(new Error('a'), new Error('a')));
out.errName = r(() => assert.deepStrictEqual(new TypeError('a'), new Error('a')));
const a = new DataView(new ArrayBuffer(4)); const b = new DataView(new ArrayBuffer(8));
out.dvLen = r(() => assert.deepStrictEqual(a, b));
const c = new DataView(new ArrayBuffer(4)); new Uint8Array(c.buffer)[0] = 9;
out.dvBytes = r(() => assert.deepStrictEqual(a, c));
out.dvSame = r(() => assert.deepStrictEqual(a, new DataView(new ArrayBuffer(4))));
return out;
"#####),
        r#####"{"sparseLen":"ERR_ASSERTION","sparseTail":"ERR_ASSERTION","mapDiff":"ERR_ASSERTION","mapSame":"eq","setDiff":"ERR_ASSERTION","setSame":"eq","setSize":"ERR_ASSERTION","errMsg":"ERR_ASSERTION","errSame":"eq","errName":"ERR_ASSERTION","dvLen":"ERR_ASSERTION","dvBytes":"ERR_ASSERTION","dvSame":"eq"}"#####
    );
}

#[test]
fn d_assert_throws() {
    // expected: node22
    assert_eq!(
        p2c_run("d_assert_throws", r#####"const assert = require('node:assert');
const r = (f) => { try { f(); return 'ok'; } catch (e) { return e.code || e.name; } };
const out = {};
const thrower = () => { throw new TypeError('boom'); };
out.objName = r(() => assert.throws(thrower, { name: 'RangeError' }));
out.objNameOk = r(() => assert.throws(thrower, { name: 'TypeError', message: 'boom' }));
out.objMsgRe = r(() => assert.throws(thrower, { message: /bo+m/ }));
out.objMsgBad = r(() => assert.throws(thrower, { message: /zzz/ }));
out.errInst = r(() => assert.throws(thrower, new TypeError('other')));
out.errInstOk = r(() => assert.throws(thrower, new TypeError('boom')));
out.arrowValidate = r(() => assert.throws(thrower, (e) => e.message === 'boom'));
out.arrowValidateFalse = r(() => assert.throws(thrower, (e) => e.message === 'x'));
out.ctor = r(() => assert.throws(thrower, TypeError));
out.ctorBad = r(() => assert.throws(thrower, RangeError));
out.re = r(() => assert.throws(thrower, /boom/));
out.reBad = r(() => assert.throws(thrower, /nope/));
out.strictEqual16 = r(() => require('node:assert/strict').equal(1, '1'));
out.strictProp = r(() => assert.strict.equal(1, '1'));
out.strictLoose = r(() => assert.equal(1, '1'));
out.strictNotEqual = r(() => require('node:assert/strict').notEqual(1, '1'));
return out;
"#####),
        r#####"{"objName":"ERR_ASSERTION","objNameOk":"ok","objMsgRe":"ok","objMsgBad":"ERR_ASSERTION","errInst":"ERR_ASSERTION","errInstOk":"ok","arrowValidate":"ok","arrowValidateFalse":"ERR_ASSERTION","ctor":"ok","ctorBad":"ERR_ASSERTION","re":"ok","reBad":"ERR_ASSERTION","strictEqual16":"ERR_ASSERTION","strictProp":"ERR_ASSERTION","strictLoose":"ok","strictNotEqual":"ok"}"#####
    );
}

#[test]
fn d_events_once() {
    // expected: node22
    assert_eq!(
        p2c_run("d_events_once", r#####"const EventEmitter = require('node:events'); const out = {}; const T = async (k, f) => { try { out[k] = await f(); } catch (e) { out[k] = "ERR:" + (e.code || e.name); } };
await T('reentrant', async () => { const e = new EventEmitter(); const calls = []; let depth = 0;
  e.on('x', () => { calls.push('a'); if (depth++ < 1) e.emit('x'); }); e.once('x', () => calls.push('once')); e.emit('x'); return calls; });
await T('raw', async () => { const e2 = new EventEmitter(); let n = 0; const f = () => n++; e2.once('y', f);
  const raw = e2.rawListeners('y'); const a = [raw.length, raw[0].listener === f]; raw[0](); raw[0](); return [a, n, e2.listenerCount('y')]; });
await T('sameFn', async () => { const e3 = new EventEmitter(); const g = () => {}; e3.once('z', g); e3.on('z', g); e3.emit('z'); return e3.listenerCount('z'); });
await T('onClose', async () => { const e4 = new EventEmitter(); const seen = [];
  const it = EventEmitter.on(e4, 'data', { close: ['end'] });
  setTimeout(() => { e4.emit('data', 1); e4.emit('data', 2); e4.emit('end'); }, 10);
  for await (const v of it) seen.push(v); return seen; });
return out;
"#####),
        r#####"{"reentrant":["a","a","once"],"raw":[[1,true],1,0],"sameFn":1,"onClose":[[1],[2]]}"#####
    );
}

#[test]
fn d_stream_cfg() {
    // expected: node22
    assert_eq!(
        p2c_run("d_stream_cfg", r#####"const { Readable, Writable, Duplex, Transform, PassThrough } = require('node:stream');
const out = {};
// encoding option
const r = new Readable({ read() {}, encoding: 'utf8' }); r.push(Buffer.from('hi')); r.push(null);
const chunks = []; for await (const c of r) chunks.push(typeof c);
out.readableEncoding = chunks;
const t = new Transform({ encoding: 'utf8', transform(c, e, cb) { cb(null, c); } }); t.end('x');
const tc = []; for await (const c of t) tc.push(typeof c); out.transformEncoding = tc;
// writev only
const got = []; const w = new Writable({ writev(chunks, cb) { got.push(chunks.length); cb(); } });
w.write('a'); await new Promise(r => w.end(r)); out.writevOnly = got;
const w2 = new Writable({ writev(chunks, cb) { got.push('b' + chunks.length); cb(); } });
w2.cork(); w2.write('a'); w2.write('b'); w2.uncork(); await new Promise(r => w2.end(r)); out.writevCork = got.slice(1);
// duplex hwm
const d = new Duplex({ readableHighWaterMark: 3, writableHighWaterMark: 5, read() {}, write(c, e, cb) { cb(); } });
out.duplexHwm = [d.readableHighWaterMark, d.writableHighWaterMark];
const d2 = new Duplex({ highWaterMark: 7, read() {}, write(c, e, cb) { cb(); } });
out.duplexHwm2 = [d2.readableHighWaterMark, d2.writableHighWaterMark];
// autoDestroy false
let destroyed = false;
const w3 = new Writable({ autoDestroy: false, write(c, e, cb) { cb(new Error('x')); }, destroy(e, cb) { destroyed = true; cb(e); } });
w3.on('error', () => {}); w3.write('a'); await new Promise(r => setTimeout(r, 20));
out.autoDestroyFalse = destroyed;
return out;
"#####),
        r#####"{"readableEncoding":["string"],"transformEncoding":["string"],"writevOnly":[1,"b2"],"writevCork":["b2"],"duplexHwm":[3,5],"duplexHwm2":[7,7],"autoDestroyFalse":false}"#####
    );
}

#[test]
fn d_stream_order() {
    // expected: node22
    assert_eq!(
        p2c_run("d_stream_order", r#####"const { Writable, Readable, PassThrough } = require('node:stream'); const out = {}; const T = async (k, f) => { try { out[k] = await f(); } catch (e) { out[k] = "ERR:" + (e.code || e.name); } };
await T('drain', async () => { const w = new Writable({ highWaterMark: 2, write(c, e, cb) { cb(); } }); const ret = w.write('abcd');
  return [ret, await new Promise(res => { w.once('drain', () => res('drained')); setTimeout(() => res('missed'), 100); })]; });
await T('corkOrder', async () => { const w2 = new Writable({ write(c, e, cb) { cb(); } }); const log = [];
  w2.cork(); w2.write('a', () => log.push('cb-a')); w2.write('b', () => log.push('cb-b')); w2.on('finish', () => log.push('finish'));
  w2.end(); await new Promise(r => setTimeout(r, 30)); return log; });
await T('endCb', async () => { const w3 = new Writable({ emitClose: false, write(c, e, cb) { cb(); }, final(cb) { cb(new Error('F')); } }); w3.on('error', () => {});
  return await new Promise(res => { w3.end(e => res(e ? e.message : 'no error')); setTimeout(() => res('never'), 100); }); });
await T('many', async () => { const w4 = new Writable({ write(c, e, cb) { cb(); } }); w4.cork(); for (let i = 0; i < 20000; i++) w4.write('x'); w4.uncork();
  return await new Promise(res => { w4.end(() => res('ok')); setTimeout(() => res('hang'), 2000); }); });
await T('pt', async () => { const pt = new PassThrough({ highWaterMark: 2 }); let accepted = 0; for (let i = 0; i < 10; i++) { if (pt.write('ab')) accepted++; }
  return [accepted < 10, pt.writableLength, pt.readableLength]; });
return out;
"#####),
        r#####"{"drain":[true,"missed"],"corkOrder":["cb-a","cb-b","finish"],"endCb":"F","many":"ok","pt":[true,20,2]}"#####
    );
}

#[test]
fn d_stream_flow() {
    // expected: node22
    assert_eq!(
        p2c_run("d_stream_flow", r#####"const { Readable } = require('node:stream');
const out = {};
// data listener with readable listener: no auto flow
const r = new Readable({ read() {} }); const seen = [];
r.on('readable', () => { });
r.on('data', c => seen.push(String(c)));
r.push('a'); await new Promise(res => setTimeout(res, 10));
out.dataWithReadable = seen.slice(); out.flowing = r.readableFlowing;
const x = r.read(); out.readReturn = String(x); out.seenAfterRead = seen.slice();
// unshift keeps decoder state
const u = new Readable({ read() {} }); u.setEncoding('utf8');
u.push(Buffer.from([0xe3])); u.unshift('X'); u.push(Buffer.from([0x81, 0x82])); u.push(null);
let s = ''; for await (const c of u) s += c; out.unshiftDecoder = s;
// _read reentrancy
let reads = 0; const rr = new Readable({ read() { reads++; setTimeout(() => this.push(null), 20); } });
rr.read(); rr.read(); rr.read(); await new Promise(res => setTimeout(res, 50)); out.reads = reads;
// Readable.from push respects backpressure
let pulled = 0; const it = (function* () { for (let i = 0; i < 100; i++) { pulled++; yield i; } })();
const f = Readable.from(it, { highWaterMark: 2 }); f.read(0); await new Promise(res => setTimeout(res, 20));
out.pulled = pulled < 20;
// Readable.from with Uint8Array
const u8 = []; for await (const c of Readable.from(new Uint8Array([1, 2]))) u8.push(Array.isArray(c) ? 'arr' : typeof c + ':' + (c.length ?? c));
out.fromU8 = u8;
// async iterator ends on destroy mid-wait
const d = new Readable({ read() {} }); const ds = [];
setTimeout(() => d.destroy(), 20);
out.destroyMid = await new Promise(res => { (async () => { try { for await (const c of d) ds.push(c); res('ended'); } catch (e) { res('err:' + e.code); } })(); setTimeout(() => res('stuck'), 200); });
// take(1) with open stream
const t = new Readable({ read() {} }); t.push('a');
out.take1 = await new Promise(res => { t.take(1).toArray().then(v => res(v.map(String)), e => res('err')); setTimeout(() => res('stuck'), 200); });
const t0 = new Readable({ read() {} }); t0.push('a');
out.take0 = await new Promise(res => { t0.take(0).toArray().then(v => res(v.length), e => res('err')); setTimeout(() => res('stuck'), 200); });
// errored then iterate
const er = new Readable({ read() {} }); er.on('error', () => {}); er.destroy(new Error('E'));
await new Promise(res => setTimeout(res, 10));
out.erroredToArray = await er.take(1).toArray().then(() => 'resolved', e => 'rejected:' + e.message);
return out;
"#####),
        r#####"{"dataWithReadable":[],"flowing":false,"readReturn":"a","seenAfterRead":["a"],"unshiftDecoder":"Xあ","reads":1,"pulled":true,"fromU8":["number:1","number:2"],"destroyMid":"err:ERR_STREAM_PREMATURE_CLOSE","take1":["a"],"take0":0,"erroredToArray":"rejected:E"}"#####
    );
}

#[test]
fn d_stream_web() {
    // expected: node22
    assert_eq!(
        p2c_run("d_stream_web", r#####"const { Readable, Writable, Duplex } = require('node:stream');
const out = {};
const rw = Readable.toWeb(new Readable({ read() {}, emitClose: false }));
const reader = rw.getReader();
out.cancelEmitCloseFalse = await new Promise(res => { reader.cancel().then(() => res('resolved')); setTimeout(() => res('pending'), 100); });
const src = new Readable({ read() {} });
const web = Readable.toWeb(src); const rd = web.getReader();
const p = rd.read(); src.destroy();
out.destroyMid = await new Promise(res => { p.then(v => res('done:' + v.done), e => res('rej')); setTimeout(() => res('pending'), 100); });
const dst = new Writable({ write(c, e, cb) { cb(); } });
dst.on('error', () => {});
const ww = Writable.toWeb(dst); const wr = ww.getWriter();
dst.destroy(new Error('D'));
out.writableToWebErr = await new Promise(res => { wr.closed.then(() => res('closed'), e => res('rej:' + e.message)); setTimeout(() => res('pending'), 100); });
let ctl; const ws = new WritableStream({ start(c) { ctl = c; } });
const nodeW = Writable.fromWeb(ws); nodeW.on('error', () => {});
ctl.error(new Error('W'));
out.fromWebErr = await new Promise(res => { nodeW.on('close', () => res('closed:' + nodeW.destroyed)); setTimeout(() => res('pending'), 100); });
return out;
"#####),
        r#####"{"cancelEmitCloseFalse":"resolved","destroyMid":"rej","writableToWebErr":"rej:D","fromWebErr":"closed:true"}"#####
    );
}

#[test]
fn d_dc() {
    // expected: node22
    assert_eq!(
        p2c_run("d_dc", r#####"const dc = require('node:diagnostics_channel'); const out = {}; const T = async (k, f) => { try { out[k] = await f(); } catch (e) { out[k] = "ERR:" + (e.code || e.name); } };
await T('sym', async () => { const s1 = Symbol('same'), s2 = Symbol('same'); return [dc.channel(s1) !== dc.channel(s2), dc.channel(s1).name === s1, dc.channel('Symbol(same)') !== dc.channel(s1)]; });
await T('iso', async () => { const c = dc.channel('iso'); const got = []; process.once('uncaughtException', e => got.push('uncaught:' + e.message));
  c.subscribe(() => { throw new Error('S1'); }); c.subscribe(() => got.push('second')); let threw = false; try { c.publish({}); } catch (e) { threw = true; }
  await new Promise(r => setTimeout(r, 20)); return { threw, got }; });
await T('tracePromise', async () => { const tc = dc.tracingChannel('tp'); const ev = [];
  tc.subscribe({ start: () => ev.push('start'), end: () => ev.push('end'), asyncStart: () => ev.push('asyncStart'), asyncEnd: () => ev.push('asyncEnd'), error: () => ev.push('error') });
  const p = tc.tracePromise(async function () { ev.push('fn'); await null; return 1; }, {}); ev.push('after-call'); await p; await new Promise(r => setTimeout(r, 5)); return ev; });
await T('tracePromiseThrow', async () => { const ev2 = []; const tc2 = dc.tracingChannel('tp2');
  tc2.subscribe({ start: () => ev2.push('start'), end: () => ev2.push('end'), error: () => ev2.push('error') });
  try { tc2.tracePromise(() => { throw new Error('x'); }, {}); } catch (e) { ev2.push('threw'); } return ev2; });
await T('traceCallbackNeg', async () => { const tc3 = dc.tracingChannel('tcb'); const ev3 = [];
  tc3.subscribe({ start: () => ev3.push('start'), end: () => ev3.push('end'), asyncStart: () => ev3.push('asyncStart'), asyncEnd: () => ev3.push('asyncEnd'), error: () => ev3.push('error') });
  await new Promise(res => tc3.traceCallback((a, cb) => setImmediate(cb, null, 1), -1, {}, null, 5, () => res())); return ev3; });
await T('traceCallbackUndef', async () => { const tc4 = dc.tracingChannel('tcb2'); const ev4 = [];
  tc4.subscribe({ start: () => ev4.push('start'), end: () => ev4.push('end'), asyncStart: () => ev4.push('asyncStart'), asyncEnd: () => ev4.push('asyncEnd') });
  await new Promise(res => tc4.traceCallback((a, cb) => setImmediate(cb, null, 1), undefined, {}, null, 5, () => res())); return ev4; });
await T('bindStore', async () => { const { AsyncLocalStorage } = require('node:async_hooks'); const als = new AsyncLocalStorage();
  const ch = dc.channel('bs'); ch.bindStore(als, d => 'old'); ch.bindStore(als, d => 'new'); return ch.runStores({}, () => als.getStore()); });
await T('runStoresPublish', async () => { const ch2 = dc.channel('rs'); let pub = 0; ch2.subscribe(() => pub++); ch2.runStores({}, () => {}); return pub; });
return out;
"#####),
        r#####"{"sym":[true,true,true],"iso":{"threw":false,"got":["second","uncaught:S1"]},"tracePromise":["start","fn","end","after-call","asyncStart","asyncEnd"],"tracePromiseThrow":["start","error","end","threw"],"traceCallbackNeg":["start","end","asyncStart","asyncEnd"],"traceCallbackUndef":["start","end","asyncStart","asyncEnd"],"bindStore":"new","runStoresPublish":1}"#####
    );
}

#[test]
fn d_v8s() {
    // expected: node22
    assert_eq!(
        p2c_run("d_v8s", r#####"const v8 = require('node:v8');
const rt = (v) => v8.deserialize(v8.serialize(v));
const t = (f) => { try { return f(); } catch (e) { return 'ERR:' + e.name; } };
const out = {};
out.fn = t(() => { rt(() => 1); return 'no error'; });
out.sym = t(() => { rt(Symbol('x')); return 'no error'; });
out.negZero = Object.is(rt(-0), -0);
out.negZeroNested = Object.is(rt({ a: -0 }).a, -0);
out.sparse = t(() => { const a = [1, , 3]; a.length = 5; const b = rt(a); return [b.length, 1 in b, b[2]]; });
out.u16 = t(() => { const b = rt(new Uint16Array([1, 258])); return [b.constructor.name, Array.from(b)]; });
out.ab = t(() => { const b = rt(new Uint8Array([1, 2, 3]).buffer); return [b.constructor.name, b.byteLength, Array.from(new Uint8Array(b))]; });
out.dv = t(() => { const d = new DataView(new ArrayBuffer(4)); d.setInt16(0, 7); const b = rt(d); return [b.constructor.name, b.getInt16(0)]; });
out.proto = t(() => { const o = JSON.parse('{"__proto__": {"x": 1}}'); const b = rt(o); return [Object.getPrototypeOf(b) === Object.prototype, Object.keys(b)]; });
out.numWrap = t(() => { const b = rt(new Number(42)); return [typeof b, b.valueOf()]; });
out.strWrap = t(() => { const b = rt(new String('s')); return [typeof b, b.valueOf()]; });
out.boolWrap = t(() => { const b = rt(new Boolean(false)); return [typeof b, b.valueOf()]; });
out.err = t(() => { const b = rt(new RangeError('m')); return [b instanceof Error, b.name, b.message, typeof b.stack]; });
return out;
"#####),
        r#####"{"fn":"ERR:Error","sym":"ERR:Error","negZero":true,"negZeroNested":true,"sparse":[5,false,3],"u16":["Uint16Array",[1,258]],"ab":["ArrayBuffer",3,[1,2,3]],"dv":["DataView",7],"proto":[true,["__proto__"]],"numWrap":["object",42],"strWrap":["object","s"],"boolWrap":["object",false],"err":[true,"RangeError","m","string"]}"#####
    );
}

#[test]
fn d_zlib() {
    // expected: node22
    assert_eq!(
        p2c_run("d_zlib", r#####"const zlib = require('node:zlib'); const out = {}; const T = async (k, f) => { try { out[k] = await f(); } catch (e) { out[k] = "ERR:" + (e.code || e.name); } };
const data = Buffer.from('hello hello hello hello');
await T('unzipSync', async () => [zlib.unzipSync(zlib.gzipSync(data)).toString(), zlib.unzipSync(zlib.deflateSync(data)).toString()]);
await T('unzipAsync', async () => await new Promise(r => zlib.unzip(zlib.gzipSync(data), (e, b) => r(e ? 'err' : b.toString()))));
return out;
"#####),
        r#####"{"unzipSync":["hello hello hello hello","hello hello hello hello"],"unzipAsync":"hello hello hello hello"}"#####
    );
}

#[test]
fn d_vm() {
    // expected: node22
    assert_eq!(
        p2c_run("d_vm", r#####"const vm = require('node:vm');
const out = {};
const t = (f) => { try { return f(); } catch (e) { return 'ERR:' + e.name; } };
out.topLevelReturn = t(() => { new vm.Script('return 1'); return 'compiled'; });
out.codeOverwrite = t(() => { const s = new vm.Script('1+1'); s.code = '2+2'; return s.runInThisContext(); });
out.dontContextify = t(() => typeof vm.constants.DONT_CONTEXTIFY);
out.createCtxDont = t(() => { const c = vm.createContext(vm.constants.DONT_CONTEXTIFY); return typeof c; });
out.cachedRejected = t(() => new vm.Script('1', { cachedData: Buffer.from('bogus') }).cachedDataRejected);
out.compileFn = t(() => { const ctx = vm.createContext({ zz: 5 }); return vm.compileFunction('return zz + a', ['a'], { parsingContext: ctx })(1); });
out.compileExt = t(() => vm.compileFunction('return q', [], { contextExtensions: [{ q: 7 }] })());
return out;
"#####),
        r#####"{"topLevelReturn":"ERR:SyntaxError","codeOverwrite":2,"dontContextify":"symbol","createCtxDont":"object","cachedRejected":true,"compileFn":6,"compileExt":7}"#####
    );
}

#[test]
fn d_misc_small() {
    // expected: node22
    assert_eq!(
        p2c_run("d_misc_small", r#####"const out = {};
const punycode = require('node:punycode');
const t = (f) => { try { return f(); } catch (e) { return 'ERR:' + e.name; } };
out.decodeDash = t(() => punycode.decode('-'));
out.decodeBig = t(() => punycode.decode('a-9999999999999'));
out.roundtrip = punycode.encode('mañana');
out.isAsciiAb = t(() => require('node:buffer').isAscii(new Uint8Array([0xc3, 0xa9]).buffer));
out.isAsciiU16 = t(() => require('node:buffer').isAscii(new Uint16Array([0x4141])));
out.isAsciiOk = t(() => require('node:buffer').isAscii(Buffer.from('abc')));
const tty = require('node:tty');
return out;
"#####),
        r#####"{"decodeDash":"ERR:RangeError","decodeBig":"ERR:RangeError","roundtrip":"maana-pta","isAsciiAb":false,"isAsciiU16":true,"isAsciiOk":true}"#####
    );
}

#[test]
fn d_readline() {
    // expected: node22
    assert_eq!(
        p2c_run("d_readline", r#####"const readline = require('node:readline'); const { PassThrough } = require('node:stream'); const out = {}; const T = async (k, f) => { try { out[k] = await f(); } catch (e) { out[k] = "ERR:" + (e.code || e.name); } };
await T('crlfSplit', async () => { const inp = new PassThrough(); const rl = readline.createInterface({ input: inp, crlfDelay: Infinity }); const lines = []; rl.on('line', l => lines.push(l));
  inp.write('a\r'); inp.write('\nb\n'); await new Promise(r => setTimeout(r, 20)); return lines; });
await T('utf8Split', async () => { const inp2 = new PassThrough(); const rl2 = readline.createInterface({ input: inp2 }); const l2 = []; rl2.on('line', l => l2.push(l));
  const bytes = Buffer.from('あ\n'); inp2.write(bytes.subarray(0, 1)); inp2.write(bytes.subarray(1)); await new Promise(r => setTimeout(r, 20)); return l2; });
await T('questionAfterClose', async () => { const inp3 = new PassThrough(); const rl3 = readline.createInterface({ input: inp3 }); rl3.close();
  return await new Promise(res => { try { rl3.question('q?', () => res('answered')); setTimeout(() => res('pending'), 50); } catch (e) { res(e.code || e.name); } }); });
await T('finalLine', async () => { const rp = require('node:readline/promises'); const inp4 = new PassThrough(); const rl4 = rp.createInterface({ input: inp4 }); const l4 = [];
  rl4.on('line', l => l4.push(l)); inp4.write('x\ny'); inp4.end(); await new Promise(r => setTimeout(r, 30)); return l4; });
await T('listenersAfterClose', async () => { const inp5 = new PassThrough(); const rl5 = readline.createInterface({ input: inp5 }); rl5.close(); return [inp5.listenerCount('data'), inp5.listenerCount('end')]; });
return out;
"#####),
        r#####"{"crlfSplit":["a","b"],"utf8Split":["あ"],"questionAfterClose":"ERR_USE_AFTER_CLOSE","finalLine":["x","y"],"listenersAfterClose":[0,0]}"#####
    );
}

#[test]
fn d_child_left() {
    // expected: node22
    assert_eq!(
        p2c_run("d_child_left", r#####"const cp = require('node:child_process');
const out = {};
out.shell = cp.spawnSync('echo hello | tr a-z A-Z', { shell: true, encoding: 'utf8' }).stdout;
out.kill0 = await new Promise(res => { const c = cp.spawn('sleep', ['1']); const r = c.kill(0); c.kill('SIGKILL'); res(r); });
out.syncSignal = cp.spawnSync('sleep', ['5'], { timeout: 100 }).signal;
try { cp.execFileSync('sleep', ['5'], { timeout: 100 }); } catch (e) { out.execSyncSignal = e.signal; }
out.abortKills = await new Promise(res => { const ac = new AbortController(); const c = cp.spawn('sleep', ['5'], { signal: ac.signal }); c.on('error', () => {}); c.on('exit', (code, sig) => res(sig)); ac.abort(); setTimeout(() => res('alive'), 2000); });
return out;
"#####),
        r#####"{"shell":"HELLO\n","kill0":true,"syncSignal":"SIGTERM","execSyncSignal":"SIGTERM","abortKills":"SIGTERM"}"#####
    );
}

#[test]
fn d_tty() {
    // expected: node22
    assert_eq!(
        p2c_run("d_tty", r#####"const tty = require('node:tty'); const out = {};
out.bad = [tty.isatty(99), tty.isatty(-1), tty.isatty('x'), tty.isatty(1.5)];
return out;
"#####),
        r#####"{"bad":[false,false,false,false]}"#####
    );
}

#[test]
fn d_inspector_once() {
    // expected: node22
    assert_eq!(
        p2c_run("d_inspector_once", r#####"const out = {}; const T = async (k, f) => { try { out[k] = await f(); } catch (e) { out[k] = "ERR:" + (e.code || e.name); } };
await T('inspectorCallbackOnce', async () => { const inspector = require('node:inspector'); const s = new inspector.Session(); s.connect(); let calls = 0;
  const p = new Promise(res => { s.post('Runtime.evaluate', { expression: '1+1' }, (err, r) => { calls++; if (calls === 1) { setTimeout(() => res(calls), 20); throw new Error('cb'); } }); });
  process.once('uncaughtException', () => {}); return await p; });
await T('perfHistogram', async () => { const { monitorEventLoopDelay } = require('node:perf_hooks'); const h = monitorEventLoopDelay({ resolution: 10 }); h.enable(); await new Promise(r => setTimeout(r, 60)); h.disable(); return [typeof h.min, typeof h.max, typeof h.mean, typeof h.percentile(50), h.count > 0]; });
return out;
"#####),
        r#####"{"inspectorCallbackOnce":1,"perfHistogram":["number","number","number","number",true]}"#####
    );
}
