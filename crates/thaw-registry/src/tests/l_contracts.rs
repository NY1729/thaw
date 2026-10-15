// Behaviour contracts for wave L (node:vm contexts, REPL on top of them). Each case runs the same JavaScript body through
// the bundled Thaw built-in; the expected JSON was recorded by running the identical body under real Node.js 22.

#[test]
fn l_vm_basic() {
    // expected: node22
    assert_eq!(
        p2c_run("l_vm_basic", r#####"const vm = require('node:vm');
const out = {};
const sb = { a: 1, f() { return this === sb; } };
vm.createContext(sb);
out.isContext = [vm.isContext(sb), vm.isContext({})];
out.r1 = vm.runInContext('var x = 10; let y = 20; const z = 30; class K {}; function g(){ return 7 }; x + y + z', sb);
out.persist = vm.runInContext('x + y + z + typeof K + g()', sb);
out.sb = [sb.x, sb.y, sb.z, typeof sb.K, typeof sb.g];
out.keys = Object.keys(sb).sort();
sb.w = 5; out.hostSet = vm.runInContext('w * 2', sb);
vm.runInContext('w = 99; delete a; newGlobal = 3; globalThis.viaGlobal = 4', sb);
out.after = [sb.w, 'a' in sb, sb.newGlobal, sb.viaGlobal];
out.thisIsSandbox = [vm.runInContext('this', sb) === sb, vm.runInContext('globalThis', sb) === sb];
out.typeofUnresolved = vm.runInContext('typeof nope', sb);
try { vm.runInContext('nope', sb); } catch (e) { out.ref = [e.name, e.message, e instanceof ReferenceError]; }
try { vm.runInContext('throw new TypeError("boom")', sb); } catch (e) { out.type = [e.name, e.message, e instanceof TypeError, e instanceof Error]; }
out.f = vm.runInContext('f()', sb);
out.arr = [vm.runInContext('[1,2,3]', sb) instanceof Array, Array.isArray(vm.runInContext('[1,2,3]', sb))];
out.globals = ['console', 'require', 'process', 'setTimeout', 'Buffer'].map((n) => vm.runInContext('typeof ' + n, sb));
out.newctx = vm.runInNewContext('a + b', { a: 1, b: 2 });
const s = new vm.Script('count = (typeof count === "number" ? count : 0) + 1');
const c1 = vm.createContext({}), c2 = vm.createContext({});
s.runInContext(c1); s.runInContext(c1); s.runInContext(c2);
out.script = [c1.count, c2.count];
out.desc = Object.getOwnPropertyDescriptor(vm.createContext({ q: 1 }), 'q');
const d = vm.createContext({});
vm.runInContext('var v1 = 1; function fn(){}; Object.defineProperty(globalThis, "dp", {value: 5, enumerable: true, configurable: true})', d);
out.defprop = [Object.keys(d).sort(), d.dp];
out.inst = [vm.runInNewContext('[]') instanceof Array, vm.runInNewContext('({})') instanceof Object];
out.fnThis = vm.runInContext('(function(){ return this === globalThis })()', c1);
out.strict = vm.runInNewContext('"use strict"; var u = 1; typeof u');
out.shared = vm.runInNewContext('[typeof Intl, typeof WebAssembly, typeof SharedArrayBuffer, typeof Atomics, typeof Promise, typeof JSON, typeof Math].join()');
return out;
"#####),
        r#####"{"isContext":[true,false],"r1":60,"persist":"60function7","sb":[10,null,null,"undefined","function"],"keys":["a","f","g","x"],"hostSet":10,"after":[99,false,3,4],"thisIsSandbox":[false,false],"typeofUnresolved":"undefined","ref":["ReferenceError","nope is not defined",false],"type":["TypeError","boom",false,false],"f":false,"arr":[false,true],"globals":["object","undefined","undefined","undefined","undefined"],"newctx":3,"script":[2,1],"desc":{"value":1,"writable":true,"enumerable":true,"configurable":true},"defprop":[["dp","fn","v1"],5],"inst":[false,false],"fnThis":true,"strict":"number","shared":"object,object,function,object,function,object,object"}"#####
    );
}

#[test]
fn l_vm_objects() {
    // expected: node22
    assert_eq!(
        p2c_run("l_vm_objects", r#####"const vm = require('node:vm');
const out = {};
const proto = { inherited: 'p' };
const sb = Object.create(proto);
sb.own = 1;
Object.defineProperty(sb, 'getter', { get() { return 'g' + (sb.own); }, enumerable: true, configurable: true });
Object.defineProperty(sb, 'hidden', { value: 'h', enumerable: false, configurable: true });
const sym = Symbol('s'); sb[sym] = 'symval';
vm.createContext(sb);
out.read = vm.runInContext('[own, getter, hidden, typeof inherited]', sb);
vm.runInContext('own = 2', sb);
out.afterWrite = [sb.own, sb.getter];
out.freezeNoCtx = (() => { const f = vm.createContext(Object.freeze({ fz: 1 })); try { vm.runInContext('fz = 2; fz', f); return vm.runInContext('fz', f); } catch (e) { return e.name; } })();
const fnSandbox = vm.createContext({ add(a, b) { return a + b; }, obj: { n: 1 } });
out.callHost = vm.runInContext('add(2, 3) + obj.n', fnSandbox);
vm.runInContext('obj.n = 10; obj.extra = "e"', fnSandbox);
out.mutated = fnSandbox.obj;
out.shadow = (() => { const c = vm.createContext({ Math: { max: () => 'shadowed' } }); return vm.runInContext('Math.max(1, 2)', c); })();
out.shadowRestored = (() => { const c = vm.createContext({ Array: 1 }); const first = vm.runInContext('typeof Array', c); delete c.Array; return [first, vm.runInContext('typeof Array', c)]; })();
out.sameCtxTwice = (() => { const o = {}; return vm.createContext(o) === o && vm.createContext(o) === o; })();
out.badCtx = (() => { try { vm.createContext(1); } catch (e) { return e.name; } })();
out.notCtx = (() => { try { vm.runInContext('1', {}); } catch (e) { return [e.name, e.code, e.message]; } })();
out.badArgs = [() => vm.isContext(1), () => vm.isContext(null), () => vm.runInNewContext('1', 5), () => vm.createContext(null), () => vm.runInContext('1', vm.createContext({}), { timeout: 'x' })].map((f) => { try { f(); } catch (e) { return [e.name, e.code, e.message]; } });
out.hasOwnAfter = (() => { const c = vm.createContext({}); vm.runInContext('var declared = 1; implicit = 2; globalThis.explicit = 3; let lexical = 4', c); return [Object.keys(c).sort(), 'lexical' in c, c.declared, c.implicit, c.explicit]; })();
out.hostMutatesDeclared = (() => { const c = vm.createContext({}); vm.runInContext('var counter = 1; function bump() { return ++counter }', c); c.counter = 10; const r = [vm.runInContext('counter', c)]; c.bump(); r.push(c.counter); return r; })();
out.separation = (() => { const a = vm.createContext({}), b = vm.createContext({}); vm.runInContext('var shared = "a"', a); return [vm.runInContext('typeof shared', b), a.shared, b.shared]; })();
out.constAcrossRuns = (() => { const c = vm.createContext({}); vm.runInContext('const k = 5', c); try { vm.runInContext('k = 6', c); } catch (e) { return [e.name, vm.runInContext('k', c)]; } })();
out.getterOnContextRuns = (() => { const c = vm.createContext({ get live() { return 'now'; } }); return vm.runInContext('live', c); })();
out.proxyGlobal = (() => { const c = vm.createContext({ x: 1 }); return vm.runInContext('Object.keys(globalThis).includes("x") + "," + ("x" in globalThis) + "," + globalThis.hasOwnProperty("x")', c); })();
return out;
"#####),
        r#####"{"read":[1,"g1","h","string"],"afterWrite":[2,"g2"],"freezeNoCtx":1,"callHost":6,"mutated":{"n":10,"extra":"e"},"shadow":"shadowed","shadowRestored":["number","function"],"sameCtxTwice":true,"badCtx":"TypeError","notCtx":["TypeError","ERR_INVALID_ARG_TYPE","The \"contextifiedObject\" argument must be an vm.Context. Received an instance of Object"],"badArgs":[["TypeError","ERR_INVALID_ARG_TYPE","The \"object\" argument must be of type object. Received type number (1)"],["TypeError","ERR_INVALID_ARG_TYPE","The \"object\" argument must be of type object. Received null"],["TypeError","ERR_INVALID_ARG_TYPE","The \"object\" argument must be of type object. Received type number (5)"],["TypeError","ERR_INVALID_ARG_TYPE","The \"object\" argument must be of type object. Received null"],["TypeError","ERR_INVALID_ARG_TYPE","The \"options.timeout\" property must be of type number. Received type string ('x')"]],"hasOwnAfter":[["declared","explicit","implicit"],false,1,2,3],"hostMutatesDeclared":[10,11],"separation":["undefined","a",null],"constAcrossRuns":["TypeError",5],"getterOnContextRuns":"now","proxyGlobal":"true,true,true"}"#####
    );
}

#[test]
fn l_vm_errors() {
    // expected: node22
    assert_eq!(
        p2c_run("l_vm_errors", r#####"const vm = require('node:vm');
const out = {};
const sb = vm.createContext({});
try { vm.runInContext('null.x', sb, { filename: 'ctx-file.js' }); } catch (e) { out.typeError = [e.name, e instanceof TypeError, e.constructor.name, String(e.stack).includes('ctx-file.js')]; }
try { vm.runInContext('let a = ;', sb, { filename: 'bad-syntax.js' }); } catch (e) { out.syntax = [e.name, e instanceof SyntaxError, String(e.stack).includes('bad-syntax.js')]; }
try { vm.runInContext('throw { custom: 1 }', sb); } catch (e) { out.thrownObject = e; }
try { vm.runInContext('throw 5', sb); } catch (e) { out.thrownPrimitive = e; }
try { vm.runInContext('class E2 extends Error {}; throw new E2("sub")', sb); } catch (e) { out.sub = [e.name, e.message, e instanceof Error]; }
out.errorProps = (() => { try { vm.runInContext('const e = new Error("m"); e.extra = 7; throw e', sb); } catch (e) { return [e.message, e.extra, Object.prototype.toString.call(e)]; } })();
out.afterError = vm.runInContext('typeof e', sb);
try { vm.runInNewContext('a.b.c', { a: {} }, { filename: 'new-ctx.js' }); } catch (e) { out.newCtx = [e.name, String(e.stack).includes('new-ctx.js')]; }
const script = new vm.Script('throw new Error("from script")', { filename: 'script-file.js' });
try { script.runInContext(vm.createContext({})); } catch (e) { out.scriptStack = String(e.stack).includes('script-file.js'); }
try { script.runInNewContext({}); } catch (e) { out.scriptNew = e.message; }
return out;
"#####),
        r#####"{"typeError":["TypeError",false,"TypeError",true],"syntax":["SyntaxError",false,true],"thrownObject":{"custom":1},"thrownPrimitive":5,"sub":["Error","sub",false],"errorProps":["m",7,"[object Error]"],"afterError":"object","newCtx":["TypeError",true],"scriptStack":true,"scriptNew":"from script"}"#####
    );
}

#[test]
fn l_vm_nested() {
    // expected: node22
    assert_eq!(
        p2c_run("l_vm_nested", r#####"const vm = require('node:vm');
const out = {};
const outer = vm.createContext({ vm, inner: vm.createContext({}) });
out.nested = vm.runInContext('vm.runInContext("var deep = 1; deep + 1", inner)', outer);
out.innerDeclared = [vm.runInContext('typeof deep', outer), vm.runInContext('deep', outer.inner)];
try { vm.runInContext('vm.runInContext("while(true){}", inner, { timeout: 20 })', outer, { timeout: 2000 }); } catch (e) { out.innerTimeout = e.code; }
try { vm.runInContext('vm.runInContext("1+1", inner, { timeout: 2000 }); while(true){}', outer, { timeout: 30 }); } catch (e) { out.outerTimeout = e.code; }
out.recovered = vm.runInContext('1 + 1', outer);
try { vm.runInContext('vm.runInContext("while(true){}", inner, { timeout: 2000 })', outer, { timeout: 30 }); } catch (e) { out.innerLongerThanOuter = e.code; }
const many = [];
for (let i = 0; i < 50; i++) { const c = vm.createContext({ i }); many.push(vm.runInContext('var sq = i * i; sq', c)); }
out.many = many.slice(0, 5).concat(many.length);
return out;
"#####),
        r#####"{"nested":2,"innerDeclared":["undefined",1],"innerTimeout":"ERR_SCRIPT_EXECUTION_TIMEOUT","outerTimeout":"ERR_SCRIPT_EXECUTION_TIMEOUT","recovered":2,"innerLongerThanOuter":"ERR_SCRIPT_EXECUTION_TIMEOUT","many":[0,1,4,9,16,50]}"#####
    );
}

#[test]
fn l_vm_timeout() {
    // expected: node22
    assert_eq!(
        p2c_run("l_vm_timeout", r#####"const vm = require('node:vm');
const out = {};
const sb = vm.createContext({});
try { vm.runInContext('while(true){}', sb, { timeout: 50 }); } catch (e) { out.t1 = [e.code, e.message, e.name, e instanceof Error]; }
out.afterTimeout = vm.runInContext('1+1', sb);
try { vm.runInContext('for(;;){}', vm.createContext({}), { timeout: 20 }); } catch (e) { out.t2 = [e.code, e.message]; }
try { vm.runInThisContext('while(true){}', { timeout: 30 }); } catch (e) { out.t3 = e.code; }
try { new vm.Script('while(true){}').runInNewContext({}, { timeout: 25 }); } catch (e) { out.t4 = [e.code, e.message]; }
out.fastOk = vm.runInContext('let n = 0; for (let i = 0; i < 1000; i++) n += i; n', sb, { timeout: 1000 });
try { vm.runInContext('1', sb, { timeout: 0 }); } catch (e) { out.badZero = [e.code, e.name]; }
try { vm.runInContext('1', sb, { timeout: 'x' }); } catch (e) { out.badStr = [e.code, e.name]; }
try { vm.runInContext('1', sb, { timeout: 1.5 }); } catch (e) { out.badFrac = [e.code, e.name]; }
try { vm.runInContext('try { while(true){} } catch (e) { caught = true }', sb, { timeout: 25 }); } catch (e) { out.uncatchable = [e.code, sb.caught]; }
return out;
"#####),
        r#####"{"t1":["ERR_SCRIPT_EXECUTION_TIMEOUT","Script execution timed out after 50ms","Error",false],"afterTimeout":2,"t2":["ERR_SCRIPT_EXECUTION_TIMEOUT","Script execution timed out after 20ms"],"t3":"ERR_SCRIPT_EXECUTION_TIMEOUT","t4":["ERR_SCRIPT_EXECUTION_TIMEOUT","Script execution timed out after 25ms"],"fastOk":499500,"badZero":["ERR_OUT_OF_RANGE","RangeError"],"badStr":["ERR_INVALID_ARG_TYPE","TypeError"],"badFrac":["ERR_OUT_OF_RANGE","RangeError"],"uncatchable":["ERR_SCRIPT_EXECUTION_TIMEOUT",null]}"#####
    );
}

#[test]
fn l_vm_async() {
    // expected: node22
    assert_eq!(
        p2c_run("l_vm_async", r#####"const vm = require('node:vm');
const out = {};
const sb = vm.createContext({ log: [] });
const p = vm.runInContext('(async () => { await null; log.push("a"); return 5; })()', sb);
out.isPromise = typeof p.then;
out.value = await p;
vm.runInContext('Promise.resolve(7).then(v => { log.push("then" + v); })', sb);
await new Promise((r) => setTimeout(r, 5));
out.log = sb.log;
vm.runInContext('class Counter { static n = 0; static inc() { return ++Counter.n; } }', sb);
out.cls = [vm.runInContext('Counter.inc() + Counter.inc()', sb), vm.runInContext('typeof Counter', sb)];
try { vm.runInContext('let dup = 1', sb); vm.runInContext('let dup = 2', sb); } catch (e) { out.dup = [e.name, e instanceof SyntaxError]; }
vm.runInContext('var fnCalls = 0; function bump() { return ++fnCalls }', sb);
out.bump = [sb.bump(), sb.bump(), sb.fnCalls];
sb.bump = () => 100;
out.replaced = vm.runInContext('bump()', sb);
sb.late = 'x';
out.late = vm.runInContext('late + late', sb);
delete sb.late;
out.deleted = vm.runInContext('typeof late', sb);
return out;
"#####),
        r#####"{"isPromise":"function","value":5,"log":["a","then7"],"cls":[3,"function"],"dup":["SyntaxError",false],"bump":[1,2,2],"replaced":100,"late":"xx","deleted":"undefined"}"#####
    );
}

#[test]
fn l_repl_basic() {
    // expected: node22
    assert_eq!(
        p2c_run("l_repl_basic", r#####"const repl = require('node:repl');
const { PassThrough } = require('node:stream');
const input = new PassThrough(), output = new PassThrough();
let text = '';
output.on('data', (c) => { text += c; });
const server = repl.start({ input, output, terminal: false, prompt: '> ', useColors: false });
input.write('var a = 5\n');
input.write('a * 2\n');
input.write('let b = a + 1\n');
input.write('b\n');
input.write('[1,2,3].map(x => x * a)\n');
input.write('nope\n');
input.write('function add(x, y) {\n  return x + y;\n}\n');
input.write('add(2, 3)\n');
input.write('.exit\n');
await new Promise((r) => setTimeout(r, 50));
return { text, ctxKeys: Object.keys(server.context).filter((k) => ['a', 'add'].includes(k)).sort(), closed: server.closed === true || undefined };
"#####),
        r#####"{"text":"> undefined\n> 10\n> undefined\n> 6\n> [ 5, 10, 15 ]\n> Uncaught ReferenceError: nope is not defined\n> ... ... undefined\n> 5\n> ","ctxKeys":["a","add"],"closed":true}"#####
    );
}
