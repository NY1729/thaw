use super::*;
use crate::process_env::thaw_tjs_set_process_env;
use std::ffi::CString;

/// Run `src` in a fresh interpreter and return its console.log lines (expectations recorded from node 22).
fn lines(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    it.run(src)
        .unwrap_or_else(|_| panic!("program must run: {src}"));
    it.out
}

#[test]
fn env_write_coerces_to_string_and_reaches_libc() {
    let got = lines(
        "process.env.THAW_PENV_T1_A = 5; console.log(typeof process.env.THAW_PENV_T1_A, process.env.THAW_PENV_T1_A);\
         process.env.THAW_PENV_T1_A = undefined; console.log(process.env.THAW_PENV_T1_A);\
         process.env.THAW_PENV_T1_A = null; console.log(process.env.THAW_PENV_T1_A);\
         process.env.THAW_PENV_T1_A = true; console.log(process.env.THAW_PENV_T1_A);\
         process.env.THAW_PENV_T1_A = 1.5; console.log(process.env.THAW_PENV_T1_A);\
         process.env.THAW_PENV_T1_A = 'hello'; console.log(process.env.THAW_PENV_T1_A);",
    );
    assert_eq!(
        got,
        ["string 5", "undefined", "null", "true", "1.5", "hello"]
    );
    // the compiled read is libc getenv: it must see the last JS write
    assert_eq!(std::env::var("THAW_PENV_T1_A").as_deref(), Ok("hello"));
    std::env::remove_var("THAW_PENV_T1_A");
}

#[test]
fn env_inspect_shows_the_live_environment() {
    std::env::set_var("THAW_PENV_INSPECT", "x");
    let mut it = Interp::new();
    it.run("globalThis.penv = process.env; console.log(process.env);")
        .unwrap_or_else(|_| panic!("program must run"));
    // console.log goes through the same inspector as Interp::inspect
    assert!(
        it.out.join("|").contains("THAW_PENV_INSPECT: 'x'"),
        "{:?}",
        it.out
    );
    let g = it.global.clone();
    let env = it.lookup(&g, "penv").expect("penv");
    let text = it.inspect(&env, false);
    assert!(text.contains("THAW_PENV_INSPECT: 'x'"), "{text}");
    assert!(lines("console.log(JSON.stringify(process.env).includes('\"THAW_PENV_INSPECT\":\"x\"'), Object.keys(process.env).includes('THAW_PENV_INSPECT'));")
        == ["true true"]);
    std::env::remove_var("THAW_PENV_INSPECT");
}

#[test]
fn env_delete_in_and_has_own_match_node() {
    let got = lines(
        "process.env.THAW_PENV_T2_A = 'x';\
         console.log('THAW_PENV_T2_A' in process.env, 'THAW_PENV_T2_NOPE' in process.env, 'toString' in process.env);\
         console.log(delete process.env.THAW_PENV_T2_A, 'THAW_PENV_T2_A' in process.env, process.env.THAW_PENV_T2_A);\
         console.log(process.env.hasOwnProperty('THAW_PENV_T2_A'), process.env.hasOwnProperty('toString'));",
    );
    assert_eq!(
        got,
        ["true false true", "true false undefined", "false false"]
    );
    assert!(std::env::var_os("THAW_PENV_T2_A").is_none());
}

#[test]
fn env_enumeration_and_json_see_writes() {
    let got = lines(
        "process.env.THAW_PENV_T3_B = '5';\
         console.log(Object.keys(process.env).includes('THAW_PENV_T3_B'), Object.entries(process.env).some(([k, v]) => k === 'THAW_PENV_T3_B' && v === '5'));\
         console.log(JSON.parse(JSON.stringify(process.env)).THAW_PENV_T3_B);\
         console.log(typeof process.env.hasOwnProperty, Object.getOwnPropertyDescriptor(process.env, 'THAW_PENV_T3_B').value, Object.getOwnPropertyDescriptor(process.env, 'THAW_PENV_T3_NOPE'));",
    );
    assert_eq!(got, ["true true", "5", "function 5 undefined"]);
    std::env::remove_var("THAW_PENV_T3_B");
}

#[test]
fn set_process_env_alias_writes_libc_env() {
    let name = CString::new("THAW_PENV_T4_ALIAS").unwrap();
    let value = CString::new("/opt/bin/esbuild").unwrap();
    assert_eq!(thaw_tjs_set_process_env(name.as_ptr(), value.as_ptr()), 1);
    assert_eq!(
        std::env::var("THAW_PENV_T4_ALIAS").as_deref(),
        Ok("/opt/bin/esbuild")
    );
    assert_eq!(
        lines("console.log(process.env.THAW_PENV_T4_ALIAS);"),
        ["/opt/bin/esbuild"]
    );
    // a name setenv refuses is not written
    let bad = CString::new("THAW=BAD").unwrap();
    assert_eq!(thaw_tjs_set_process_env(bad.as_ptr(), value.as_ptr()), 0);
    assert!(std::env::var_os("THAW=BAD").is_none());
    std::env::remove_var("THAW_PENV_T4_ALIAS");
}
