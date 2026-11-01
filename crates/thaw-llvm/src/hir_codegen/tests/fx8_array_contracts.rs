// Array loose equality against a string/number/any value compares the array's
// `join(",")` text; `Array.prototype.toLocaleString` formats each element with the
// locale arguments; `any`-typed `localeCompare` between strings uses ICU collation.
// Expected output recorded from node 22.

#[test]
fn fx8_contract_array_loose_equality_uses_join_text() {
    let source = r##"function main(): void {
    const s: any = "1,2";
    const a = [1, 2];
    const b: any = [1, 2];
    console.log(a == s, s == a, b == s, [1, 2] == "1,2", ["a"] == ("a" as any), [1, [2, 3]] == ("1,2,3" as any), [1] == (1 as any));
    console.log([] == false, [1] == true, [0] == "", [1, 2] != "1,2", [5] == 5);
}
"##;
    assert_eq!(
        compile_and_run(source, "fx8_contract_array_loose_equality_uses_join_text"),
        "true true true true true true true
true true false false true
"
    );
}

#[test]
fn fx8_contract_array_to_locale_string_formats_elements() {
    let source = r##"function main(): void {
    console.log([1234.5, 6].toLocaleString("de-DE", { maximumFractionDigits: 1 }));
    console.log([new Date(0)].toLocaleString("en-US", { timeZone: "UTC" }));
    console.log([1234.5, 6].toLocaleString("en-US", { maximumFractionDigits: 0 }));
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "fx8_contract_array_to_locale_string_formats_elements"
        ),
        "1.234,5,6
1/1/1970, 12:00:00 AM
1,235,6
"
    );
}
