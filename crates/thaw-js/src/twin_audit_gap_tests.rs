//! Regression tests for gaps the Node-twin audit found in thaw-js: Blob/File conversion order, brand checks and
//! private state, WebAssembly argument typing, EventTarget / URL details. Each program prints lines joined with `|`;
//! the expected text is the output of real Node 22 for the same program.

use super::*;

/// Run `src` and return the console.log lines joined with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => match it.run_event_loop() {
            Ok(_) => it.out.join("|"),
            Err(_) => "event loop failed".into(),
        },
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

#[test]
fn blob_file_webidl_conversion_order_and_number_handling() {
    assert_eq!(
        one(r##"const out = [];
const P = (n, f) => { try { out.push(n + '=' + JSON.stringify(f())); } catch (e) { out.push(n + '!' + (e && e.name) + (e && e.code ? ':' + e.code : '')); } };
const ts = v => new File([], 'x', { lastModified: v }).lastModified;
P('name', () => new File([], '\uD800😀\uDC00').name === '�😀�');
for (const [k, v] of [['1.9', 1.9], ['-1.9', -1.9], ['NaN', NaN], ['Inf', Infinity], ['-Inf', -Infinity], ['-0', -0], ['null', null], ['2^64', 2 ** 64], ['2^63', 2 ** 63], ['1n', 1n], ['sym', Symbol()], ['str', '5'], ['obj', {}]]) P('ts' + k, () => ts(v));
P('nameSym', () => new File([], Symbol('n')));
let ir = 0; const it = { get [Symbol.iterator]() { ir++; return function* () { yield 'x'; }; } };
P('noargs', () => new File()); P('onearg', () => new File(it)); P('iterReads', () => ir);
P('blobNull', () => new Blob(null)); P('blobArrayLike', () => new Blob({ length: 0 })); P('fileUndef', () => new File(undefined, 'x'));
P('empty', () => [new Blob().size, new Blob(undefined).size]);
P('blobNum', () => new Blob(5)); P('blobStr', () => new Blob('abc').size);
let closed = 0; const S = {};
P('closing', () => { try { new Blob({ *[Symbol.iterator]() { try { yield { toString() { throw S; } }; yield 'u'; } finally { closed++; } } }); } catch (e) { return [e === S, closed]; } });
const order = [];
P('order', () => { new File([{ toString() { order.push('parts'); return 'x'; } }], { toString() { order.push('name'); return 'n'; } }, {
  get endings() { order.push('endings-get'); return { toString() { order.push('endings-convert'); return 'transparent'; } }; },
  get type() { order.push('type-get'); return { toString() { order.push('type-convert'); return 'TEXT/PLAIN'; } }; },
  get lastModified() { order.push('last-get'); return { valueOf() { order.push('last-convert'); return 1.9; } }; } }); return order.join(); });
P('later', () => { let laterReads = 0; try { new File([], 'x', { endings: 'transparent', get type() { throw S; }, get lastModified() { laterReads++; return 0; } }); } catch (e) { return [e === S, laterReads]; } });
P('typeLower', () => { const f = new File([], 'x', { type: 'TEXT/PLAIN', lastModified: { valueOf() { return 1.9; } } }); return [f.type, f.lastModified]; });
P('badEndings', () => new Blob([], { endings: 'bogus' }));
P('changed', async () => 0);
const source = new Uint8Array([1, 2]);
const changed = new File([source], { toString() { source[0] = 9; return 'changed'; } }, { get endings() { source[1] = 8; return 'transparent'; }, type: 'application/test' });
changed.bytes().then(b => { out.push('changedBytes=' + Array.from(b)); 
  console.log(out.join('\n')); });"##),
        r##"name=true|ts1.9=1.9|ts-1.9=-1.9|tsNaN=0|tsInf=null|ts-Inf=null|ts-0=0|tsnull=0|ts2^64=18446744073709552000|ts2^63=9223372036854776000|ts1n!TypeError|tssym!TypeError|tsstr=5|tsobj=0|nameSym!TypeError|noargs!TypeError:ERR_MISSING_ARGS|onearg!TypeError:ERR_MISSING_ARGS|iterReads=0|blobNull!TypeError:ERR_INVALID_ARG_TYPE|blobArrayLike!TypeError:ERR_INVALID_ARG_TYPE|fileUndef={}|empty=[0,0]|blobNum!TypeError:ERR_INVALID_ARG_TYPE|blobStr!TypeError:ERR_INVALID_ARG_TYPE|closing=[true,1]|order="type-get,endings-get,endings-convert,parts,type-convert,last-get,last-convert,name"|later=[true,0]|typeLower=["text/plain",1.9]|badEndings!TypeError:ERR_INVALID_ARG_VALUE|changed={}|changedBytes=1,8"##
    );
}

#[test]
fn blob_file_brand_checks_private_state_and_slice_conversions() {
    assert_eq!(
        one(r##"const R = [];
const P = (n, f) => { try { const v = f(); if (v && typeof v.then === 'function') return v.then(x => R.push(n + '=' + JSON.stringify(x)), e => R.push(n + '!' + e.name + (e.code ? ':' + e.code : ''))); R.push(n + '=' + JSON.stringify(v)); } catch (e) { R.push(n + '!' + (e && e.name) + (e && e.code ? ':' + e.code : '')); } };
(async () => {
const buffer = new ArrayBuffer(4), input = new Uint8Array(buffer); input.set([10, 11, 12, 13]);
const view = new DataView(buffer, 1, 2);
const blob = new Blob([view], { type: 'TEXT/PLAIN' }); input[1] = 90;
await P('bytes', async () => Array.from(await blob.bytes()));
await P('bytesCopy', async () => { const c = await blob.bytes(); c[0] = 70; return Array.from(await blob.bytes()); });
const stream = blob.stream(), reader = stream.getReader();
await P('stream', async () => { const r = await reader.read(); r.value[0] = 72; return [stream.locked, (await reader.read()).done, Array.from(await blob.bytes()), r.value.constructor.name]; });
const nested = new Blob([blob]).slice(0, 2);
Object.defineProperty(blob, 'size', { value: 99, configurable: true });
Object.defineProperty(blob, '__thawBytes', { value: new Uint8Array([99]), configurable: true });
const sliced = blob.slice(0, 2);
const file = new File([blob], 'original', { type: 'IMAGE/PNG', lastModified: 42 });
(function sloppy(v, f) { v.type = 'changed'; v.size = 0; f.name = 'changed'; f.type = 'changed'; f.lastModified = -1; })(blob, file);
P('afterSloppy', () => [blob.type, blob.size, file.name, file.type, file.lastModified]);
for (const [n, o, k] of [['blob.size', blob, 'size'], ['blob.type', blob, 'type'], ['file.name', file, 'name'], ['file.lastModified', file, 'lastModified']]) P('strict ' + n, () => { 'use strict'; o[k] = 1; return 'noerr'; });
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); return x && [typeof x.get, typeof x.set, x.enumerable, x.configurable]; };
P('desc', () => ['size', 'type', 'slice', 'text', 'arrayBuffer', 'bytes', 'stream'].map(k => d(Blob.prototype, k)));
P('fdesc', () => ['name', 'lastModified', 'webkitRelativePath'].map(k => d(File.prototype, k)));
P('getterOnPlain', () => d(Blob.prototype, 'size').get.call({}));
P('fileNameOnBlob', () => d(File.prototype, 'name').get.call(blob));
P('sliceOnPlain', () => Blob.prototype.slice.call({}));
await P('bytesOnPlain', () => Blob.prototype.bytes.call({}));
await P('textOnPlain', () => Blob.prototype.text.call({}));
await P('streamOnPlain', () => Blob.prototype.stream.call({}));
P('forged', () => Object.create(Blob.prototype).size);
P('callNew', () => Blob()); P('callNewFile', () => File([], 'x'));
P('child', () => new (class C extends Blob {})(['x']) instanceof Blob);
P('tag', () => [Object.prototype.toString.call(blob), Object.prototype.toString.call(file), Blob.length, File.length, Blob.name, File.name, Object.getPrototypeOf(File) === Blob, Object.keys(blob)]);
P('endings', () => []);
await P('endingsText', async () => [await new Blob(['a\r\nb\rc\nd'], { endings: 'native' }).text(), await new Blob(['a\r\nb\rc\nd']).text()]);
await P('parts', async () => { const b = new Blob([new Blob(['q']), new Uint16Array([0x4142]), new DataView(new ArrayBuffer(1)), new ArrayBuffer(2), 5, {}, null, undefined]); return await b.text(); });
await P('sliceform', async () => { const b = new Blob(['abcdef']); return [await b.slice(1.5, 4.5).text(), await b.slice(-2.5).text(), await b.slice(NaN).text(), await b.slice(Infinity).text(), await b.slice(2, null).text(), await b.slice(-Infinity, Infinity).text(), await b.slice(1.9, 3.9).text()]; });
P('sliceSym', () => new Blob(['a']).slice(Symbol()));
P('slice1n', () => new Blob(['a']).slice(1n));
P('sliceorder', () => { const o = []; const b = new Blob(['abcdef']).slice({ valueOf() { o.push('s'); return 1; } }, { valueOf() { o.push('e'); return 2; } }, { toString() { o.push('t'); return 'X/Y'; } }); return [o.join(), b.type]; });
P('slicethrow', () => { let late = 0; try { new Blob(['abc']).slice({ valueOf() { throw 7; } }, { valueOf() { late++; return 1; } }, { toString() { late++; return 'x'; } }); } catch (e) { return [e, late]; } });
P('filesliceType', () => { const s = new File(['abcdef'], 'f').slice(1, 3, 'TEXT/PLAIN'); return [s instanceof File, s instanceof Blob, s.type, s.size]; });
P('typeNonAscii', () => [new Blob([], { type: 'é' }).type, new Blob([], { type: 'A\x7f' }).type, new Blob([], { type: ' x ' }).type, new Blob([], { type: 'Text/PLAIN;Charset=UTF-8' }).type]);
P('typeSym', () => new Blob([], { type: Symbol() }));
P('optsBad', () => [new Blob([], null).size]);
P('optsStr', () => new Blob([], 'x'));
P('optsNum', () => new Blob([], 5));
P('fileOptsNull', () => new File([], 'x', null).name);
P('fname', () => [new File([], undefined).name, new File([], null).name, new File([], 5).name, new File([], 'a/b').name]);
P('fileEndings', () => 0);
P('thenable', () => 0);
await Promise.resolve();
console.log(R.join('\n'));
})();"##),
        r##"bytes=[11,12]|bytesCopy=[11,12]|stream=[true,true,[11,12],"Uint8Array"]|afterSloppy=["text/plain",99,"original","image/png",42]|strict blob.size!TypeError|strict blob.type!TypeError|strict file.name!TypeError|strict file.lastModified!TypeError|desc=[["function","undefined",true,true],["function","undefined",true,true],["undefined","undefined",true,true],["undefined","undefined",true,true],["undefined","undefined",true,true],["undefined","undefined",true,true],["undefined","undefined",true,true]]|fdesc=[["function","undefined",true,true],["function","undefined",true,true],null]|getterOnPlain!TypeError|fileNameOnBlob!TypeError|sliceOnPlain!TypeError:ERR_INVALID_THIS|bytesOnPlain!TypeError:ERR_INVALID_THIS|textOnPlain!TypeError:ERR_INVALID_THIS|streamOnPlain!TypeError:ERR_INVALID_THIS|forged!TypeError:ERR_INVALID_THIS|callNew!TypeError|callNewFile!TypeError|child=true|tag=["[object Blob]","[object File]",0,2,"Blob","File",true,[]]|endings=[]|endingsText=["a\nb\rc\nd","a\r\nb\rc\nd"]|parts="qBA\u0000\u0000\u00005[object Object]nullundefined"|sliceform=["bcd","ef","abcdef","abcdef","","","bc"]|sliceSym!TypeError|slice1n!TypeError|sliceorder=["s,e,t","x/y"]|slicethrow=[7,0]|filesliceType=[false,true,"text/plain",2]|typeNonAscii=["",""," x ","text/plain;charset=utf-8"]|typeSym!TypeError|optsBad=[0]|optsStr!TypeError:ERR_INVALID_ARG_TYPE|optsNum!TypeError:ERR_INVALID_ARG_TYPE|fileOptsNull="x"|fname=["undefined","null","5","a/b"]|fileEndings=0|thenable=0"##
    );
}

#[test]
fn blob_file_argument_errors_match_node() {
    assert_eq!(
        one(
            r##"const m = f => { try { const r = f(); if (r && r.then) return r.then(() => 'resolved', e => 'REJECT ' + e.message); return 'ok'; } catch (e) { return 'THROW ' + e.name + ' | ' + e.message; } };
(async () => {
const L = [];
for (const [n, f] of Object.entries({
 null: () => new Blob(null), num: () => new Blob(5), str: () => new Blob('abc'), obj: () => new Blob({}), optsStr: () => new Blob([], 'x'), optsNum: () => new Blob([], 5),
 badEnd: () => new Blob([], { endings: 'x' }), fileNone: () => new File(), fileOne: () => new File([]), fileNullBits: () => new File(null, 'n'), fileOptsStr: () => new File([], 'n', 'x'),
 textThis: () => Blob.prototype.text.call({}), abThis: () => Blob.prototype.arrayBuffer.call({}), bytesThis: () => Blob.prototype.bytes.call({}), streamThis: () => Blob.prototype.stream.call({}),
 sliceThis: () => Blob.prototype.slice.call({}), sizeThis: () => Object.getOwnPropertyDescriptor(Blob.prototype, 'size').get.call({}), fnameThis: () => Object.getOwnPropertyDescriptor(File.prototype, 'name').get.call(new Blob()),
 callBlob: () => Blob([]), callFile: () => File([], 'x'), sliceSym: () => new Blob().slice(Symbol()), ts1n: () => new File([], 'n', { lastModified: 1n }), nameSym: () => new File([], Symbol()), partSym: () => new Blob([Symbol()]),
})) L.push(n + ': ' + await m(f));
console.log(L.join('\n'));
})();"##
        ),
        r##"null: THROW TypeError | The "sources" argument must be a sequence. Received null|num: THROW TypeError | The "sources" argument must be a sequence. Received type number (5)|str: THROW TypeError | The "sources" argument must be a sequence. Received type string ('abc')|obj: THROW TypeError | The "sources" argument must be a sequence. Received an instance of Object|optsStr: THROW TypeError | The "options" argument must be a dictionary. Received type string ('x')|optsNum: THROW TypeError | The "options" argument must be a dictionary. Received type number (5)|badEnd: THROW TypeError | The property 'options.endings' is invalid. Received 'x'|fileNone: THROW TypeError | The "fileBits" and "fileName" arguments must be specified|fileOne: THROW TypeError | The "fileBits" and "fileName" arguments must be specified|fileNullBits: THROW TypeError | The "sources" argument must be a sequence. Received null|fileOptsStr: THROW TypeError | The "options" argument must be a dictionary. Received type string ('x')|textThis: REJECT Value of "this" must be of type Blob|abThis: REJECT Value of "this" must be of type Blob|bytesThis: REJECT Value of "this" must be of type Blob|streamThis: THROW TypeError | Value of "this" must be of type Blob|sliceThis: THROW TypeError | Value of "this" must be of type Blob|sizeThis: THROW TypeError | Value of "this" must be of type Blob|fnameThis: THROW TypeError | Cannot read private member #name from an object whose class did not declare it|callBlob: THROW TypeError | Class constructor Blob cannot be invoked without 'new'|callFile: THROW TypeError | Class constructor File cannot be invoked without 'new'|sliceSym: THROW TypeError | Cannot convert a Symbol value to a number|ts1n: THROW TypeError | Cannot convert a BigInt value to a number|nameSym: THROW TypeError | Cannot convert a Symbol value to a string|partSym: THROW TypeError | Cannot convert a Symbol value to a string"##
    );
}

#[test]
fn blob_text_ignores_patched_text_decoder() {
    assert_eq!(
        one(r##"async function blobTextDecoderIsolation() {
              const blob = new Blob([new Uint8Array([65, 66])]);
              const file = new File([new Uint8Array([67, 68])], 'cd.txt');
              const descriptor = Object.getOwnPropertyDescriptor(TextDecoder.prototype, 'decode');
              const originalDecode = descriptor.value;
              const inputs = [];
              let decoded = false;
              try {
                TextDecoder.prototype.decode = function(input, options) {
                  inputs.push(input);
                  return originalDecode.call(this, input, options);
                };
                decoded = await blob.text() === 'AB' && await file.text() === 'CD';
              } finally {
                Object.defineProperty(TextDecoder.prototype, 'decode', descriptor);
              }
              const retainedInputs = inputs.length === 2
                && inputs.every(input => input instanceof Uint8Array);
              for (const input of inputs) {
                for (let index = 0; index < input.length; index++) input[index] = 0;
              }
              return [decoded, retainedInputs,
                JSON.stringify(Array.from(await blob.bytes())) === '[65,66]'
                  && JSON.stringify(Array.from(await file.bytes())) === '[67,68]',
                JSON.stringify(Array.from(await blob.slice(0, 2).bytes())) === '[65,66]'
                  && JSON.stringify(Array.from(await file.slice(0, 2).bytes())) === '[67,68]'];
            }
blobTextDecoderIsolation().then(v=>console.log(JSON.stringify(v)),e=>console.log('ERR',e&&e.name,e&&e.message));"##),
        r##"[true,false,true,true]"##
    );
}

#[test]
fn structured_clone_of_blob_and_file_yields_a_blob_with_the_same_bytes() {
    assert_eq!(
        one(r##"(async()=>{
const blob = new Blob([new Uint8Array([1,2])], {type:'text/plain'}), file = new File([new Uint8Array([3,4])], 'sample.txt', {type:'text/csv', lastModified: 42});
for (const [n, v] of [['blob', blob], ['file', file]]) {
  try { const c = structuredClone({ v }).v; console.log(n, c instanceof Blob, c instanceof File, c.size, c.type, c.name, c.lastModified, Array.from(new Uint8Array(await c.arrayBuffer())).join(), c === v); } catch (e) { console.log(n, 'ERR', e.name, e.message); }
}
const shadow = new Blob([new Uint8Array([9])]); Object.defineProperty(shadow, 'size', { value: 77 });
const c = structuredClone(shadow); console.log(c.size, Object.keys(c));
const g = { get blob() { return 1; } };
})();"##),
        r##"blob true false 2 text/plain undefined undefined 1,2 false|file true false 2 text/csv undefined undefined 3,4 false|1 []"##
    );
}

#[test]
fn event_target_duplicate_signal_recursion_error_and_sticky_stop_match_node() {
    assert_eq!(
        one(r##"const out = [];
// the signal option of a duplicate registration still arms the abort: it removes the equal listener already registered
{
  const calls = [], cb = () => calls.push('cb');
  for (const key of ['capture', 'once', 'passive', 'signal']) {
    const t = new EventTarget(), controller = new AbortController(), options = {};
    Object.defineProperty(options, key, { get() { t.addEventListener('r', cb); return key === 'signal' ? controller.signal : key === 'once'; } });
    t.addEventListener('r', cb, options);
    controller.abort();
    t.dispatchEvent(new Event('r')); out.push(key + ':' + calls.length); calls.length = 0;
  }
  const t = new EventTarget(), c = new AbortController();
  t.addEventListener('x', cb); t.addEventListener('x', cb, { signal: c.signal }); c.abort();
  t.dispatchEvent(new Event('x')); out.push('dup-abort:' + calls.length); calls.length = 0;
}
// re-dispatching an event during its own dispatch is a plain Error with a code
{
  const e = new Event('y'), t = new EventTarget(), o = new EventTarget();
  const seen = [];
  const rec = (x) => seen.push([x.name, x.code, x.message, x instanceof DOMException].join('/'));
  t.addEventListener('y', () => { try { t.dispatchEvent(e); } catch (x) { rec(x); } try { o.dispatchEvent(e); } catch (x) { rec(x); } });
  t.dispatchEvent(e);
  out.push(seen.join(';'));
}
// stopImmediatePropagation sticks to the event object: dispatching it again runs no listener
{
  const t = new EventTarget(), e = new Event('x'); let n = 0;
  t.addEventListener('x', ev => { n++; ev.stopImmediatePropagation(); }); t.addEventListener('x', () => n += 10);
  t.dispatchEvent(e); out.push(n); t.dispatchEvent(e); out.push(n);
}
console.log(out.join('|'));"##),
        r##"capture:1|once:1|passive:1|signal:0|dup-abort:0|Error/ERR_EVENT_RECURSION/The event "y" is already being dispatched/false;Error/ERR_EVENT_RECURSION/The event "y" is already being dispatched/false|1|1"##
    );
}

#[test]
fn buffer_write_validates_offset_length_and_never_splits_a_character() {
    assert_eq!(
        one(r##"const R = [];
const t = (n, f) => { try { R.push(n + '=' + JSON.stringify(f())); } catch (e) { R.push(n + '!' + e.name + ':' + e.code + ':' + e.message); } };
const b = () => Buffer.alloc(4);
t('w1', () => { const x = b(); return [x.write('abcd', 0, 4), x.toString('hex')]; });
t('w2', () => { const x = Buffer.alloc(2); return x.write('abcd', 0, 4); });
t('w3', () => { const x = Buffer.alloc(2); return x.write('ab', 0, 2.5); });
t('w4', () => { const x = Buffer.alloc(2); return x.write('ab', 0, Infinity); });
t('w5', () => { const x = Buffer.alloc(2); return x.write('ab', 0, -1); });
t('w6', () => { const x = Buffer.alloc(2); return x.write('ab', 3); });
t('w7', () => { const x = Buffer.alloc(2); return x.write('ab', 1.5); });
t('w8', () => { const x = Buffer.alloc(2); return [x.write('あ'), x.toString('hex')]; });
t('w9', () => { const x = Buffer.alloc(4); return [x.write('あい'), x.toString('hex')]; });
t('w10', () => { const x = Buffer.alloc(4); return [x.write('aあ', 1, 3), x.toString('hex')]; });
t('w11', () => { const x = Buffer.alloc(4); return [x.write('abcdef', 'latin1'), x.toString('hex')]; });
t('w12', () => { const x = Buffer.alloc(4); return [x.write('abcdef', 2, 'latin1'), x.toString('hex')]; });
t('w13', () => { const x = Buffer.alloc(4); return [x.write('abcdef', 1, 2), x.toString('hex')]; });
t('w14', () => { const x = Buffer.alloc(4); return [x.write('abcdef', 1, 10), x.toString('hex')]; });
t('w15', () => { const x = Buffer.alloc(4); return x.write('ab', '2'); });
t('w16', () => { const x = Buffer.alloc(4); return x.write('ab', 0, '2'); });
t('w17', () => { const x = Buffer.alloc(4); return x.write('ab', 0, undefined, 'hex'); });
t('w18', () => { const x = Buffer.alloc(2); return [x.write('6162', 'hex'), x.write('616263', 'hex'), x.toString('hex')]; });
t('w19', () => { const x = Buffer.alloc(3); return [x.write('😀'), x.toString('hex')]; });
t('w20', () => { const x = Buffer.alloc(5); return [x.write('a😀', 0, 4), x.toString('hex')]; });
t('w21', () => { const x = Buffer.alloc(4); return x.write(5); });
t('w22', () => { const x = Buffer.alloc(4); return [x.write('ab', 0, 0), x.write('', 4)]; });
t('w23', () => { const x = Buffer.alloc(4); return x.write('ab', 4, 1); });
t('w24', () => { const x = Buffer.alloc(4); return [x.write('ab', 2, 5)]; });
t('w25', () => { const x = Buffer.alloc(4); return [x.write('abcd', 0, 4, 'ucs2'), x.toString('hex')]; });
t('w26', () => { const x = Buffer.alloc(3); return [x.write('ab', 0, 'ucs2'), x.toString('hex')]; });
t('w27', () => { const x = Buffer.alloc(4); return x.write('ab', 0, null); });
t('w28', () => { const x = Buffer.alloc(4); return x.write('ab', null); });
t('w29', () => { const x = Buffer.alloc(4); return x.write('ab', NaN); });
t('w30', () => { const x = Buffer.alloc(4); return x.write('ab', 0, NaN); });
t('f1', () => Buffer.alloc(5, new Uint8Array([1, 2])).toString('hex'));
t('f2', () => Buffer.from(new Uint16Array([0x1234])).toString('hex'));
t('f3', () => Buffer.alloc(1).readUInt16LE(0));
t('f4', () => Buffer.alloc(1).writeUInt16LE(7, 0));
t('lone', () => [Array.from(Buffer.from('\uD800a')), Buffer.byteLength('\uD83D'), Array.from(Buffer.from('😀'))]);
console.log(R.join('\n'));"##),
        r##"w1=[4,"61626364"]|w2!RangeError:ERR_OUT_OF_RANGE:The value of "length" is out of range. It must be >= 0 && <= 2. Received 4|w3!RangeError:ERR_OUT_OF_RANGE:The value of "length" is out of range. It must be an integer. Received 2.5|w4!RangeError:ERR_OUT_OF_RANGE:The value of "length" is out of range. It must be an integer. Received Infinity|w5!RangeError:ERR_OUT_OF_RANGE:The value of "length" is out of range. It must be >= 0 && <= 2. Received -1|w6!RangeError:ERR_OUT_OF_RANGE:The value of "offset" is out of range. It must be >= 0 && <= 2. Received 3|w7!RangeError:ERR_OUT_OF_RANGE:The value of "offset" is out of range. It must be an integer. Received 1.5|w8=[0,"0000"]|w9=[3,"e3818200"]|w10=[1,"00610000"]|w11=[4,"61626364"]|w12=[2,"00006162"]|w13=[2,"00616200"]|w14!RangeError:ERR_OUT_OF_RANGE:The value of "length" is out of range. It must be >= 0 && <= 4. Received 10|w15!TypeError:ERR_UNKNOWN_ENCODING:Unknown encoding: 2|w16!TypeError:ERR_UNKNOWN_ENCODING:Unknown encoding: 2|w17=1|w18=[2,2,"6162"]|w19=[0,"000000"]|w20=[1,"6100000000"]|w21!TypeError:ERR_INVALID_ARG_TYPE:argument must be a string|w22=[0,0]|w23=0|w24!RangeError:ERR_OUT_OF_RANGE:The value of "length" is out of range. It must be >= 0 && <= 4. Received 5|w25=[4,"61006200"]|w26=[2,"610000"]|w27!TypeError:ERR_INVALID_ARG_TYPE:The "length" argument must be of type number. Received null|w28!TypeError:ERR_INVALID_ARG_TYPE:The "offset" argument must be of type number. Received null|w29!RangeError:ERR_OUT_OF_RANGE:The value of "offset" is out of range. It must be an integer. Received NaN|w30!RangeError:ERR_OUT_OF_RANGE:The value of "length" is out of range. It must be an integer. Received NaN|f1="0102010201"|f2="34"|f3!RangeError:ERR_BUFFER_OUT_OF_BOUNDS:Attempt to access memory outside buffer bounds|f4!RangeError:ERR_BUFFER_OUT_OF_BOUNDS:Attempt to access memory outside buffer bounds|lone=[[239,191,189,97],3,[240,159,152,128]]"##
    );
}
