// non-direct forms and sloppy global-level
globalThis.gv='G';
function f(){var gv='L';return [eval('gv'),(0,eval)('gv'),globalThis.eval('gv'),(eval)('gv'),eval?.('gv'),[eval][0]('gv'),(1&&eval)('gv'),eval.call(null,'gv'),new Function('return gv')()]}
console.log(JSON.stringify(f()));
console.log(eval('var topv=1;typeof topv'), typeof topv);
(function(){ eval('var a=1'); console.log(a); (function(){console.log(typeof a)})() })();
function h(){ eval('var h1=1'); return function(){return h1} } console.log(h()());
console.log(eval('(function(){return typeof this})()'));
function sf(){'use strict';return eval('(function(){return typeof this})()')} console.log(sf());
var e2=eval; var lv=1; console.log(e2('typeof lv'));
console.log((function(eval){return eval})(5));
console.log(eval("`${1+1}`"), eval('/a/g').flags);
console.log(eval('"use strict"; var s1=1; s1'), typeof s1);
