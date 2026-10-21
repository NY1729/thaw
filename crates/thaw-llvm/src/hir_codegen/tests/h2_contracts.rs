// H2: a class instance's user-defined toString/valueOf/Symbol.toPrimitive drive string
// conversion (template, String(), `+`) in ToPrimitive order. Expected output recorded from node 22.

#[test]
fn h2_contract_class_to_primitive_follows_spec_order() {
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

class Q {
    [Symbol.toPrimitive](hint: string): any {
        return hint === "number" ? 7 : "Q:" + hint;
    }
}

function main(): void {
    const p = new P();
    console.log(`${p}`, String(p), "" + p, p + "");
    const v = new V();
    console.log(`${v}`, String(v), v + 1);
    const q = new Q();
    console.log(`${q}`, String(q), q + "", +q);
}
"##;
    assert_eq!(
        compile_and_run(source, "h2_contract_class_to_primitive_follows_spec_order"),
        "P! P! P! P!
v v 6
Q:string Q:string Q:default 7
"
    );
}
