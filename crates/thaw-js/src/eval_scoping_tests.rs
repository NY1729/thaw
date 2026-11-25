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
