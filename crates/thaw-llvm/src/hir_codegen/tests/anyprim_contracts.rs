// `any`-typed values that are live class instances with Symbol.toPrimitive
// must ToPrimitive with hint "default" for loose `==` and binary `+`, and
// "number"/"string" only where the operator asks for them. Expected output
// recorded from node 22.

#[test]
fn any_class_to_primitive_uses_default_hint_for_equality_and_add() {
    let source = r##"class Q {
    [Symbol.toPrimitive](hint: string): any {
        return "Q:" + hint;
    }
}

class N {
    [Symbol.toPrimitive](hint: string): any {
        return hint === "number" ? 7 : "N:" + hint;
    }
}

function main(): void {
    const q = new Q();
    const a: any = q;
    console.log(a == "Q:default", a + "", a + 1);
    const n = new N();
    const b: any = n;
    console.log(b == "N:default", b * 2, b - 1, b + "", b == 7, `${b}`, b > 6, String(b));
    const s: any = "x";
    const k: any = 5;
    console.log(s == "x", s + "", s + 1, k == 5, k + "", k + 1, k * 2);
}
"##;
    assert_eq!(
        compile_and_run(source, "any_class_to_primitive_uses_default_hint_for_equality_and_add"),
        "true Q:default Q:default1
true 14 6 N:default false N:string true N:string
true x x1 true 5 6 10
"
    );
}
