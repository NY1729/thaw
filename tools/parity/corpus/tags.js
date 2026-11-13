const r=[];
function t(n,f){let v;try{v=f();}catch(e){v="THROW "+(e&&e.name)+": "+String(e&&e.message).slice(0,60);}r.push(n+" = "+String(v).replace(/\n/g,"\\n").slice(0,160));}
function G(p){return p.split(".").reduce((o,k)=>o==null?undefined:o[k],globalThis);}

// Object.prototype.toString tags of builtins, web classes, iterators and functions
const tag=o=>Object.prototype.toString.call(o);
for (const n of ['Math','JSON','Reflect','Atomics','WebAssembly','Symbol.prototype','Promise.prototype','Map.prototype','Set.prototype','WeakMap.prototype','WeakSet.prototype','ArrayBuffer.prototype','SharedArrayBuffer.prototype','DataView.prototype','BigInt.prototype','Date.prototype','RegExp.prototype','Error.prototype','Function.prototype','Object.prototype','globalThis','Iterator.prototype','Intl','console','performance','crypto','navigator'])
  t('tag '+n,()=>tag(G(n)));
t('tag Map iterator',()=>tag(new Map().entries()));
t('tag Set iterator',()=>tag(new Set().values()));
t('tag Array iterator',()=>tag([][Symbol.iterator]()));
t('tag String iterator',()=>tag(''[Symbol.iterator]()));
t('tag RegExp String iterator',()=>tag('a'.matchAll(/a/g)));
t('tag generator object',()=>tag((function*(){})()));
t('tag generator function',()=>tag(function*(){}));
t('tag async function',()=>tag(async function(){}));
t('tag async generator function',()=>tag(async function*(){}));
t('tag async generator object',()=>tag((async function*(){})()));
t('tag arrow fn',()=>tag(()=>1));
t('tag Buffer instance',()=>tag(G('Buffer')&&Buffer.alloc(1)));
t('tag URL',()=>tag(new URL('http://a')));
t('tag URLSearchParams',()=>tag(new URLSearchParams()));
t('tag Promise instance',()=>tag(Promise.resolve()));
t('tag Error instance',()=>tag(new Error('x')));
t('tag TypeError instance',()=>tag(new TypeError('x')));
t('tag AggregateError instance',()=>tag(new AggregateError([],'x')));
t('tag AbortController',()=>tag(new AbortController()));
t('tag AbortSignal',()=>tag(new AbortController().signal));
t('tag Event',()=>tag(new Event('x')));
t('tag EventTarget',()=>tag(new EventTarget()));
t('tag TextEncoder',()=>tag(new TextEncoder()));
t('tag TextDecoder',()=>tag(new TextDecoder()));
t('tag WeakRef',()=>tag(new WeakRef({})));
t('tag Headers',()=>tag(G('Headers')&&new Headers()));
t('tag Blob',()=>tag(G('Blob')&&new Blob()));
t('tag DOMException',()=>tag(G('DOMException')&&new DOMException('m')));
t('tag Proxy target',()=>tag(new Proxy({},{})));
t('tag plain class instance',()=>{class A{};return tag(new A());});
t('tag class constructor',()=>{class A{};return tag(A);});
t('tag null',()=>tag(null));
t('tag bigint',()=>tag(1n));
t('tag symbol',()=>tag(Symbol()));
t('tag Date',()=>tag(new Date(0)));
t('tag RegExp',()=>tag(/a/));
t('tag Map',()=>tag(new Map()));
t('tag typed array Uint8Array',()=>tag(new Uint8Array(1)));
t('tag Float16Array',()=>tag(G('Float16Array')&&new Float16Array(1)));
t('tag Uint8Array prototype toStringTag getter',()=>tag(Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype),Symbol.toStringTag).get.call(new Uint8Array(1))));
t('toStringTag Math value',()=>Math[Symbol.toStringTag]);
t('toStringTag JSON value',()=>JSON[Symbol.toStringTag]);
t('toStringTag Reflect value',()=>Reflect[Symbol.toStringTag]);
t('toStringTag Promise.prototype value',()=>Promise.prototype[Symbol.toStringTag]);
t('toStringTag Symbol.prototype value',()=>Symbol.prototype[Symbol.toStringTag]);
t('toStringTag Map.prototype value',()=>Map.prototype[Symbol.toStringTag]);
t('toStringTag process value',()=>G('process')&&process[Symbol.toStringTag]);
t('toStringTag Buffer.prototype value',()=>G('Buffer')&&Buffer.prototype[Symbol.toStringTag]);
t('toStringTag Generator.prototype value',()=>Object.getPrototypeOf(function*(){}).prototype[Symbol.toStringTag]);
t('toStringTag AsyncFunction.prototype value',()=>Object.getPrototypeOf(async function(){})[Symbol.toStringTag]);
console.log(r.join("\n"));
