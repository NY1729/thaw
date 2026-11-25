//! Parser gaps fixed by the vendored swc patch (`vendor/swc_ecma_parser/PATCH.md`): Annex B.3.4
//! `if (x) function f(){}` in sloppy code and escaped reserved words as IdentifierNames.

use crate::compile::CompileMode;
use crate::strict::StrictMode;
use crate::Interp;

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
fn if_function_declaration_is_var_hoisted_in_sloppy_code() {
    assert_eq!(
        run_all(
            "console.log(typeof f);\n\
             if (true) function f() { return 1; }\n\
             console.log(typeof f, f());\n\
             if (false) function g() {} else function h() { return 2; }\n\
             console.log(typeof g, typeof h, h());\n\
             (function () {\n\
               console.log(typeof k);\n\
               if (true) function k() { return 3; }\n\
               console.log(k());\n\
             })();"
        ),
        "undefined|function 1|undefined function 2|undefined|3"
    );
}

#[test]
fn if_function_declaration_stays_a_syntax_error_in_strict_and_for_generators() {
    assert_eq!(
        run_all(
            "function t(src) { try { (0, eval)(src); return 'ok'; } catch (e) { return e.name; } }\n\
             console.log(t('\"use strict\"; if (1) function f() {}'));\n\
             console.log(t('if (1) function* f() {}'));\n\
             console.log(t('if (1) async function f() {}'));\n\
             console.log(t('while (0) function f() {}'));\n\
             console.log(t('if (1) function f() {}'));"
        ),
        "SyntaxError|SyntaxError|SyntaxError|SyntaxError|ok"
    );
}

#[test]
fn escaped_reserved_words_are_property_names_but_not_identifiers() {
    assert_eq!(
        run_all(
            "var o = { bre\\u0061k: 1, \\u0069f: 2, aw\\u0061it: 3 };\n\
             console.log(o.break, o.if, o.await, o.bre\\u0061k, o.c\\u0061se);\n\
             class C { static cl\\u0061ss() { return 4; } get th\\u0069s() { return 5; } }\n\
             console.log(C.class(), new C().this);\n\
             function t(src) { try { (0, eval)(src); return 'ok'; } catch (e) { return e.name; } }\n\
             console.log(t('var bre\\\\u0061k;'), t('\\\\u0069f (1) ;'), t('var x = tru\\\\u0065;'));\n\
             console.log(t('({ \\\\u0069f: 1 })'), t('var \\\\u0076ar'), t('var aw\\\\u0061it;'));"
        ),
        "1 2 3 1 undefined|4 5|SyntaxError SyntaxError SyntaxError|ok SyntaxError ok"
    );
}
