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

// `Array.prototype` is an Array exotic object. Expectations are Node 22 (v22.23.1) output, from:
//   console.log(Array.isArray(Array.prototype), Object.prototype.toString.call(Array.prototype));
//   console.log(Array.prototype.length, JSON.stringify(Object.getOwnPropertyDescriptor(Array.prototype, 'length')));
//   console.log(JSON.stringify(Object.getOwnPropertyNames(Array.prototype)));
//   console.log(JSON.stringify(Array.prototype)); console.log(Array.prototype);

#[test]
fn array_prototype_is_an_array() {
    assert_eq!(
        one("console.log(Array.isArray(Array.prototype), Object.prototype.toString.call(Array.prototype));"),
        "true [object Array]"
    );
}

#[test]
fn array_prototype_length_descriptor() {
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor(Array.prototype, 'length'); console.log(Array.prototype.length, JSON.stringify(d));"),
        "0 {\"value\":0,\"writable\":true,\"enumerable\":false,\"configurable\":false}"
    );
}

#[test]
fn array_prototype_own_names_start_with_length() {
    assert_eq!(
        one("console.log(JSON.stringify(Object.getOwnPropertyNames(Array.prototype)));"),
        "[\"length\",\"constructor\",\"at\",\"concat\",\"copyWithin\",\"fill\",\"find\",\"findIndex\",\"findLast\",\"findLastIndex\",\"lastIndexOf\",\"pop\",\"push\",\"reverse\",\"shift\",\"unshift\",\"slice\",\"sort\",\"splice\",\"includes\",\"indexOf\",\"join\",\"keys\",\"entries\",\"values\",\"forEach\",\"filter\",\"flat\",\"flatMap\",\"map\",\"every\",\"some\",\"reduce\",\"reduceRight\",\"toReversed\",\"toSorted\",\"toSpliced\",\"with\",\"toLocaleString\",\"toString\"]"
    );
}

#[test]
fn array_prototype_json_and_inspect() {
    assert_eq!(one("console.log(JSON.stringify(Array.prototype));"), "[]");
    assert_eq!(one("console.log(Array.prototype);"), "Object(0) []");
}

#[test]
fn object_created_from_array_prototype_stays_plain() {
    assert_eq!(
        one("const o = Object.create(Array.prototype); console.log(Array.isArray(o), Object.prototype.toString.call(o), JSON.stringify(o), o.length, o[0]);"),
        "false [object Object] {} 0 undefined"
    );
    assert_eq!(
        one("const o = Object.create(Array.prototype); o[0] = 5; console.log(o[0], o.length, Array.isArray(o), Array.prototype[0]);"),
        "5 0 false undefined"
    );
    assert_eq!(
        one("const o = Object.create(Array.prototype); let n = 0; for (const x of o) n++; console.log('iter', n, typeof o.push);"),
        "iter 0 function"
    );
    assert_eq!(
        one("const o = Object.create(Array.prototype); o.length = 2; console.log(o.length, Array.isArray(o));"),
        "2 false"
    );
}

#[test]
fn array_prototype_mutators_and_cleanup() {
    assert_eq!(
        one("console.log(Array.prototype.push(1), Array.prototype[0], Array.prototype.length); Array.prototype.length = 0; console.log(Array.prototype.length, Object.getOwnPropertyNames(Array.prototype).length, Array.prototype[0]);"),
        "1 1 1|0 40 undefined"
    );
}

#[test]
fn array_prototype_is_not_spread_by_concat_as_elements() {
    assert_eq!(
        one("console.log([].concat(Array.prototype).length, Array.prototype.concat([1], [2]).length);"),
        "0 2"
    );
}

#[test]
fn instances_are_unchanged() {
    assert_eq!(one("console.log([].length, Array.isArray([]));"), "0 true");
    assert_eq!(
        one("class A extends Array {} const a = new A(); a.push(3); console.log(Array.isArray(a), a.length, a[0]);"),
        "true 1 3"
    );
    assert_eq!(
        one("const z = [1, 2]; Object.setPrototypeOf(z, Array.prototype); console.log(z.length, Array.isArray(z), z[1]);"),
        "2 true 2"
    );
}
