// Behaviour contracts for wave J (zlib / stream / readline / child_process / tty / perf / tls / net / http2 leftovers).
// Each case runs the same JavaScript body through the bundled Thaw built-in; the expected JSON was recorded by running the identical body under real Node.js 22.


#[test]
fn j_zlib_options() {
    // expected: node22
    assert_eq!(
        p2c_run("j_zlib_options", r#####"const zlib = require('node:zlib');
const out = {};
const data = Buffer.from('hello hello hello hello hello hello hello hello');
out.level0Bigger = zlib.deflateSync(data, { level: 0 }).length > data.length;
out.level9Smaller = zlib.deflateSync(data, { level: 9 }).length < data.length;
out.levelsDiffer = zlib.deflateSync(data, { level: 0 }).length !== zlib.deflateSync(data, { level: 9 }).length;
const dictionary = Buffer.from('hello world dictionary');
const withDict = zlib.deflateSync(data, { dictionary });
out.dictRoundTrip = zlib.inflateSync(withDict, { dictionary }).toString() === data.toString();
try { zlib.inflateSync(withDict); out.noDict = 'no throw'; } catch (e) { out.noDict = e.code; }
try { zlib.inflateSync(withDict, { dictionary: Buffer.from('wrong') }); out.badDict = 'no throw'; } catch (e) { out.badDict = e.code; }
try { zlib.deflateSync(data, { level: 99 }); out.badLevel = 'no throw'; } catch (e) { out.badLevel = e.code; }
const big = zlib.deflateSync(Buffer.alloc(100000, 97));
try { zlib.inflateSync(big, { maxOutputLength: 10 }); out.maxOut = 'no throw'; } catch (e) { out.maxOut = e.code; }
out.maxOutOk = zlib.inflateSync(big, { maxOutputLength: 200000 }).length;
return out;
"#####),
        r#####"{"level0Bigger":true,"level9Smaller":true,"levelsDiffer":true,"dictRoundTrip":true,"noDict":"Z_NEED_DICT","badDict":"Z_NEED_DICT","badLevel":"ERR_OUT_OF_RANGE","maxOut":"ERR_BUFFER_TOO_LARGE","maxOutOk":100000}"#####
    );
}

#[test]
fn j_zlib_incremental() {
    // expected: node22
    assert_eq!(
        p2c_run("j_zlib_incremental", r#####"const zlib = require('node:zlib');
const { once } = require('node:events');
const out = {};
const wait = (ms) => new Promise(r => setTimeout(r, ms));
const z = zlib.createDeflate();
const chunks = [];
z.on('data', (c) => chunks.push(c));
z.write('a'.repeat(1000));
await new Promise(r => z.flush(r));
await wait(20);
out.dataBeforeEnd = chunks.length > 0;
out.bytesWritten = z.bytesWritten;
z.write('b'.repeat(500));
z.end();
await once(z, 'end');
const total = Buffer.concat(chunks);
out.roundTrip = zlib.inflateSync(total).toString() === 'a'.repeat(1000) + 'b'.repeat(500);
const g = zlib.createGunzip();
const parts = [];
g.on('data', (c) => parts.push(c));
const gz = zlib.gzipSync('x'.repeat(5000));
g.write(gz.subarray(0, 10));
await wait(20);
out.noEarlyOutput = parts.length === 0;
g.end(gz.subarray(10));
await once(g, 'end');
out.gunzipTotal = Buffer.concat(parts).length;
const bad = zlib.createInflate();
bad.on('error', () => {});
const errP = once(bad, 'error').catch(() => []);
bad.write(Buffer.from('not zlib data at all'));
const [err] = await errP;
out.errCode = err && err.code;
out.constants = [zlib.constants.Z_NO_FLUSH, zlib.constants.Z_SYNC_FLUSH, zlib.constants.Z_FULL_FLUSH, zlib.constants.Z_FINISH];
return out;
"#####),
        r#####"{"dataBeforeEnd":true,"bytesWritten":1000,"roundTrip":true,"noEarlyOutput":true,"gunzipTotal":5000,"errCode":"Z_DATA_ERROR","constants":[0,2,3,4]}"#####
    );
}

#[test]
fn j_zlib_dictionary() {
    // expected: node22
    assert_eq!(
        p2c_run("j_zlib_dictionary", r#####"const zlib = require('node:zlib');
const source = Buffer.from('dictionary guard'), dictionary = Buffer.from('dict');
function capture(fn) { try { fn(); return null; } catch (error) { return error; } }
const packed = zlib.deflateSync(source, { dictionary });
const raw = zlib.deflateRawSync(source, { dictionary });
const roundTrip = [zlib.inflateSync(packed, { dictionary }).toString(), zlib.inflateRawSync(raw, { dictionary }).toString()];
const missing = capture(() => zlib.inflateSync(packed));
const sentinel = new Error('dictionary getter');
let reads = 0;
const throwingOptions = { get dictionary() { reads++; throw sentinel; } };
const getterSame = capture(() => zlib.inflateSync(source, throwingOptions)) === sentinel && reads === 1;
reads = 0;
const getterCtor = capture(() => new zlib.Deflate(throwingOptions)) === sentinel && reads === 1;
const inherited = zlib.inflateSync(zlib.deflateSync(source, Object.create({ dictionary })), { dictionary }).toString();
const streamRoundTrip = await new Promise((resolve, reject) => {
  const encoder = zlib.createDeflate({ dictionary }), decoder = zlib.createInflate({ dictionary }), parts = [];
  decoder.on('data', (c) => parts.push(c)); decoder.on('end', () => resolve(Buffer.concat(parts).toString())); decoder.on('error', reject);
  encoder.pipe(decoder); encoder.end(source);
});
const asyncResult = await new Promise((resolve) => {
  zlib.deflate(source, { dictionary }, (e1, value) => zlib.inflate(value, { dictionary }, (e2, v2) => resolve([e1, e2, String(v2)])));
});
const undefinedDictionary = zlib.inflateSync(zlib.deflateSync(source, { dictionary: undefined })).toString();
return { roundTrip, missing: missing && missing.code, getterSame, getterCtor, inherited, streamRoundTrip, asyncResult, undefinedDictionary };
"#####),
        r#####"{"roundTrip":["dictionary guard","dictionary guard"],"missing":"Z_NEED_DICT","getterSame":true,"getterCtor":true,"inherited":"dictionary guard","streamRoundTrip":"dictionary guard","asyncResult":[null,null,"dictionary guard"],"undefinedDictionary":"dictionary guard"}"#####
    );
}

#[test]
fn j_st_map_conc() {
    // expected: node22
    assert_eq!(
        p2c_run("j_st_map_conc", r#####"const S = require('node:stream');
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const race = (p, ms = 300) => Promise.race([p, sleep(ms).then(() => 'TIMEOUT')]);
const r=new S.Readable({objectMode:true,read(){}});r.push(1);const it=r.map(x=>x*2,{concurrency:2})[Symbol.asyncIterator]();return (await race(it.next()));
"#####),
        r#####"{"value":2,"done":false}"#####
    );
}

#[test]
fn j_st_pipe_dest_closed() {
    // expected: node22
    assert_eq!(
        p2c_run("j_st_pipe_dest_closed", r#####"const S = require('node:stream');
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const race = (p, ms = 300) => Promise.race([p, sleep(ms).then(() => 'TIMEOUT')]);
const src=new S.Readable({read(){}});const slow=new S.Writable({highWaterMark:1,write(c,e,cb){}});const fast=[];const w2=new S.Writable({write(c,e,cb){fast.push(String(c));cb()}});src.pipe(slow);src.pipe(w2);src.push('a');src.push('b');await sleep(10);slow.on('error',()=>{});slow.destroy();src.push('c');await sleep(30);return 'w2='+fast.join('');
"#####),
        r#####""w2=abc""#####
    );
}

#[test]
fn j_st_from_iter_destroy() {
    // expected: node22
    assert_eq!(
        p2c_run("j_st_from_iter_destroy", r#####"const S = require('node:stream');
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const race = (p, ms = 300) => Promise.race([p, sleep(ms).then(() => 'TIMEOUT')]);
let returned=false;const it={[Symbol.asyncIterator](){return {next(){return new Promise(()=>{})},return(){returned=true;return Promise.resolve({done:true})}}}};const r=S.Readable.from(it);r.on('error',()=>{});r.resume();await sleep(10);r.destroy();await sleep(30);return 'return called='+returned;
"#####),
        r#####""return called=true""#####
    );
}

#[test]
fn j_st_take_after_return() {
    // expected: node22
    assert_eq!(
        p2c_run("j_st_take_after_return", r#####"const S = require('node:stream');
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const race = (p, ms = 300) => Promise.race([p, sleep(ms).then(() => 'TIMEOUT')]);
const r=new S.Readable({objectMode:true,read(){}});r.push(1);r.push(2);const it=r[Symbol.asyncIterator]({destroyOnReturn:false});await it.return();const n=await it.next();return (n);
"#####),
        r#####"{"done":true}"#####
    );
}

#[test]
fn j_st_fromweb_err() {
    // expected: node22
    assert_eq!(
        p2c_run("j_st_fromweb_err", r#####"const S = require('node:stream');
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const race = (p, ms = 300) => Promise.race([p, sleep(ms).then(() => 'TIMEOUT')]);
let ctl;const rs=new ReadableStream({start(c){ctl=c;c.enqueue(new Uint8Array(100))}});const r=S.Readable.fromWeb(rs,{highWaterMark:1});let ev='none';r.on('error',e=>ev='error');await sleep(20);ctl.error(new Error('e'));await sleep(40);return ev;
"#####),
        r#####""error""#####
    );
}

#[test]
fn j_rl_late_iterator() {
    // expected: node22
    assert_eq!(
        p2c_run("j_rl_late_iterator", r#####"const readline = require('node:readline');
const { PassThrough } = require('node:stream');
const input = new PassThrough();
const rl = readline.createInterface({ input, crlfDelay: Infinity });
const early = [];
rl.on('line', (l) => early.push(l));
input.write('a\nb\n');
await new Promise(r => setTimeout(r, 20));
const it = rl[Symbol.asyncIterator]();
input.write('c\n');
await new Promise(r => setTimeout(r, 20));
input.end();
const late = [];
for await (const l of it) late.push(l);
return { early, late };
"#####),
        r#####"{"early":["a","b","c"],"late":["c"]}"#####
    );
}

#[test]
fn j_rl_end_flush() {
    // expected: node22
    assert_eq!(
        p2c_run("j_rl_end_flush", r#####"const rlp = require('node:readline/promises');
const rlc = require('node:readline');
const { PassThrough } = require('node:stream');
const out = {};
for (const [name, mod] of [['cb', rlc], ['promises', rlp]]) {
  const input = new PassThrough();
  const rl = mod.createInterface({ input });
  const lines = [], events = [];
  rl.on('line', (l) => { lines.push(l); events.push('line'); });
  rl.on('close', () => events.push('close'));
  input.end('first\nlast-without-newline');
  await new Promise(r => setTimeout(r, 30));
  out[name] = { lines, events };
}
return out;
"#####),
        r#####"{"cb":{"lines":["first","last-without-newline"],"events":["line","line","close"]},"promises":{"lines":["first","last-without-newline"],"events":["line","line","close"]}}"#####
    );
}

#[test]
fn j_worker_error_props() {
    // expected: node22
    assert_eq!(
        p2c_run("j_worker_error_props", r#####"const { Worker } = require('node:worker_threads');
const res = await new Promise((resolve) => {
  const w = new Worker('const e = new TypeError("boom", { cause: new RangeError("inner") }); e.code = "E_X"; throw e;', { eval: true });
  w.on('error', (e) => resolve({ name: e.name, message: e.message, code: e.code, isTypeError: e instanceof TypeError, hasStack: typeof e.stack === 'string' && e.stack.includes('boom'), cause: e.cause && [e.cause.name, e.cause.message] }));
  w.on('exit', () => setTimeout(() => resolve({ noError: true }), 50));
});
return res;
"#####),
        r#####"{"name":"TypeError","message":"boom","code":"E_X","isTypeError":true,"hasStack":true,"cause":["RangeError","inner"]}"#####
    );
}

#[test]
fn j_worker_error_late() {
    // expected: node22
    assert_eq!(
        p2c_run("j_worker_error_late", r#####"const { Worker } = require('node:worker_threads');
const res = await new Promise((resolve) => {
  const w = new Worker('require("node:worker_threads").parentPort.on("message", () => { throw new SyntaxError("late"); });', { eval: true });
  w.on('error', (e) => resolve({ name: e.name, message: e.message, isSyntax: e instanceof SyntaxError }));
  w.on('online', () => w.postMessage(1));
  setTimeout(() => resolve({ timeout: true }), 3000);
});
return res;
"#####),
        r#####"{"name":"SyntaxError","message":"late","isSyntax":true}"#####
    );
}

#[test]
fn j_worker_clone_getter() {
    // expected: node22
    assert_eq!(
        p2c_run("j_worker_clone_getter", r#####"const { Worker } = require('node:worker_threads');
let reads = 0;
const w = new Worker('const { parentPort } = require("node:worker_threads"); parentPort.on("message", (m) => { parentPort.postMessage(m); });', { eval: true });
const got = await new Promise((resolve) => {
  w.on('message', resolve);
  w.postMessage({ get a() { reads++; return 1; }, b: 2 });
});
await w.terminate();
return { reads, got };
"#####),
        r#####"{"reads":1,"got":{"a":1,"b":2}}"#####
    );
}

#[test]
fn j_child_utf8_split() {
    // expected: node22
    assert_eq!(
        p2c_run("j_child_utf8_split", r#####"const cp = require('node:child_process');
const out = {};
const script = 'printf "\\342\\202"; sleep 0.1; printf "\\254 end"';
out.setEncoding = await new Promise((resolve) => {
  const c = cp.spawn('sh', ['-c', script]);
  let s = '';
  c.stdout.setEncoding('utf8');
  c.stdout.on('data', (d) => { s += d; });
  c.on('close', () => resolve(s));
});
out.exec = await new Promise((resolve) => {
  cp.exec(script, { encoding: 'utf8' }, (e, stdout) => resolve(stdout));
});
out.execFile = await new Promise((resolve) => {
  cp.execFile('sh', ['-c', script], (e, stdout) => resolve(stdout));
});
return out;
"#####),
        r#####"{"setEncoding":"€ end","exec":"€ end","execFile":"€ end"}"#####
    );
}

#[test]
fn j_child_ipc_utf8_split() {
    // expected: thaw-only
    assert_eq!(
        p2c_run("j_child_ipc_utf8_split", r#####"// thaw-only: {"messages":[{"t":"€"},{"t":"€"}],"events":["disconnect"]}
const cp = require('node:child_process');
const host = globalThis;
const previous = { spawn: host.__thaw_child_process_spawn, stdin: host.__thaw_child_process_stdin, poll: host.__thaw_child_process_poll, active: host.__thaw_child_process_active };
const hex = (bytes) => Buffer.from(bytes).toString('hex');
const prefix = [...Buffer.from('__THAW_IPC__{"t":"')];
const suffix = [...Buffer.from('"}\n')];
const euro = [0xe2, 0x82, 0xac];
const batches = [
  [{ type: 'spawn', pid: 7 }, { type: 'stdout', value: hex([...prefix, 0xe2, 0x82]) }],
  [{ type: 'stdout', value: hex([0xac, ...suffix, ...prefix, 0xe2]) }],
  [{ type: 'stdout', value: hex([0x82, 0xac, ...suffix]) }, { type: 'exit', code: 0, signal: null }],
];
try {
  let handle = 0, polls = 0;
  host.__thaw_child_process_spawn = function () { handle = 91; return handle; };
  host.__thaw_child_process_stdin = function () {};
  host.__thaw_child_process_active = function () { return polls < batches.length; };
  host.__thaw_child_process_poll = function () { const batch = batches[polls++]; return JSON.stringify(batch ? batch.map((e) => ({ handle, ...e })) : []); };
  const child = cp.fork('/nowhere.js', [], { silent: true });
  const messages = [], events = [];
  child.on('message', (m) => messages.push(m));
  child.on('disconnect', () => events.push('disconnect'));
  await new Promise((resolve) => child.on('close', resolve));
  return { messages, events };
} finally {
  host.__thaw_child_process_spawn = previous.spawn; host.__thaw_child_process_stdin = previous.stdin; host.__thaw_child_process_poll = previous.poll; host.__thaw_child_process_active = previous.active;
}
"#####),
        r#####"{"messages":[{"t":"€"},{"t":"€"}],"events":["disconnect"]}"#####
    );
}

#[test]
fn j_child_listener_throw_stub() {
    // expected: thaw-only
    assert_eq!(
        p2c_run("j_child_listener_throw_stub", r#####"// thaw-only: {"caught":["exit-listener"],"otherClosed":true,"firstClosed":false}
const cp = require('node:child_process');
const host = globalThis;
const previous = { spawn: host.__thaw_child_process_spawn, stdin: host.__thaw_child_process_stdin, poll: host.__thaw_child_process_poll, active: host.__thaw_child_process_active };
const caught = [];
const handler = (e) => caught.push(e.message);
process.on('uncaughtException', handler);
try {
  let next = 60, polls = 0;
  const handles = [];
  host.__thaw_child_process_spawn = function () { const h = next++; handles.push(h); return h; };
  host.__thaw_child_process_stdin = function () {};
  host.__thaw_child_process_active = function () { return polls < 2; };
  host.__thaw_child_process_poll = function () {
    polls++;
    if (polls !== 1) return '[]';
    return JSON.stringify(handles.flatMap((h) => [{ handle: h, type: 'exit', code: 0, signal: null }]));
  };
  const a = cp.spawn('a'), b = cp.spawn('b');
  let firstClosed = false, otherClosed = false;
  a.on('exit', () => { throw new Error('exit-listener'); });
  a.on('close', () => { firstClosed = true; });
  b.on('close', () => { otherClosed = true; });
  await new Promise((resolve) => setTimeout(resolve, 100));
  return { caught, otherClosed, firstClosed };
} finally {
  process.removeListener('uncaughtException', handler);
  host.__thaw_child_process_spawn = previous.spawn; host.__thaw_child_process_stdin = previous.stdin; host.__thaw_child_process_poll = previous.poll; host.__thaw_child_process_active = previous.active;
}
"#####),
        r#####"{"caught":["exit-listener"],"otherClosed":true,"firstClosed":false}"#####
    );
}

#[test]
fn j_child_fork_execargv() {
    // expected: thaw-only
    assert_eq!(
        p2c_run("j_child_fork_execargv", r#####"// thaw-only: {"inherited":true,"explicit":true,"override":false}
const cp = require('node:child_process');
const host = globalThis;
const previous = { spawn: host.__thaw_child_process_spawn, stdin: host.__thaw_child_process_stdin, poll: host.__thaw_child_process_poll, active: host.__thaw_child_process_active };
const savedExecArgv = process.execArgv;
const seen = [];
try {
  host.__thaw_child_process_spawn = function (command, args) { seen.push(args); return 5; };
  host.__thaw_child_process_stdin = function () {};
  host.__thaw_child_process_active = function () { return false; };
  host.__thaw_child_process_poll = function () { return '[]'; };
  process.execArgv = ['--inherited-flag'];
  cp.fork('/nowhere.js');
  cp.fork('/nowhere.js', [], { execArgv: ['--explicit-flag'] });
  const has = (i, flag) => String(seen[i]).includes(flag);
  return { inherited: has(0, '--inherited-flag'), explicit: has(1, '--explicit-flag'), override: has(1, '--inherited-flag') };
} finally {
  process.execArgv = savedExecArgv;
  host.__thaw_child_process_spawn = previous.spawn; host.__thaw_child_process_stdin = previous.stdin; host.__thaw_child_process_poll = previous.poll; host.__thaw_child_process_active = previous.active;
}
"#####),
        r#####"{"inherited":true,"explicit":true,"override":false}"#####
    );
}

#[test]
fn j_perf_histogram() {
    // expected: node22
    assert_eq!(
        p2c_run("j_perf_histogram", r#####"const { createHistogram } = require('node:perf_hooks');
const out = {};
const snap = (h, ps) => ({ count: h.count, min: h.min, max: h.max, mean: h.mean, stddev: h.stddev, exceeds: h.exceeds, percentiles: [...h.percentiles], p: ps.map((p) => h.percentile(p)) });
const small = createHistogram();
for (let v = 1; v <= 10; v++) small.record(v);
out.small = snap(small, [1, 25, 50, 90, 99, 100]);
const wide = createHistogram();
for (const v of [100000, 2047, 2048, 5000, 123456789, 5000, 77, 2049, 4095, 4096]) wide.record(v);
out.wide = snap(wide, [10, 50, 75, 99.9, 100]);
const empty = createHistogram();
out.empty = { count: empty.count, min: empty.min, max: empty.max, mean: String(empty.mean), stddev: String(empty.stddev), percentiles: [...empty.percentiles], p50: empty.percentile(50) };
out.bigint = { min: String(small.minBigInt), max: String(small.maxBigInt), count: String(small.countBigInt), p50: String(small.percentileBigInt(50)), emptyMin: String(empty.minBigInt) };
let bad = [];
for (const p of [0, 101, '5']) { try { small.percentile(p); bad.push('ok'); } catch (e) { bad.push(e.code); } }
out.bad = bad;
// large sample counts must stay cheap: 300k records over a few thousand buckets
const big = createHistogram();
let x = 12345;
for (let i = 0; i < 300000; i++) { x = (x * 1103515245 + 12345) % 2147483648; big.record((x % 1000000) + 1); }
out.bigCount = big.count;
out.bigPercentiles = [big.percentile(50), big.percentile(99), big.min, big.max, Math.round(big.mean), Math.round(big.stddev)];
small.reset();
out.reset = { count: small.count, max: small.max };
return out;
"#####),
        r#####"{"small":{"count":10,"min":1,"max":10,"mean":5.5,"stddev":2.8722813232690143,"exceeds":0,"percentiles":[[0,1],[50,5],[75,8],[87.5,9],[93.75,10],[100,10]],"p":[1,3,5,9,10,10]},"wide":{"count":10,"min":77,"max":123469823,"mean":12356147.5,"stddev":37026980.81092374,"exceeds":0,"percentiles":[[0,77],[50,4094],[75,5000],[87.5,99968],[93.75,123404288],[100,123404288]],"p":[77,4095,5003,123469823,123469823]},"empty":{"count":0,"min":9223372036854776000,"max":0,"mean":"NaN","stddev":"NaN","percentiles":[[100,0]],"p50":0},"bigint":{"min":"1","max":"10","count":"10","p50":"5","emptyMin":"9223372036854775807"},"bad":["ERR_OUT_OF_RANGE","ERR_OUT_OF_RANGE","ERR_INVALID_ARG_TYPE"],"bigCount":300000,"bigPercentiles":[497663,989695,1,999935,499997,287717],"reset":{"count":0,"max":0}}"#####
    );
}

#[test]
fn j_vm_script_compile() {
    // expected: node22
    assert_eq!(
        p2c_run("j_vm_script_compile", r#####"const vm = require('node:vm');
const out = {};
try { new vm.Script('return 1'); out.topReturn = 'no throw'; } catch (e) { out.topReturn = e.name; }
const s = new vm.Script('1 + 1', { filename: 'one.js' });
out.codeProp = typeof s.code;
out.run = s.runInThisContext();
try { new vm.Script('throw new Error("boom")', { filename: 'my-file.js' }).runInThisContext(); } catch (e) { out.stackFile = String(e.stack).includes('my-file.js'); }
try { new vm.Script('let x = ;', { filename: 'bad.js' }); } catch (e) { out.syntaxName = e.name; out.syntaxFile = String(e.stack).includes('bad.js'); }
out.keys = Object.keys(s);
return out;
"#####),
        r#####"{"topReturn":"SyntaxError","codeProp":"undefined","run":2,"stackFile":true,"syntaxName":"SyntaxError","syntaxFile":true,"keys":["sourceURL","sourceMapURL"]}"#####
    );
}

#[test]
fn j_v8_heap_growth() {
    // expected: node22
    assert_eq!(
        p2c_run("j_v8_heap_growth", r#####"const v8 = require('node:v8');
const before = v8.getHeapStatistics();
const keep = [];
for (let i = 0; i < 200; i++) keep.push(new Array(50000).fill(i));
const after = v8.getHeapStatistics();
const out = {
  keys: ['total_heap_size', 'used_heap_size', 'heap_size_limit', 'total_available_size'].every((k) => typeof after[k] === 'number'),
  grew: after.used_heap_size > before.used_heap_size + 10 * 1024 * 1024,
  usedLeTotal: after.used_heap_size <= after.total_heap_size,
};
return out;
"#####),
        r#####"{"keys":true,"grew":true,"usedLeTotal":true}"#####
    );
}

#[test]
fn j_vm_measure_memory() {
    // expected: thaw-only
    assert_eq!(
        p2c_run("j_vm_measure_memory", r#####"// thaw-only: {"summary":[["total"],"number",true],"detailed":["total","current","other"],"bad":"ERR_INVALID_ARG_VALUE"}
// (node 22 resolves vm.measureMemory() only from its message loop, so the shape below is the documented one)
const vm = require('node:vm');
const m = await vm.measureMemory();
const d = await vm.measureMemory({ mode: 'detailed' });
let bad;
try { await vm.measureMemory({ mode: 'bogus' }); bad = 'ok'; } catch (e) { bad = e.code; }
return { summary: [Object.keys(m), typeof m.total.jsMemoryEstimate, Array.isArray(m.total.jsMemoryRange)], detailed: Object.keys(d), bad };
"#####),
        r#####"{"summary":[["total"],"number",true],"detailed":["total","current","other"],"bad":"ERR_INVALID_ARG_VALUE"}"#####
    );
}

#[test]
fn j_net_error_kinds() {
    // expected: node22
    assert_eq!(
        p2c_run("j_net_error_kinds", r#####"const net = require('node:net');
const http = require('node:http');
const out = {};
const connectErr = (opts) => new Promise((resolve) => {
  const s = net.connect(opts);
  s.on('error', (e) => resolve({ code: e.code, syscall: e.syscall, errno: e.errno, hostname: e.hostname, address: e.address, port: e.port, message: e.message }));
  s.on('connect', () => { s.destroy(); resolve('connected'); });
});
out.refused = await connectErr({ host: '127.0.0.1', port: 1 });
out.nxdomain = await connectErr({ host: 'definitely-not-a-real-host.invalid', port: 80 });
out.httpRefused = await new Promise((resolve) => {
  const r = http.get({ host: '127.0.0.1', port: 1, path: '/' });
  r.on('error', (e) => resolve({ code: e.code, syscall: e.syscall, address: e.address, port: e.port, message: e.message }));
});
out.httpNx = await new Promise((resolve) => {
  const r = http.get({ host: 'definitely-not-a-real-host.invalid', port: 80, path: '/' });
  r.on('error', (e) => resolve({ code: e.code, syscall: e.syscall, hostname: e.hostname, message: e.message }));
});
const tls = require('node:tls');
out.tlsRefused = await new Promise((resolve) => { const s = tls.connect({ host: '127.0.0.1', port: 1 }); s.on('error', (e) => resolve({ code: e.code, syscall: e.syscall, address: e.address, port: e.port })); });
out.tlsNx = await new Promise((resolve) => { const s = tls.connect({ host: 'definitely-not-a-real-host.invalid', port: 443 }); s.on('error', (e) => resolve({ code: e.code, syscall: e.syscall, hostname: e.hostname })); });
return out;
"#####),
        r#####"{"refused":{"code":"ECONNREFUSED","syscall":"connect","errno":-111,"address":"127.0.0.1","port":1,"message":"connect ECONNREFUSED 127.0.0.1:1"},"nxdomain":{"code":"ENOTFOUND","syscall":"getaddrinfo","errno":-3008,"hostname":"definitely-not-a-real-host.invalid","message":"getaddrinfo ENOTFOUND definitely-not-a-real-host.invalid"},"httpRefused":{"code":"ECONNREFUSED","syscall":"connect","address":"127.0.0.1","port":1,"message":"connect ECONNREFUSED 127.0.0.1:1"},"httpNx":{"code":"ENOTFOUND","syscall":"getaddrinfo","hostname":"definitely-not-a-real-host.invalid","message":"getaddrinfo ENOTFOUND definitely-not-a-real-host.invalid"},"tlsRefused":{"code":"ECONNREFUSED","syscall":"connect","address":"127.0.0.1","port":1},"tlsNx":{"code":"ENOTFOUND","syscall":"getaddrinfo","hostname":"definitely-not-a-real-host.invalid"}}"#####
    );
}

#[test]
fn j_fs_cp_mode() {
    // expected: node22
    assert_eq!(
        p2c_run("j_fs_cp_mode", r#####"const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cpmode-'));
fs.writeFileSync(path.join(dir, 'a.txt'), 'hello');
const out = {};
fs.cpSync(path.join(dir, 'a.txt'), path.join(dir, 'b.txt'));
out.plain = fs.readFileSync(path.join(dir, 'b.txt'), 'utf8');
try { fs.cpSync(path.join(dir, 'a.txt'), path.join(dir, 'b.txt'), { force: false, errorOnExist: true }); out.errorOnExist = 'no throw'; } catch (e) { out.errorOnExist = [e.code, e.name, e.errno, e.syscall, e.path === path.join(dir, 'b.txt'), e.message.replace(new RegExp(dir.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g'), 'D'), Object.keys(e)]; }
try { fs.cpSync(path.join(dir, 'a.txt'), path.join(dir, 'c.txt'), { mode: fs.constants.COPYFILE_EXCL }); out.excl1 = fs.readFileSync(path.join(dir, 'c.txt'), 'utf8'); } catch (e) { out.excl1 = e.code; }
try { fs.cpSync(path.join(dir, 'a.txt'), path.join(dir, 'b.txt'), { mode: fs.constants.COPYFILE_EXCL, force: false, errorOnExist: true }); out.excl3 = 'no throw'; } catch (e) { out.excl3 = e.code; }
try { fs.cpSync(path.join(dir, 'a.txt'), path.join(dir, 'c.txt'), { mode: fs.constants.COPYFILE_EXCL, force: true }); out.excl2 = 'no throw'; } catch (e) { out.excl2 = e.code; }
fs.mkdirSync(path.join(dir, 'tree'));
fs.writeFileSync(path.join(dir, 'tree', 'x.txt'), 'x');
fs.cpSync(path.join(dir, 'tree'), path.join(dir, 'tree2'), { recursive: true });
out.tree = fs.readdirSync(path.join(dir, 'tree2'));
try { fs.cpSync(path.join(dir, 'tree'), path.join(dir, 'tree3')); out.dirNoRecursive = 'no throw'; } catch (e) { out.dirNoRecursive = e.code; }
const norm = (e) => ({ code: e.code, name: e.name, message: e.message.split(dir).join('D') });
const probe = (f) => { try { f(); return 'no throw'; } catch (e) { return norm(e); } };
fs.mkdirSync(path.join(dir, 'dir', 'sub'), { recursive: true });
out.same = probe(() => fs.cpSync(path.join(dir, 'a.txt'), path.join(dir, 'a.txt')));
out.intoSelf = probe(() => fs.cpSync(path.join(dir, 'dir'), path.join(dir, 'dir', 'sub', 'x'), { recursive: true }));
out.dirOverFile = probe(() => fs.cpSync(path.join(dir, 'dir'), path.join(dir, 'a.txt'), { recursive: true }));
out.fileOverDir = probe(() => fs.cpSync(path.join(dir, 'a.txt'), path.join(dir, 'dir')));
fs.rmSync(dir, { recursive: true, force: true });
return out;
"#####),
        r#####"{"plain":"hello","errorOnExist":["ERR_FS_CP_EEXIST","SystemError",17,"cp",true,"Target already exists: cp returned EEXIST (D/b.txt already exists) D/b.txt",["code","info","errno","syscall","path"]],"excl1":"hello","excl3":"ERR_FS_CP_EEXIST","excl2":"no throw","tree":["x.txt"],"dirNoRecursive":"ERR_FS_EISDIR","same":{"code":"ERR_FS_CP_EINVAL","name":"Error","message":"src and dest cannot be the same D/a.txt"},"intoSelf":{"code":"ERR_FS_CP_EINVAL","name":"Error","message":"Cannot copy D/dir/ to a subdirectory of self D/dir/sub/x"},"dirOverFile":{"code":"ERR_FS_CP_DIR_TO_NON_DIR","name":"Error","message":"Cannot overwrite non-directory D/a.txt with directory D/dir"},"fileOverDir":{"code":"ERR_FS_CP_NON_DIR_TO_DIR","name":"Error","message":"Cannot overwrite directory D/dir with non-directory D/a.txt"}}"#####
    );
}

#[test]
fn j_child_stdio_fd() {
    // expected: node22
    assert_eq!(
        p2c_run("j_child_stdio_fd", r#####"const cp = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'stdiofd-'));
const file = path.join(dir, 'out.txt');
const fd = fs.openSync(file, 'w');
const out = {};
const child = cp.spawn('sh', ['-c', 'echo to-fd; echo err-fd >&2'], { stdio: ['ignore', fd, fd] });
out.pipes = [child.stdin, child.stdout, child.stderr].map((s) => s === null);
await new Promise((resolve) => child.on('close', resolve));
fs.closeSync(fd);
out.file = fs.readFileSync(file, 'utf8').split('\n').sort();
// inherit: child stdout goes to parent, no pipe object
const inh = cp.spawn('sh', ['-c', 'exit 3'], { stdio: 'inherit' });
out.inheritPipes = [inh.stdin, inh.stdout, inh.stderr].map((s) => s === null);
out.inheritCode = await new Promise((resolve) => inh.on('close', resolve));
fs.rmSync(dir, { recursive: true, force: true });
return out;
"#####),
        r#####"{"pipes":[true,true,true],"file":["","err-fd","to-fd"],"inheritPipes":[true,true,true],"inheritCode":3}"#####
    );
}

#[test]
fn j_child_exit_before_eof() {
    // expected: node22
    assert_eq!(
        p2c_run("j_child_exit_before_eof", r#####"const cp = require('node:child_process');
const t0 = Date.now();
const child = cp.spawn('sh', ['-c', '(sleep 0.8 >/dev/null 2>&1 &) ; echo hi; (sleep 0.8; echo late) &']);
const events = [];
let data = '';
child.stdout.on('data', (d) => { data += d; });
child.on('exit', () => events.push(['exit', Date.now() - t0 < 600]));
await new Promise((resolve) => child.on('close', () => { events.push(['close', Date.now() - t0 >= 700]); resolve(); }));
return { events, data: data.trim().split('\n') };
"#####),
        r#####"{"events":[["exit",true],["close",true]],"data":["hi","late"]}"#####
    );
}
