// Surface parity: own property names and descriptor shape for globals, process and key builtins.
// Output: one line per property, `path = kind;typeof;flags[;length;name;tag]`.
// Descriptors only: no getter is invoked, so the probe has no side effects.
const out=[];
const flags=d=>(d.enumerable?'e':'-')+(d.writable?'w':'-')+(d.configurable?'c':'-');
function keyName(k){return typeof k==='symbol'?'['+String(k)+']':k;}
function describe(path,o){
  if(o===undefined||o===null){out.push(path+' = absent');return;}
  if(typeof o!=='object'&&typeof o!=='function'){out.push(path+' = primitive '+typeof o);return;}
  let names=[];
  try{names=Object.getOwnPropertyNames(o).concat(Object.getOwnPropertySymbols(o));}catch(e){out.push(path+' = THROW '+e.name);return;}
  for(const k of names){
    const p=path+'.'+keyName(k);
    const d=Object.getOwnPropertyDescriptor(o,k);
    if(!d){out.push(p+' = missing-descriptor');continue;}
    if('value' in d){
      const v=d.value;
      let s='data;'+typeof v+';'+flags(d);
      if(typeof v==='function'){
        const ln=Object.getOwnPropertyDescriptor(v,'length'), nm=Object.getOwnPropertyDescriptor(v,'name');
        s+=';length='+(ln&&'value' in ln?ln.value:'none')+';name='+(nm&&'value' in nm?JSON.stringify(nm.value):'none');
      }
      if(v!==null&&(typeof v==='object'||typeof v==='function')){
        const tg=Object.getOwnPropertyDescriptor(v,Symbol.toStringTag);
        if(tg&&'value' in tg) s+=';tag='+JSON.stringify(tg.value);
      }
      out.push(p+' = '+s);
    }else{
      out.push(p+' = accessor;'+(d.get?'get':'')+(d.set?'set':'')+';'+flags(d));
    }
  }
}
const globalNames=['globalThis','process','Object','Function','Array','String','Number','Boolean','Symbol','BigInt','Math','JSON','Reflect','Atomics','Date','RegExp','Error','Map','Set','WeakMap','WeakSet','WeakRef','FinalizationRegistry','Promise','ArrayBuffer','SharedArrayBuffer','DataView','Uint8Array','Int8Array','Uint16Array','Int32Array','Float32Array','Float64Array','BigInt64Array','BigUint64Array','Proxy','Iterator','Buffer','URL','URLSearchParams','TextEncoder','TextDecoder','AbortController','AbortSignal','Event','EventTarget','DOMException','Blob','Headers','Request','Response','console','performance','crypto','navigator','WebAssembly'];
const protoNames=['Object.prototype','Function.prototype','Array.prototype','String.prototype','Number.prototype','Boolean.prototype','Symbol.prototype','BigInt.prototype','Date.prototype','RegExp.prototype','Error.prototype','Map.prototype','Set.prototype','WeakMap.prototype','WeakSet.prototype','Promise.prototype','ArrayBuffer.prototype','DataView.prototype','Uint8Array.prototype','Iterator.prototype'];
function resolve(p){return p.split('.').reduce((o,k)=>o==null?undefined:o[k],globalThis);}
// Generator.prototype is reached via a generator function, not a global name
const genProto=Object.getPrototypeOf(function*(){}).prototype;
for(const n of globalNames){
  const o=globalThis[n];
  describe('globalThis.'+n, o);
  if(o!==undefined&&o!==null&&(typeof o==='object'||typeof o==='function')&&n!=='globalThis') describe(n, o);
}
for(const p of protoNames){ describe(p, resolve(p)); }
describe('Generator.prototype(instance-proto)', genProto);
for(const k of ['Math','JSON','Reflect']) describe('tagged.'+k, globalThis[k]);
console.log(out.join('\n'));
