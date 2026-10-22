// Loose `==` against a class instance with a ToPrimitive method uses ToPrimitive
// with hint "default" (not "number"/"string").
// Expected output recorded from node 22.

#[test]
fn looseeq_contract_object_to_primitive_uses_default_hint() {
    let source = r##"class P {
    toString(): string {
        return "P!";
    }
}

class V {
    valueOf(): number {
        return 5;
    }
    toString(): string {
        return "v";
    }
}

class O {}

class Q {
    [Symbol.toPrimitive](hint: string): any {
        return hint === "number" ? 7 : "Q:" + hint;
    }
}

function main(): void {
    const p = new P();
    const v = new V();
    const q = new Q();
    console.log(q == "Q:default", q == "Q:string", "Q:default" == q, q != "Q:default");
    console.log(p == "P!", v == 5, v == "v", v == "5");
    console.log(q + "", q + 1, v + "", v == 5);
    const o = new O();
    console.log(o == "[object Object]", o == 0, q == 7, q == "7");
}
"##;
    assert_eq!(
        compile_and_run(source, "looseeq_contract_object_to_primitive_uses_default_hint"),
        "true false true false
true true false true
Q:default Q:default1 5 true
true false false false
"
    );
}
