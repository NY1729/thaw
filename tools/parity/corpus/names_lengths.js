const r=[];
function t(n,f){let v;try{v=f();}catch(e){v="THROW "+(e&&e.name)+": "+String(e&&e.message).slice(0,60);}r.push(n+" = "+String(v).replace(/\n/g,"\\n").slice(0,160));}
function G(p){return p.split(".").reduce((o,k)=>o==null?undefined:o[k],globalThis);}
// name and length of builtins and global functions, and function-object own props
const paths=['Array','Array.from','Array.of','Array.prototype.map','Array.prototype.push','Array.prototype.concat','Array.prototype.splice','Array.prototype.toSorted','Array.prototype.with','Object','Object.assign','Object.defineProperty','Object.entries','Object.freeze','Object.groupBy','Object.fromEntries','Object.prototype.hasOwnProperty','Object.prototype.toString','Object.prototype.__defineGetter__','Object.prototype.propertyIsEnumerable','Function','Function.prototype','Function.prototype.call','Function.prototype.apply','Function.prototype.bind','Function.prototype.toString','String.prototype.padStart','String.prototype.replaceAll','String.prototype.at','String.fromCharCode','String.raw','Number','Number.parseFloat','Number.isNaN','Math.max','Math.hypot','Math.clz32','JSON.parse','JSON.stringify','Reflect.apply','Reflect.construct','Reflect.ownKeys','Promise','Promise.all','Promise.allSettled','Promise.any','Promise.race','Promise.withResolvers','Promise.prototype.then','Promise.prototype.finally','Map','Map.prototype.set','Map.groupBy','Set.prototype.union','Set.prototype.add','WeakMap.prototype.set','WeakRef.prototype.deref','Symbol','Symbol.for','Symbol.keyFor','Symbol.prototype.toString','BigInt','BigInt.asIntN','BigInt64Array','BigUint64Array','Date','Date.UTC','Date.prototype.getTime','RegExp','RegExp.prototype.exec','Error','Error.captureStackTrace','TypeError','AggregateError','Proxy.revocable','ArrayBuffer.isView','ArrayBuffer.prototype.slice','DataView.prototype.getInt8','Atomics.wait','Uint8Array','Uint8Array.from','Uint8Array.prototype.set','Generator','globalThis.queueMicrotask','queueMicrotask','structuredClone','atob','btoa','setTimeout','setInterval','setImmediate','clearTimeout','clearInterval','clearImmediate','parseInt','parseFloat','isNaN','isFinite','encodeURIComponent','decodeURIComponent','escape','unescape','eval','fetch','TextEncoder','TextDecoder','URL','URLSearchParams','AbortController','AbortSignal.timeout','Event','EventTarget','DOMException','Blob','Headers','Request','Response','WebSocket','BroadcastChannel','MessageChannel','performance.now','navigator','Intl.DisplayNames','Intl.Collator','Intl.NumberFormat','Intl.DateTimeFormat','Intl.Segmenter','Intl.getCanonicalLocales'];
for (const p of paths) t('fn '+p, ()=>{const f=G(p); if(f===undefined) return 'undefined'; if(typeof f!=='function') return typeof f; return typeof f+' name='+JSON.stringify(f.name)+' length='+f.length;});
t('fn name anon arrow',()=>(()=>1).name);
t('fn name bound',()=>(function a(x,y){}).bind(null,1).name+' '+(function a(x,y){}).bind(null,1).length);
t('fn name getter',()=>Object.getOwnPropertyDescriptor({get q(){return 1}},'q').get.name);
t('fn name setter',()=>Object.getOwnPropertyDescriptor({set q(v){}},'q').set.name);
t('fn name symbol method',()=>{const s=Symbol('d');return ({[s](){}})[s].name;});
t('fn name computed string',()=>{const k='dyn';return ({[k](){}})[k].name;});
t('fn name class static',()=>{class A{static m(){}};return A.m.name;});
t('fn name class',()=>{class A{};return A.name;});
t('fn name generator',()=>(function*g(){}).name);
t('fn name async',()=>(async function af(){}).name);
t('fn length default param',()=>((a,b=2,c)=>0).length);
t('fn length rest',()=>((a,...r)=>0).length);
t('Function.prototype own names',()=>Object.getOwnPropertyNames(Function.prototype).sort().join());
t('Function.prototype name empty',()=>JSON.stringify(Function.prototype.name));
t('Function.prototype length 0',()=>Function.prototype.length);
t('Function.prototype callable',()=>Function.prototype()===undefined);
t('Function own names of plain fn',()=>Object.getOwnPropertyNames(function q(){}).sort().join());
t('Function own names of arrow',()=>Object.getOwnPropertyNames(()=>1).sort().join());
t('Function own names of class',()=>{class A{};return Object.getOwnPropertyNames(A).sort().join();});
t('Function.prototype.toString native',()=>/\{\s*\[native code\]\s*\}/.test(Math.max.toString()));
t('Math.max toString',()=>Math.max.toString());
t('getter of Map.prototype.size name',()=>Object.getOwnPropertyDescriptor(Map.prototype,'size').get.name);
t('Symbol.prototype.description getter name',()=>Object.getOwnPropertyDescriptor(Symbol.prototype,'description').get.name);
t('Symbol.iterator name',()=>Symbol.iterator.toString());
t('Array.prototype[Symbol.unscopables] tag',()=>Object.prototype.toString.call(Array.prototype[Symbol.unscopables]));
console.log(r.join("\n"));
