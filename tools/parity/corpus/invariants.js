const r=[];
function t(n,f){let v;try{v=f();}catch(e){v="THROW "+(e&&e.name)+": "+String(e&&e.message).slice(0,60);}r.push(n+" = "+String(v).replace(/\n/g,"\\n").slice(0,160));}
function G(p){return p.split(".").reduce((o,k)=>o==null?undefined:o[k],globalThis);}

// strict-mode mutation violations and their sloppy-mode counterparts
t('strict frozen assign',()=>{'use strict';const o=Object.freeze({a:1});o.a=2;return 'no throw';});
t('sloppy frozen assign',()=>{const o=Object.freeze({a:1});o.a=2;return o.a;});
t('strict frozen add',()=>{'use strict';const o=Object.freeze({a:1});o.b=2;return 'no throw';});
t('strict frozen array push',()=>{const o=Object.freeze([1]);o.push(2);return 'no throw';});
t('strict frozen array index assign',()=>{'use strict';const o=Object.freeze([1]);o[0]=2;return 'no throw';});
t('strict sealed delete',()=>{'use strict';const o=Object.seal({a:1});delete o.a;return 'no throw';});
t('sloppy sealed delete',()=>{const o=Object.seal({a:1});return delete o.a;});
t('strict nonwritable assign',()=>{'use strict';const o={};Object.defineProperty(o,'x',{value:1});o.x=2;return 'no throw';});
t('strict getter-only assign',()=>{'use strict';const o={get x(){return 1}};o.x=2;return 'no throw';});
t('sloppy getter-only assign',()=>{const o={get x(){return 1}};o.x=2;return o.x;});
t('strict non-extensible add',()=>{'use strict';const o=Object.preventExtensions({});o.y=2;return 'no throw';});
t('sloppy non-extensible add',()=>{const o=Object.preventExtensions({});o.y=2;return o.y;});
t('strict delete nonconfigurable',()=>{'use strict';const o={};Object.defineProperty(o,'k',{value:1});delete o.k;return 'no throw';});
t('defineProperty on frozen',()=>{Object.defineProperty(Object.freeze({}),'z',{value:1});return 'no throw';});
t('setPrototypeOf on frozen',()=>{Object.setPrototypeOf(Object.freeze({}),null);return 'no throw';});
t('Reflect.set frozen',()=>Reflect.set(Object.freeze({a:1}),'a',2));
t('Reflect.deleteProperty sealed',()=>Reflect.deleteProperty(Object.seal({a:1}),'a'));
t('Reflect.defineProperty frozen add',()=>Reflect.defineProperty(Object.freeze({}),'q',{value:1}));
t('frozen Map still mutable',()=>{const m=Object.freeze(new Map());m.set(1,1);return m.size;});
t('isFrozen empty object',()=>Object.isFrozen(Object.preventExtensions({})));
t('isSealed frozen',()=>Object.isSealed(Object.freeze({a:1})));
t('strict string index assign',()=>{'use strict';'abc'[0]='x';return 'no throw';});
t('strict primitive prop assign',()=>{'use strict';(5).x=1;return 'no throw';});
t('strict undeclared assign',()=>{'use strict';undeclaredVarXyz=1;return 'no throw';});
t('strict non-writable global undefined',()=>{'use strict';return typeof undefined;});
t('frozen array length set',()=>{'use strict';const a=Object.freeze([1,2]);a.length=0;return 'no throw';});
t('writable array length nonwritable',()=>{'use strict';const a=[1];Object.defineProperty(a,'length',{writable:false});a.push(2);return 'no throw';});
t('frozen class instance strict',()=>{'use strict';class A{constructor(){this.v=1;Object.freeze(this);}set(){this.v=2;}};new A().set();return 'no throw';});
t('frozen fn property',()=>{'use strict';const f=Object.freeze(function q(){});f.x=1;return 'no throw';});
t('getOwnPropertyDescriptor frozen',()=>JSON.stringify(Object.getOwnPropertyDescriptor(Object.freeze({a:1}),'a')));
t('Object.freeze returns same',()=>{const o={};return Object.freeze(o)===o;});
t('freeze typed array element strict',()=>{'use strict';const u=Object.freeze(new Uint8Array(2));u[0]=5;return 'no throw';});
console.log(r.join("\n"));
