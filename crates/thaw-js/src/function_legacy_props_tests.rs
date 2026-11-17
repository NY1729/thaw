//! Own `arguments` and `caller` of a sloppy plain `function` (W8-b). Node 22 output, from a sloppy CJS
//! file run with `node probe.js`:
//! ```js
//! console.log(Object.getOwnPropertyNames(function(){}).join());           // length,name,arguments,caller,prototype
//! console.log(Object.getOwnPropertyNames(()=>1).join());                  // length,name
//! console.log(Object.getOwnPropertyNames(class{}).join());                // length,name,prototype
//! console.log(Object.getOwnPropertyNames(function(){'use strict'}).join()); // length,name,prototype
//! console.log(Object.getOwnPropertyNames({m(){}}.m).join());              // length,name
//! console.log(Object.getOwnPropertyNames(function*(){}).join());          // length,name,prototype
//! console.log(Object.getOwnPropertyNames(async function(){}).join());     // length,name
//! console.log(Object.getOwnPropertyNames(function(){}.bind(null)).join()); // length,name
//! function f(){}
//! console.log(JSON.stringify(Object.getOwnPropertyDescriptor(f,'caller'))); // {"value":null,"writable":false,"enumerable":false,"configurable":false}
//! console.log(Reflect.ownKeys(f).join());                                  // length,name,arguments,caller,prototype
//! console.log((()=>1).hasOwnProperty('caller'), (function(){'use strict'}).hasOwnProperty('arguments')); // false false
//! console.log(f.caller, f.arguments);                                      // null null
//! console.log(delete f.caller, f.caller);                                  // false null
//! console.log(Object.getOwnPropertyDescriptor(f,'arguments').value);       // null
//! try { Object.defineProperty(f,'caller',{value:1}) } catch (e) { console.log(e.message) } // Cannot redefine property: caller
//! (function(){ 'use strict'; try { f.caller = 1 } catch (e) { console.log(e.message) } })();
//! // Cannot assign to read only property 'caller' of function 'function f(){}'
//! (function(){ 'use strict'; try { delete f.caller } catch (e) { console.log(e.message) } })();
//! // Cannot delete property 'caller' of function f(){}
//! (function(){ 'use strict'; console.log(f.caller, f.arguments) })();      // null null
//! ```
//! Ceiling: `caller` is always `null` (Node answers the calling sloppy function), so the probe's
//! `caller` value line is not pinned here.

use crate::compile::CompileMode;
use crate::strict::StrictMode;
use crate::Interp;

/// Runs the sloppy script `src` under each engine with the switch on; every engine must print the same.
fn run_all(src: &str) -> String {
    let mut want: Option<Vec<String>> = None;
    for mode in [CompileMode::Off, CompileMode::On, CompileMode::Check] {
        let mut it = Interp::with_compile_mode(mode);
        it.compile_after = 1;
        it.set_strict_mode(StrictMode::On);
        it.run_sloppy(src)
            .unwrap_or_else(|_| panic!("{mode:?}: script must run: {src}"));
        let got = std::mem::take(&mut it.out);
        match &want {
            None => want = Some(got),
            Some(w) => assert_eq!(&got, w, "{mode:?} differs: {src}"),
        }
    }
    want.unwrap().join("|")
}

#[test]
fn function_legacy_props_own_names_follow_node() {
    assert_eq!(
        run_all(
            "console.log(Object.getOwnPropertyNames(function(){}).join());\n\
             console.log(Object.getOwnPropertyNames(function f(){}).join());\n\
             console.log(Object.getOwnPropertyNames(()=>1).join());\n\
             console.log(Object.getOwnPropertyNames(class{}).join());\n\
             console.log(Object.getOwnPropertyNames(function(){'use strict'}).join());\n\
             console.log(Object.getOwnPropertyNames({m(){}}.m).join());\n\
             console.log(Object.getOwnPropertyNames(function*(){}).join());\n\
             console.log(Object.getOwnPropertyNames(async function(){}).join());\n\
             console.log(Object.getOwnPropertyNames(function(){}.bind(null)).join());\n\
             console.log(Reflect.ownKeys(function(){}).join());"
        ),
        "length,name,arguments,caller,prototype|length,name,arguments,caller,prototype|length,name|\
         length,name,prototype|length,name,prototype|length,name|length,name,prototype|length,name|\
         length,name|length,name,arguments,caller,prototype"
    );
}

#[test]
fn function_legacy_props_descriptors_and_own_checks() {
    assert_eq!(
        run_all(
            "function f(){}\n\
             console.log(JSON.stringify(Object.getOwnPropertyDescriptor(f,'caller')));\n\
             console.log(JSON.stringify(Object.getOwnPropertyDescriptor(f,'arguments')));\n\
             console.log((()=>1).hasOwnProperty('caller'), (function(){'use strict'}).hasOwnProperty('arguments'), f.hasOwnProperty('caller'));"
        ),
        "{\"value\":null,\"writable\":false,\"enumerable\":false,\"configurable\":false}|\
         {\"value\":null,\"writable\":false,\"enumerable\":false,\"configurable\":false}|false false true"
    );
}

#[test]
fn function_legacy_props_reads_and_sloppy_writes() {
    assert_eq!(
        run_all(
            "function f(){}\n\
             console.log(f.caller, f.arguments);\n\
             f.caller = 1;\n\
             console.log(f.caller, delete f.caller, f.caller);"
        ),
        "null null|null false null"
    );
}

#[test]
fn function_legacy_props_define_and_strict_refusals_match_v8() {
    assert_eq!(
        run_all(
            "function f(){}\n\
             try { Object.defineProperty(f,'caller',{value:1}) } catch (e) { console.log(e.message) }\n\
             Object.defineProperty(f,'arguments',{value:null}); console.log('same value ok');\n\
             (function(){ 'use strict'; try { f.caller = 1 } catch (e) { console.log(e.message) } })();\n\
             (function(){ 'use strict'; try { f.arguments = 1 } catch (e) { console.log(e.message) } })();\n\
             (function(){ 'use strict'; try { delete f.caller } catch (e) { console.log(e.message) } })();\n\
             (function(){ 'use strict'; console.log(f.caller, f.arguments) })();"
        ),
        "Cannot redefine property: caller|same value ok|\
         Cannot assign to read only property 'caller' of function 'function f(){}'|\
         Cannot assign to read only property 'arguments' of function 'function f(){}'|\
         Cannot delete property 'caller' of function f(){}|null null"
    );
}
