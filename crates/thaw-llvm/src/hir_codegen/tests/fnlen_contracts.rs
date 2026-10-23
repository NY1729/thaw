// An extracted method called without its receiver throws the per-site
// "Cannot read properties of undefined (reading 'x')" TypeError.
// Expected output recorded from node 22 (strict mode).

#[test]
fn fnlen_contract_extracted_method_receiver_message_names_property() {
    let source = r##"class K {
    x = 3;
    m(): number {
        return this.x;
    }
}

function run(): void {
    const o = { x: 7, get(): number { return this.x; } };
    const e = o.get;
    try { console.log(e()); } catch (err) { console.log((err as Error).message); }
    function make(): () => number { return function (this: any): number { return this.x; }; }
    const c = make();
    try { console.log(c()); } catch (err) { console.log((err as Error).message); }
    const cond = true;
    const b = cond ? o.get : o.get;
    try { console.log(b()); } catch (err) { console.log((err as Error).message); }
    const km = new K().m;
    try { console.log(km()); } catch (err) { console.log((err as Error).message); }
    const p = {
        x: 1,
        helper(): number { return 2; },
        callHelper(): number { return this.helper(); },
        write(): void { this.x = 5; },
    };
    const a = p.callHelper;
    try { console.log(a()); } catch (err) { console.log((err as Error).message); }
    const w = p.write;
    try { w(); console.log("ok"); } catch (err) { console.log((err as Error).message); }
}

function main(): void {
    run();
}
"##;
    assert_eq!(
        compile_and_run(source, "fnlen_contract_extracted_method_receiver_message_names_property"),
        "Cannot read properties of undefined (reading 'x')
Cannot read properties of undefined (reading 'x')
Cannot read properties of undefined (reading 'x')
Cannot read properties of undefined (reading 'x')
Cannot read properties of undefined (reading 'helper')
Cannot set properties of undefined (setting 'x')
"
    );
}
