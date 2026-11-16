use super::*;

/// Run `src` and return the console.log lines (or the uncaught error text).
fn run(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out,
        Err(Ctl::Throw(v)) => {
            let mut out = it.out.clone();
            out.push(format!("Uncaught {}", it.to_str(&v)));
            out
        }
        Err(Ctl::Unsupported(f)) => vec![format!("unsupported: {f}")],
        Err(_) => vec!["stray completion".into()],
    }
}

fn one(src: &str) -> String {
    run(src).join("|")
}

// Node: Object.keys(setTimeout(...)) and Object.keys(setImmediate(...)) list these enumerable
// string-keyed own properties, in this order (verified with node v22).

#[test]
fn timeout_own_keys_match_node() {
    assert_eq!(
        one("var t = setTimeout(function () {}, 1); console.log(Object.keys(t).join()); clearTimeout(t);"),
        "_idleTimeout,_idlePrev,_idleNext,_idleStart,_onTimeout,_timerArgs,_repeat,_destroyed"
    );
    assert_eq!(
        one("var t = setTimeout(function () {}, 1); console.log(Object.getOwnPropertyNames(t).sort().join()); clearTimeout(t);"),
        "_destroyed,_idleNext,_idlePrev,_idleStart,_idleTimeout,_onTimeout,_repeat,_timerArgs"
    );
}

#[test]
fn timeout_own_property_values_match_node() {
    assert_eq!(
        one("var t = setTimeout(function () {}, 5); console.log(t._idleTimeout, typeof t._onTimeout, typeof t._idleStart, t._timerArgs, t._repeat, t._destroyed); clearTimeout(t);"),
        "5 function number undefined null false"
    );
    assert_eq!(
        one("var t = setInterval(function () {}, 7); console.log(t._idleTimeout, t._repeat); clearInterval(t);"),
        "7 7"
    );
    assert_eq!(
        one("var t = setTimeout(function (x, y) {}, 3, 1, 2); console.log(t._idleTimeout, t._timerArgs.join()); clearTimeout(t);"),
        "3 1,2"
    );
}

#[test]
fn immediate_own_keys_match_node() {
    assert_eq!(
        one("var i = setImmediate(function () {}); console.log(Object.keys(i).join()); clearImmediate(i);"),
        "_idleNext,_idlePrev,_onImmediate,_argv,_destroyed"
    );
    assert_eq!(
        one("var i = setImmediate(function (x) {}, 9); console.log(typeof i._onImmediate, i._argv.join(), i._destroyed); clearImmediate(i);"),
        "function 9 false"
    );
}
