//! Builtin mutations that Node throws on even in sloppy code: array mutators on frozen, sealed or
//! non-extensible arrays (push/pop/shift/unshift/splice, then sort/reverse/fill/copyWithin on frozen),
//! `Object.setPrototypeOf` and `__proto__` on non-extensible objects, and `Object.freeze` on a typed
//! array with elements. Expected output is Node 22 (v22.23.1). Generator: write the `PROBE` source
//! to `probe.js` and run `node probe.js`; its stdout is `EXPECTED`, which is embedded verbatim.
#[cfg(test)]
use super::*;

/// Run `src` and return its console.log output lines (or the uncaught error text).
fn run_lines(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => std::mem::take(&mut it.out).join("\n"),
        Err(Ctl::Throw(v)) => {
            let mut out = std::mem::take(&mut it.out);
            out.push(format!("Uncaught {}", it.to_str(&v)));
            out.join("\n")
        }
        Err(_) => "stray completion".into(),
    }
}

const PROBE: &str = r##"
const r=[];
function t(n,f){try{r.push(n+" = "+String(f()));}catch(e){r.push(n+" = THROW "+e.name+": "+e.message);}}
const shapes={frozen:()=>Object.freeze([1,2,3]),sealed:()=>Object.seal([1,2,3]),nonext:()=>Object.preventExtensions([1,2,3])};
const muts={push:a=>a.push(9),pop:a=>a.pop(),shift:a=>a.shift(),unshift:a=>a.unshift(9),splice:a=>a.splice(0,1),
  sort:a=>a.sort(),reverse:a=>a.reverse(),fill:a=>a.fill(0),copyWithin:a=>a.copyWithin(0,1)};
for(const m in muts) for(const s in shapes) t(`sloppy ${s} ${m}`,()=>{const a=shapes[s]();const v=muts[m](a);return v===a?a.join():v;});
for(const m in muts) for(const s in shapes) t(`strict ${s} ${m}`,()=>{'use strict';const a=shapes[s]();const v=muts[m](a);return v===a?a.join():v;});
t('frozen empty push',()=>Object.freeze([]).push(1));
t('frozen empty push none',()=>Object.freeze([]).push());
t('frozen empty pop',()=>Object.freeze([]).pop());
t('sealed empty pop',()=>Object.seal([]).pop());
t('frozen empty shift',()=>Object.freeze([]).shift());
t('frozen empty unshift none',()=>Object.freeze([]).unshift());
t('nonext empty unshift',()=>Object.preventExtensions([]).unshift(1,2));
t('frozen len1 shift',()=>Object.freeze([1]).shift());
t('sealed len1 pop',()=>Object.seal([1]).pop());
t('nonext pop ok',()=>Object.preventExtensions([1,2]).pop());
t('nonext shift ok',()=>Object.preventExtensions([1,2]).shift());
t('nonext splice rm ok',()=>{const a=Object.preventExtensions([1,2]);return a.splice(0,1)+' '+a.join()});
t('sealed push none ok',()=>Object.seal([1]).push());
t('sealed unshift none ok',()=>Object.seal([1,2,3]).unshift());
t('sealed sort ok',()=>Object.seal([2,1]).sort().join());
t('nonext fill ok',()=>Object.preventExtensions([0,0]).fill(7).join());
t('sealed copyWithin ok',()=>Object.seal([1,2,3]).copyWithin(0,1).join());
t('frozen sort len1 ok',()=>Object.freeze([1]).sort().join());
t('frozen fill empty range ok',()=>Object.freeze([1,2]).fill(0,2).join());
t('frozen reverse len1 ok',()=>Object.freeze([1]).reverse().join());
t('frozen copyWithin no-op ok',()=>Object.freeze([1,2,3]).copyWithin(0,0,0).join());
t('frozen fill(0,1)',()=>Object.freeze([1,2]).fill(0,1));
t('frozen copyWithin(1,0)',()=>Object.freeze([1,2,3]).copyWithin(1,0));
t('frozen copyWithin(0,1)',()=>Object.freeze([1,2,3]).copyWithin(0,1));
t('frozen copyWithin(2,0,1)',()=>Object.freeze([1,2,3]).copyWithin(2,0,1));
t('frozen splice(2,1)',()=>Object.freeze([1,2,3]).splice(2,1));
t('frozen splice(2,1,5,6)',()=>Object.freeze([1,2,3]).splice(2,1,5,6));
t('frozen splice(1,1)',()=>Object.freeze([1,2,3]).splice(1,1));
t('frozen splice(3,0)',()=>Object.freeze([1,2,3]).splice(3,0));
t('frozen splice(0,0,9)',()=>Object.freeze([1,2,3]).splice(0,0,9));
t('nonext splice(3,0)',()=>Object.preventExtensions([1,2,3]).splice(3,0));
t('nonext splice(1,3)',()=>Object.preventExtensions([1,2,3]).splice(1,3));
t('nonext splice(3,0,4,5)',()=>Object.preventExtensions([1,2,3]).splice(3,0,4,5));
t('sealed splice(3,0,4)',()=>Object.seal([1,2,3]).splice(3,0,4));
t('sealed splice(1,2)',()=>Object.seal([1,2,3]).splice(1,2));
t('sealed splice(0,0) ok',()=>Object.seal([1,2]).splice(0,0).length);
t('partial sealed splice(0,1) state',()=>{const a=Object.seal([1,2,3]);try{a.splice(0,1)}catch(e){};return a.join()});
t('partial sealed shift state',()=>{const a=Object.seal([1,2,3]);try{a.shift()}catch(e){};return a.join()});
t('partial nonext splice state',()=>{const a=Object.preventExtensions([1,2,3]);try{a.splice(2,1,5,6,7)}catch(e){};return a.join()});
t('nonext push state',()=>{const a=Object.preventExtensions([1]);try{a.push(2)}catch(e){};return a.join()});
t('frozen setPrototypeOf same ok',()=>{const p={};const o=Object.freeze(Object.create(p));Object.setPrototypeOf(o,p);return 'ok'});
t('frozen setPrototypeOf null',()=>{Object.setPrototypeOf(Object.freeze({}),null);return 'ok'});
t('nonext setPrototypeOf',()=>{Object.setPrototypeOf(Object.preventExtensions({}),{});return 'ok'});
t('nonext setPrototypeOf same ok',()=>{Object.setPrototypeOf(Object.preventExtensions({}),Object.prototype);return 'ok'});
t('frozen class setPrototypeOf',()=>{class A{};Object.setPrototypeOf(Object.freeze(new A()),null);return 'ok'});
t('frozen array setPrototypeOf',()=>{Object.setPrototypeOf(Object.freeze([]),null);return 'ok'});
t('frozen null-proto setPrototypeOf',()=>{Object.setPrototypeOf(Object.freeze(Object.create(null)),{});return 'ok'});
t('frozen Map setPrototypeOf',()=>{Object.setPrototypeOf(Object.freeze(new Map()),null);return 'ok'});
t('frozen Object.create({}) setPrototypeOf',()=>{Object.setPrototypeOf(Object.freeze(Object.create({})),{});return 'ok'});
t('nonext __proto__ set',()=>{const o=Object.preventExtensions({});o.__proto__=null;return 'ok'});
t('nonext __proto__ set same ok',()=>{const o=Object.preventExtensions({});o.__proto__=Object.prototype;return 'ok'});
t('Reflect.setPrototypeOf frozen',()=>Reflect.setPrototypeOf(Object.freeze({}),{}));
t('Reflect.setPrototypeOf frozen same',()=>Reflect.setPrototypeOf(Object.freeze(Object.create(null)),null));
t('freeze u8 4',()=>Object.freeze(new Uint8Array(4)));
t('freeze empty u8',()=>{const u=Object.freeze(new Uint8Array(0));return [Object.isFrozen(u),Object.isSealed(u),Object.isExtensible(u)].join()});
t('freeze Float64Array 1',()=>Object.freeze(new Float64Array(1)));
t('freeze Buffer 0',()=>Object.isFrozen(Object.freeze(Buffer.alloc(0))));
t('freeze DataView',()=>Object.isFrozen(Object.freeze(new DataView(new ArrayBuffer(4)))));
t('seal u8 4',()=>Object.isSealed(Object.seal(new Uint8Array(4))));
t('preventExtensions u8 4',()=>Object.isExtensible(Object.preventExtensions(new Uint8Array(4))));
console.log(r.join("\n"));
"##;

/// Node 22 output of `PROBE` (one `name = value` line per probe).
const EXPECTED: &str = r##"
sloppy frozen push = THROW TypeError: Cannot add property 3, object is not extensible
sloppy sealed push = THROW TypeError: Cannot add property 3, object is not extensible
sloppy nonext push = THROW TypeError: Cannot add property 3, object is not extensible
sloppy frozen pop = THROW TypeError: Cannot delete property '2' of [object Array]
sloppy sealed pop = THROW TypeError: Cannot delete property '2' of [object Array]
sloppy nonext pop = 3
sloppy frozen shift = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
sloppy sealed shift = THROW TypeError: Cannot delete property '2' of [object Array]
sloppy nonext shift = 1
sloppy frozen unshift = THROW TypeError: Cannot add property 3, object is not extensible
sloppy sealed unshift = THROW TypeError: Cannot add property 3, object is not extensible
sloppy nonext unshift = THROW TypeError: Cannot add property 3, object is not extensible
sloppy frozen splice = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
sloppy sealed splice = THROW TypeError: Cannot delete property '2' of [object Array]
sloppy nonext splice = 1
sloppy frozen sort = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
sloppy sealed sort = 1,2,3
sloppy nonext sort = 1,2,3
sloppy frozen reverse = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
sloppy sealed reverse = 3,2,1
sloppy nonext reverse = 3,2,1
sloppy frozen fill = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
sloppy sealed fill = 0,0,0
sloppy nonext fill = 0,0,0
sloppy frozen copyWithin = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
sloppy sealed copyWithin = 2,3,3
sloppy nonext copyWithin = 2,3,3
strict frozen push = THROW TypeError: Cannot add property 3, object is not extensible
strict sealed push = THROW TypeError: Cannot add property 3, object is not extensible
strict nonext push = THROW TypeError: Cannot add property 3, object is not extensible
strict frozen pop = THROW TypeError: Cannot delete property '2' of [object Array]
strict sealed pop = THROW TypeError: Cannot delete property '2' of [object Array]
strict nonext pop = 3
strict frozen shift = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
strict sealed shift = THROW TypeError: Cannot delete property '2' of [object Array]
strict nonext shift = 1
strict frozen unshift = THROW TypeError: Cannot add property 3, object is not extensible
strict sealed unshift = THROW TypeError: Cannot add property 3, object is not extensible
strict nonext unshift = THROW TypeError: Cannot add property 3, object is not extensible
strict frozen splice = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
strict sealed splice = THROW TypeError: Cannot delete property '2' of [object Array]
strict nonext splice = 1
strict frozen sort = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
strict sealed sort = 1,2,3
strict nonext sort = 1,2,3
strict frozen reverse = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
strict sealed reverse = 3,2,1
strict nonext reverse = 3,2,1
strict frozen fill = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
strict sealed fill = 0,0,0
strict nonext fill = 0,0,0
strict frozen copyWithin = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
strict sealed copyWithin = 2,3,3
strict nonext copyWithin = 2,3,3
frozen empty push = THROW TypeError: Cannot add property 0, object is not extensible
frozen empty push none = THROW TypeError: Cannot assign to read only property 'length' of object '[object Array]'
frozen empty pop = THROW TypeError: Cannot assign to read only property 'length' of object '[object Array]'
sealed empty pop = undefined
frozen empty shift = THROW TypeError: Cannot assign to read only property 'length' of object '[object Array]'
frozen empty unshift none = THROW TypeError: Cannot assign to read only property 'length' of object '[object Array]'
nonext empty unshift = THROW TypeError: Cannot add property 0, object is not extensible
frozen len1 shift = THROW TypeError: Cannot delete property '0' of [object Array]
sealed len1 pop = THROW TypeError: Cannot delete property '0' of [object Array]
nonext pop ok = 2
nonext shift ok = 1
nonext splice rm ok = 1 2
sealed push none ok = 1
sealed unshift none ok = 3
sealed sort ok = 1,2
nonext fill ok = 7,7
sealed copyWithin ok = 2,3,3
frozen sort len1 ok = 1
frozen fill empty range ok = 1,2
frozen reverse len1 ok = 1
frozen copyWithin no-op ok = 1,2,3
frozen fill(0,1) = THROW TypeError: Cannot assign to read only property '1' of object '[object Array]'
frozen copyWithin(1,0) = THROW TypeError: Cannot assign to read only property '2' of object '[object Array]'
frozen copyWithin(0,1) = THROW TypeError: Cannot assign to read only property '0' of object '[object Array]'
frozen copyWithin(2,0,1) = THROW TypeError: Cannot assign to read only property '2' of object '[object Array]'
frozen splice(2,1) = THROW TypeError: Cannot delete property '2' of [object Array]
frozen splice(2,1,5,6) = THROW TypeError: Cannot assign to read only property '2' of object '[object Array]'
frozen splice(1,1) = THROW TypeError: Cannot assign to read only property '1' of object '[object Array]'
frozen splice(3,0) = THROW TypeError: Cannot assign to read only property 'length' of object '[object Array]'
frozen splice(0,0,9) = THROW TypeError: Cannot add property 3, object is not extensible
nonext splice(3,0) = 
nonext splice(1,3) = 2,3
nonext splice(3,0,4,5) = THROW TypeError: Cannot add property 3, object is not extensible
sealed splice(3,0,4) = THROW TypeError: Cannot add property 3, object is not extensible
sealed splice(1,2) = THROW TypeError: Cannot delete property '2' of [object Array]
sealed splice(0,0) ok = 0
partial sealed splice(0,1) state = 2,3,3
partial sealed shift state = 2,3,3
partial nonext splice state = 1,2,5
nonext push state = 1
frozen setPrototypeOf same ok = ok
frozen setPrototypeOf null = THROW TypeError: #<Object> is not extensible
nonext setPrototypeOf = THROW TypeError: #<Object> is not extensible
nonext setPrototypeOf same ok = ok
frozen class setPrototypeOf = THROW TypeError: #<A> is not extensible
frozen array setPrototypeOf = THROW TypeError: [object Array] is not extensible
frozen null-proto setPrototypeOf = THROW TypeError: [object Object] is not extensible
frozen Map setPrototypeOf = THROW TypeError: #<Map> is not extensible
frozen Object.create({}) setPrototypeOf = THROW TypeError: #<Object> is not extensible
nonext __proto__ set = THROW TypeError: #<Object> is not extensible
nonext __proto__ set same ok = ok
Reflect.setPrototypeOf frozen = false
Reflect.setPrototypeOf frozen same = true
freeze u8 4 = THROW TypeError: Cannot freeze array buffer views with elements
freeze empty u8 = true,true,false
freeze Float64Array 1 = THROW TypeError: Cannot freeze array buffer views with elements
freeze Buffer 0 = true
freeze DataView = true
seal u8 4 = true
preventExtensions u8 4 = false
"##;

#[test]
fn builtin_integrity_matches_node_22() {
    assert_eq!(run_lines(PROBE).trim(), EXPECTED.trim());
}
