use super::*;

/// Run `src` and join its console.log lines with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

// `Object.defineProperty(arr, 'length', { writable: false })`: the array keeps its length, refuses
// growth and mutators, and reports the flag. Each expectation matches Node 22.

const RO: &str = "const a = [1, 2, 3]; Object.defineProperty(a, 'length', { writable: false });";

#[test]
fn read_only_length_reports_and_keeps_length() {
    assert_eq!(
        one(&format!("{RO} const d = Object.getOwnPropertyDescriptor(a, 'length'); console.log(d.writable, d.enumerable, d.configurable, a.length);")),
        "false false false 3"
    );
}

#[test]
fn read_only_length_silently_refuses_plain_writes() {
    assert_eq!(
        one(&format!(
            "{RO} a.length = 1; a[5] = 1; a[1] = 9; console.log(a.length, a[5], a.join());"
        )),
        "3 undefined 1,9,3"
    );
}

#[test]
fn read_only_length_throws_from_push() {
    assert_eq!(
        one(&format!("{RO} try {{ a.push(4) }} catch (e) {{ console.log(e.constructor.name + ': ' + e.message) }} console.log(a.length, a[3]);")),
        "TypeError: Cannot assign to read only property 'length' of object '[object Array]'|3 undefined"
    );
}

#[test]
fn read_only_length_throws_from_mutators_and_keeps_length() {
    // ponytail: pop/shift/unshift/splice throw before mutating (Node mutates elements first), so only
    // the length and the error are asserted
    assert_eq!(
        one(&format!("{RO} for (const f of [() => a.pop(), () => a.shift(), () => a.unshift(0), () => a.splice(0, 1)]) {{ try {{ f() }} catch (e) {{ console.log(e.constructor.name + ': ' + e.message) }} }} console.log(a.length);")),
        "TypeError: Cannot assign to read only property 'length' of object '[object Array]'\
|TypeError: Cannot assign to read only property 'length' of object '[object Array]'\
|TypeError: Cannot assign to read only property 'length' of object '[object Array]'\
|TypeError: Cannot assign to read only property 'length' of object '[object Array]'\
|3"
    );
}

#[test]
fn read_only_length_defineproperty_and_reflect_results() {
    assert_eq!(
        one(&format!(
            "{RO} Object.defineProperty(a, 'length', {{ value: 3 }}); console.log(a.length);"
        )),
        "3"
    );
    assert_eq!(
        one(&format!("{RO} try {{ Object.defineProperty(a, 'length', {{ value: 1 }}) }} catch (e) {{ console.log(e.constructor.name + ': ' + e.message) }}")),
        "TypeError: Cannot redefine property: length"
    );
    assert_eq!(
        one(&format!("{RO} try {{ Object.defineProperty(a, 'length', {{ writable: true }}) }} catch (e) {{ console.log(e.constructor.name + ': ' + e.message) }}")),
        "TypeError: Cannot redefine property: length"
    );
    assert_eq!(
        one(&format!("{RO} console.log(Reflect.defineProperty(a, 'length', {{ value: 1 }}), Reflect.set(a, 'length', 1), a.length);")),
        "false false 3"
    );
}

#[test]
fn length_defined_with_value_and_writable_false_truncates_then_locks() {
    assert_eq!(
        one("const b = [1, 2, 3]; Object.defineProperty(b, 'length', { value: 1, writable: false }); console.log(b.length, JSON.stringify(b), Object.getOwnPropertyDescriptor(b, 'length').writable);"),
        "1 [1] false"
    );
}

#[test]
fn read_only_length_keys_isfrozen_fill_and_index_define() {
    assert_eq!(
        one(&format!(
            "{RO} console.log(Reflect.ownKeys(a).join(), Object.isFrozen(a), a.fill(0).join());"
        )),
        "0,1,2,length false 0,0,0"
    );
    assert_eq!(
        one(&format!("{RO} try {{ Object.defineProperty(a, 5, {{ value: 1 }}) }} catch (e) {{ console.log(e.constructor.name + ': ' + e.message) }}")),
        "TypeError: Cannot define property 5, object is not extensible"
    );
}

#[test]
fn ordinary_and_frozen_arrays_are_unchanged() {
    assert_eq!(
        one("const c = [1]; c.push(2); c.length = 1; console.log(c.join(), c.length);"),
        "1 1"
    );
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor(Object.freeze([1]), 'length'); console.log(d.writable);"),
        "false"
    );
}
