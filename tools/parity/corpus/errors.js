const r=[];
function t(n,f){let v;try{v=f();}catch(e){v="THROW "+(e&&e.name)+": "+String(e&&e.message).slice(0,60);}r.push(n+" = "+String(v).replace(/\n/g,"\\n").slice(0,160));}
function G(p){return p.split(".").reduce((o,k)=>o==null?undefined:o[k],globalThis);}
// Error.captureStackTrace, prepareStackTrace, stack and cause shapes
t('Error.captureStackTrace typeof',()=>typeof Error.captureStackTrace);
t('Error.stackTraceLimit typeof',()=>typeof Error.stackTraceLimit);
t('Error.prepareStackTrace typeof',()=>typeof Error.prepareStackTrace);
t('captureStackTrace defines stack',()=>{const o={};Error.captureStackTrace(o);return typeof o.stack;});
t('captureStackTrace own keys',()=>{const o={};Error.captureStackTrace(o);return Object.keys(o).join();});
t('captureStackTrace own names',()=>{const o={};Error.captureStackTrace(o);return Object.getOwnPropertyNames(o).sort().join();});
t('captureStackTrace stack starts with message',()=>{const o={message:'hi',name:'Custom'};Error.captureStackTrace(o);return String(o.stack).split('\n')[0];});
t('captureStackTrace on function arg',()=>{function f(){}Error.captureStackTrace(f);return typeof f.stack;});
t('captureStackTrace rejects non-object',()=>{Error.captureStackTrace(1);return 'no throw';});
t('Error stack string',()=>typeof new Error('x').stack);
t('Error stack first line',()=>new Error('boom').stack.split('\n')[0]);
t('Error stack own property',()=>Object.getOwnPropertyNames(new Error('x')).sort().join());
t('Error stack not enumerable',()=>Object.keys(new Error('x')).join());
t('Error.prototype.stack',()=>typeof Error.prototype.stack);
t('Error cause own',()=>new Error('a',{cause:1}).cause);
t('Error cause keys',()=>Object.keys(new Error('a',{cause:1})).join());
t('Error cause absent',()=>'cause' in new Error('a'));
t('AggregateError errors',()=>new AggregateError([1,2],'m').errors.length);
t('AggregateError name',()=>new AggregateError([],'m').name);
t('AggregateError message',()=>new AggregateError([],'m').message);
t('AggregateError own names',()=>Object.getOwnPropertyNames(new AggregateError([],'m')).sort().join());
t('Error toString',()=>String(new TypeError('bad')));
t('Error name on prototype',()=>Object.getOwnPropertyDescriptor(Error.prototype,'name').enumerable);
t('Error message non-enumerable',()=>Object.getOwnPropertyDescriptor(new Error('m'),'message').enumerable);
t('SuppressedError typeof',()=>typeof globalThis.SuppressedError);
t('DisposableStack typeof',()=>typeof globalThis.DisposableStack);
t('Error called without new',()=>Error('x').message);
t('RangeError proto chain',()=>Object.getPrototypeOf(RangeError)===Error);
t('EvalError name',()=>new EvalError('x').name);
t('URIError name',()=>new URIError('x').name);
t('error.constructor name',()=>new TypeError('x').constructor.name);
t('throw in prepareStackTrace is catchable',()=>{const old=Error.prepareStackTrace;Error.prepareStackTrace=(e,s)=>'PREP';try{const x=new Error('q').stack;return x;}finally{Error.prepareStackTrace=old;}});
t('prepareStackTrace receives callsites array',()=>{const old=Error.prepareStackTrace;let arr=null;Error.prepareStackTrace=(e,s)=>{arr=Array.isArray(s);return 'x';};try{new Error('q').stack;return arr;}finally{Error.prepareStackTrace=old;}});
t('callsite getFunctionName typeof',()=>{const old=Error.prepareStackTrace;let v=null;Error.prepareStackTrace=(e,s)=>{v=typeof s[0].getFunctionName;return 'x';};try{new Error('q').stack;return v;}finally{Error.prepareStackTrace=old;}});
console.log(r.join("\n"));
