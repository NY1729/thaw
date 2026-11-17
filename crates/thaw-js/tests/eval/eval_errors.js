function t(src){try{eval(src);console.log('ok')}catch(e){console.log(e.name+': '+e.message)}}
t('1+');t('let a;let a');t('var 1');t('}');t('if(');t('"abc');t('/*');t('throw new Error("x")');t('foo.bar');t('null.x');t('x=>');t('class{}');t('a b');t('1=2');t('new.target');t('super.x');t('yield 1');t('await 1');t('break');t('({a:1,a:2,__proto__:1,__proto__:2})');t('"use strict";with({}){}');t('"use strict";010');t('return 1');
try{eval('throw 1')}catch(e){console.log(e)}
try{eval('\n\nfoo()')}catch(e){console.log(e.name+': '+e.message)}
try{new Function('a','}')}catch(e){console.log(e.name+': '+e.message)}
try{setTimeout('1',1)}catch(e){console.log(e.name+' '+e.code)}
console.log(new Function('return typeof module')());
console.log(typeof setTimeout, eval.name, eval.length, JSON.stringify(Object.getOwnPropertyDescriptor(globalThis,'eval')));
