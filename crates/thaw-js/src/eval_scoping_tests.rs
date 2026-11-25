//! Spec rules of direct and indirect eval var scoping (EvalDeclarationInstantiation, PerformEval),
//! each run with the compiler off and with it on after the first call. Where V8 diverges from the
//! spec these cases are checked against test262, not Node.
use super::*;

/// The console lines of `src` run as a sloppy script, or its uncaught error, in both compile modes.
fn lines(src: &str) -> String {
    let mut seen: Vec<String> = Vec::new();
    for mode in [compile::CompileMode::Off, compile::CompileMode::On] {
        let mut it = Interp::with_compile_mode(mode);
        it.compile_after = 1;
        let out = match it.run_sloppy(src) {
            Ok(_) => it.out.join("|"),
            Err(Ctl::Throw(v)) => format!("{}|Uncaught {}", it.out.join("|"), it.to_str(&v)),
            Err(_) => "stray completion".into(),
        };
        seen.push(out);
    }
    assert_eq!(seen[0], seen[1], "compile off vs on differ for {src}");
    seen.remove(0)
}

fn throws_syntax(body: &str) -> String {
    lines(&format!(
        "function f(){{ {body} }} try {{ f(); console.log('no throw'); }} catch (e) {{ console.log(e.constructor.name); }}"
    ))
}

#[test]
fn param_expression_eval_cannot_declare_arguments() {
    assert_eq!(
        throws_syntax("function g(p = eval('var arguments')) {} g();"),
        "SyntaxError"
    );
    assert_eq!(
        throws_syntax("function g(p = eval('var arguments')) { var arguments; } g();"),
        "SyntaxError"
    );
    assert_eq!(
        throws_syntax("function g(p = eval('var arguments = 1'), arguments) {} g();"),
        "SyntaxError"
    );
    assert_eq!(
        throws_syntax("var g = (p = eval('var arguments'), arguments) => 1; g();"),
        "SyntaxError"
    );
    // an arrow without its own `arguments`, and a body-level eval, may declare it
    assert_eq!(
        throws_syntax("var g = (p = eval('var arguments')) => 1; g();"),
        "no throw"
    );
    assert_eq!(
        throws_syntax("function g(p = 1) { eval('var arguments = 7'); } g();"),
        "no throw"
    );
    assert_eq!(
        throws_syntax("function g(p = eval('var q = 5')) { return p; } g();"),
        "no throw"
    );
}

#[test]
fn strict_program_functions_are_strict_and_their_eval_vars_stay_local() {
    assert_eq!(
        lines("'use strict'; function a(){ var x = 0; eval('var x = 1'); return x; } function b(){ eval('var y = 1'); return typeof y; } console.log(a(), b(), typeof (function(){ return this; })());"),
        "0 undefined undefined"
    );
    assert_eq!(
        lines("'use strict'; function f(){ return this; } console.log(f());"),
        "undefined"
    );
}

#[test]
fn indirect_eval_declares_configurable_global_vars_and_confines_lexicals() {
    assert_eq!(
        lines("(0,eval)('var iv = 9; function iff(){}'); var d = Object.getOwnPropertyDescriptor(globalThis, 'iv'); console.log(d.enumerable, d.configurable, globalThis.propertyIsEnumerable('iff'), delete globalThis.iv, typeof iv);"),
        "true true true true undefined"
    );
    assert_eq!(
        lines("(0,eval)('let il = 1; const ic = 2;'); (0,eval)('\"use strict\"; var sv = 1; function sf(){}'); console.log(typeof il, typeof ic, typeof sv, typeof sf, 'sv' in globalThis);"),
        "undefined undefined undefined undefined false"
    );
    // an existing non-configurable global function keeps its attributes when eval redeclares it
    assert_eq!(
        lines("Object.defineProperty(globalThis, 'kept', { value: 1, writable: true, enumerable: true, configurable: false }); (0,eval)('function kept(){ return 2; }'); console.log(kept(), Object.getOwnPropertyDescriptor(globalThis, 'kept').configurable);"),
        "2 false"
    );
}

#[test]
fn eval_global_declarations_follow_can_declare_global() {
    // a function over a non-configurable, non-writable global is a TypeError and declares nothing
    assert_eq!(
        lines("try { eval('var neverDeclared; function NaN(){}'); } catch (e) { console.log(e.constructor.name, Object.getOwnPropertyDescriptor(globalThis, 'neverDeclared')); }"),
        "TypeError undefined"
    );
    assert_eq!(
        lines("try { (0,eval)('function Infinity(){}'); } catch (e) { console.log(e.constructor.name); }"),
        "TypeError"
    );
    // a configurable property is replaced by an enumerable, writable, configurable one
    assert_eq!(
        lines("Object.defineProperty(globalThis, 'cf', { value: 1, enumerable: false, writable: false, configurable: true }); eval('function cf(){}'); var d = Object.getOwnPropertyDescriptor(globalThis, 'cf'); console.log(d.enumerable, d.writable, d.configurable);"),
        "true true true"
    );
}

#[test]
fn define_property_on_global_keeps_the_requested_flags() {
    assert_eq!(
        lines("Object.defineProperty(globalThis, 'dp', { value: 1, enumerable: true, writable: true, configurable: false }); var d = Object.getOwnPropertyDescriptor(globalThis, 'dp'); console.log(d.enumerable, d.writable, d.configurable, delete globalThis.dp, dp);"),
        "true true false false 1"
    );
    assert_eq!(
        lines("Object.defineProperty(globalThis, 'dq', { value: 1, configurable: true }); var d = Object.getOwnPropertyDescriptor(globalThis, 'dq'); console.log(d.enumerable, d.configurable, delete globalThis.dq, typeof dq);"),
        "false true true undefined"
    );
}

#[test]
fn annexb_block_function_assigns_its_var_when_evaluated() {
    // the var exists (undefined) before the block runs and takes the function at the declaration
    assert_eq!(
        lines("function g(){ var before = typeof f; { before += typeof f; function f(){} before += typeof f; } return before + typeof f; } console.log(g());"),
        "undefinedfunctionfunctionfunction"
    );
    // a later top-level function declaration is overwritten by the block function
    assert_eq!(
        lines("function g(){ { function f(){ return 'inner'; } } return f(); function f(){ return 'outer'; } } console.log(g());"),
        "inner"
    );
    // switch cases and simple catch parameters
    assert_eq!(
        lines("function g(){ switch (1) { case 1: function f(){ return 1; } } return typeof f; } console.log(g());"),
        "function"
    );
    assert_eq!(
        lines("function g(){ try { throw 0; } catch (f) { { function f(){} } } return typeof f; } console.log(g());"),
        "function"
    );
}

#[test]
fn annexb_block_function_is_skipped_on_conflicts() {
    // an enclosing let, a parameter name and `arguments` get no var update
    assert_eq!(
        lines(
            "function g(){ { let f = 1; { function f(){} } } return typeof f; } console.log(g());"
        ),
        "undefined"
    );
    assert_eq!(
        lines("function g(f){ { function f(){} } return typeof f; } console.log(g(1));"),
        "number"
    );
    assert_eq!(
        lines(
            "function g(){ { function arguments(){} } return typeof arguments; } console.log(g());"
        ),
        "object"
    );
    // strict code has no Annex B binding
    assert_eq!(
        lines(
            "function g(){ 'use strict'; { function f(){} } return typeof f; } console.log(g());"
        ),
        "undefined"
    );
}

#[test]
fn annexb_in_eval_follows_the_enclosing_scopes() {
    assert_eq!(
        lines("function g(){ eval('{ function f(){} }'); return typeof f; } console.log(g());"),
        "function"
    );
    // an enclosing let in the caller blocks the var; its value is untouched
    assert_eq!(
        lines("function g(){ let f = 5; eval('{ function f(){} }'); return f; } console.log(g());"),
        "5"
    );
    // a global eval creates a configurable global
    assert_eq!(
        lines("eval('{ function ef(){} }'); var d = Object.getOwnPropertyDescriptor(globalThis, 'ef'); console.log(typeof ef, d.enumerable, d.configurable);"),
        "function true true"
    );
    // not created at all when the lexical clash is inside the eval text
    assert_eq!(
        lines("function g(){ eval('{ let f = 1; { function f(){} } }'); return typeof f; } console.log(g());"),
        "undefined"
    );
    // a var of the same name as a function in the caller is not a conflict
    assert_eq!(
        lines("function g(){ function h(){} eval('var h = 2'); return h; } console.log(g());"),
        "2"
    );
}
