#[test]
fn compiles_variadic_string_concat() {
    let source = r#"
        function receiver(): string {
            console.log("receiver");
            return "value=";
        }
        function argument(): number {
            console.log("argument");
            return 42;
        }
        async function delayed(): Promise<boolean> {
            await sleep(1);
            console.log("awaited-concat");
            return true;
        }
        async function main(): Promise<void> {
            const values: number[] = [1, 2];
            console.log(receiver().concat(argument(), ";values=", values, ";object=", { x: 1 }));
            console.log("empty".concat());
            console.log("flag=".concat(await delayed()));
            console.log(receiver().concat(argument(), await delayed()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_concat_method"),
        "receiver\nargument\nvalue=42;values=1,2;object=[object Object]\nempty\nawaited-concat\nflag=true\nreceiver\nargument\nawaited-concat\nvalue=42true\n"
    );
}

#[test]
fn compiles_native_string_repeat() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "ab";
        }
        function count(): number {
            console.log("count");
            return 2;
        }
        async function delayedText(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "😀";
        }
        async function delayedCount(): Promise<number> {
            console.log("awaited count");
            await sleep(1);
            return 2;
        }
        async function main(): Promise<void> {
            console.log("ab".repeat(3));
            console.log("😀".repeat(2));
            console.log("xy".repeat(2.9));
            console.log("x".repeat(0 / 0).length);
            console.log("z".repeat("2"));
            console.log(text().repeat(count()));
            console.log((await delayedText()).repeat(await delayedCount()));
            try {
                "x".repeat(-1);
            } catch (error) {
                console.log(error);
            }
            try {
                "x".repeat(Infinity);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_repeat"),
        "ababab\n😀😀\nxyxy\n0\nzz\nreceiver\ncount\nabab\nawaited receiver\nawaited count\n😀😀\nInvalid count value for String.prototype.repeat\nInvalid count value for String.prototype.repeat\n"
    );
}

#[test]
fn compiles_native_string_pad_start_and_pad_end() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "5";
        }
        function length(): number {
            console.log("length");
            return 3;
        }
        function pad(): string {
            console.log("pad");
            return "0";
        }
        async function delayedText(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log("5".padStart(3, "0"));
            console.log("5".padEnd(3, "0"));
            console.log("abc".padStart(2, "0"));
            console.log("5".padStart(3));
            console.log("1".padStart(5, "ab"));
            console.log("x".padStart(5, ""));
            console.log("5".padStart(3, 0));
            console.log("😀".padStart(3, "x"));
            console.log(text().padStart(length(), pad()));
            console.log((await delayedText()).padStart(3, "x"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_pad_start_and_end"),
        "005\n500\nabc\n  5\nabab1\nx\n005\nx😀\nreceiver\nlength\npad\n005\nawaited receiver\nx😀\n"
    );
}

#[test]
fn compiles_native_number_to_fixed() {
    let source = r#"
        function value(): number {
            console.log("receiver");
            return 1.5;
        }
        function digits(): number {
            console.log("digits");
            return 2;
        }
        async function delayedValue(): Promise<number> {
            console.log("awaited receiver");
            await sleep(1);
            return 3.14159;
        }
        async function main(): Promise<void> {
            console.log((1.5).toFixed(2));
            console.log((3.7).toFixed());
            console.log((5).toFixed());
            console.log((-1.005).toFixed(2));
            console.log((1e21).toFixed(2));
            console.log((0 / 0).toFixed(2));
            console.log(value().toFixed(digits()));
            console.log((await delayedValue()).toFixed(3));
            try {
                (1).toFixed(-1);
            } catch (error) {
                console.log(error);
            }
            try {
                (1).toFixed(101);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_to_fixed"),
        "1.50\n4\n5\n-1.00\n1e+21\nNaN\nreceiver\ndigits\n1.50\nawaited receiver\n3.142\ntoFixed() digits argument must be between 0 and 100\ntoFixed() digits argument must be between 0 and 100\n"
    );
}

#[test]
fn compiles_native_string_code_point_at() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "a";
        }
        function index(): number {
            console.log("index");
            return 0;
        }
        async function delayedText(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log("a".codePointAt(0));
            console.log("a".codePointAt());
            console.log("😀".codePointAt(0));
            console.log("😀".codePointAt(1));
            console.log("a".codePointAt(5));
            console.log("a".codePointAt(-1));
            console.log(text().codePointAt(index()));
            console.log((await delayedText()).codePointAt(0));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_code_point_at"),
        "97\n97\n128512\n56832\nundefined\nundefined\nreceiver\nindex\n97\nawaited receiver\n128512\n"
    );
}

#[test]
fn compiles_encode_uri_component() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "a=1&b=2";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log(encodeURIComponent("a b"));
            console.log(encodeURIComponent("a=1&b=2"));
            console.log(encodeURIComponent("abc-_.!~*'()123"));
            console.log(encodeURIComponent("café"));
            console.log(encodeURIComponent(value()));
            console.log(encodeURIComponent(await delayedValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "encode_uri_component"),
        "a%20b\na%3D1%26b%3D2\nabc-_.!~*'()123\ncaf%C3%A9\nargument\na%3D1%26b%3D2\nawaited argument\n%F0%9F%98%80\n"
    );
}

#[test]
fn compiles_decode_uri_component() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "a%3D1%26b%3D2";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "%F0%9F%98%80";
        }
        async function main(): Promise<void> {
            console.log(decodeURIComponent("a%20b"));
            console.log(decodeURIComponent("a%3D1%26b%3D2"));
            console.log(decodeURIComponent("abc-_.!~*'()123"));
            console.log(decodeURIComponent("caf%C3%A9"));
            console.log(decodeURIComponent(value()));
            console.log(decodeURIComponent(await delayedValue()));
            try {
                decodeURIComponent("%");
            } catch (error) {
                console.log(error);
            }
            try {
                decodeURIComponent("%zz");
            } catch (error) {
                console.log(error);
            }
            try {
                decodeURIComponent("%C3");
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "decode_uri_component"),
        "a b\na=1&b=2\nabc-_.!~*'()123\ncafé\nargument\na=1&b=2\nawaited argument\n😀\nURI malformed\nURI malformed\nURI malformed\n"
    );
}

#[test]
fn compiles_native_number_to_precision() {
    let source = r#"
        function value(): number {
            console.log("receiver");
            return 123.456;
        }
        function digits(): number {
            console.log("digits");
            return 4;
        }
        async function delayedValue(): Promise<number> {
            console.log("awaited receiver");
            await sleep(1);
            return 123456;
        }
        async function main(): Promise<void> {
            console.log((123.456).toPrecision(4));
            console.log((0.00001234).toPrecision(2));
            console.log((123456).toPrecision(2));
            console.log((0).toPrecision(3));
            console.log((-123.456).toPrecision(4));
            console.log((42).toPrecision());
            console.log((0 / 0).toPrecision(3));
            console.log(value().toPrecision(digits()));
            console.log((await delayedValue()).toPrecision(2));
            try {
                (1).toPrecision(0);
            } catch (error) {
                console.log(error);
            }
            try {
                (1).toPrecision(101);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_to_precision"),
        "123.5\n0.000012\n1.2e+5\n0.00\n-123.5\n42\nNaN\nreceiver\ndigits\n123.5\nawaited receiver\n1.2e+5\ntoPrecision() argument must be between 1 and 100\ntoPrecision() argument must be between 1 and 100\n"
    );
}

#[test]
fn compiles_encode_uri() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "http://a.com/a b";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "😀";
        }
        async function main(): Promise<void> {
            console.log(encodeURI("http://a.com/a b?x=1&y=2#frag"));
            console.log(encodeURI(";/?:@&=+$,#"));
            console.log(encodeURI("café"));
            console.log(encodeURI(value()));
            console.log(encodeURI(await delayedValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "encode_uri"),
        "http://a.com/a%20b?x=1&y=2#frag\n;/?:@&=+$,#\ncaf%C3%A9\nargument\nhttp://a.com/a%20b\nawaited argument\n%F0%9F%98%80\n"
    );
}

#[test]
fn compiles_decode_uri() {
    let source = r#"
        function value(): string {
            console.log("argument");
            return "http://a.com/a%20b";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited argument");
            await sleep(1);
            return "%F0%9F%98%80";
        }
        async function main(): Promise<void> {
            console.log(decodeURI("http://a.com/a%20b?x=1&y=2#frag"));
            console.log(decodeURI("%3B%2F%3F%3A%40%26%3D%2B%24%2C%23"));
            console.log(decodeURI("a%20%2Fb"));
            console.log(decodeURI("caf%C3%A9"));
            console.log(decodeURI(value()));
            console.log(decodeURI(await delayedValue()));
            try {
                decodeURI("%");
            } catch (error) {
                console.log(error);
            }
            try {
                decodeURI("%zz");
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "decode_uri"),
        "http://a.com/a b?x=1&y=2#frag\n%3B%2F%3F%3A%40%26%3D%2B%24%2C%23\na %2Fb\ncafé\nargument\nhttp://a.com/a b\nawaited argument\n😀\nURI malformed\nURI malformed\n"
    );
}

#[test]
fn compiles_string_from_char_code() {
    let source = r#"
        function first(): number {
            console.log("first");
            return 72;
        }
        function second(): number {
            console.log("second");
            return 101;
        }
        async function delayedCode(): Promise<number> {
            console.log("awaited code");
            await sleep(1);
            return 65;
        }
        async function main(): Promise<void> {
            console.log(String.fromCharCode());
            console.log(String.fromCharCode(65));
            console.log(String.fromCharCode(72, 101, 108, 108, 111));
            console.log(String.fromCharCode(65.9));
            console.log(String.fromCharCode(65 + 65536));
            console.log(String.fromCharCode(first(), second()));
            console.log(String.fromCharCode(await delayedCode()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_from_char_code"),
        "\nA\nHello\nA\nA\nfirst\nsecond\nHe\nawaited code\nA\n"
    );
}

#[test]
fn compiles_string_from_code_point() {
    let source = r#"
        function first(): number {
            console.log("first");
            return 65;
        }
        function second(): number {
            console.log("second");
            return 128512;
        }
        async function delayedPoint(): Promise<number> {
            console.log("awaited point");
            await sleep(1);
            return 128512;
        }
        async function main(): Promise<void> {
            console.log(String.fromCodePoint());
            console.log(String.fromCodePoint(65));
            console.log(String.fromCodePoint(72, 101, 108, 108, 111));
            console.log(String.fromCodePoint(128512));
            console.log(String.fromCodePoint(65, 128512));
            console.log(String.fromCodePoint(first(), second()));
            console.log(String.fromCodePoint(await delayedPoint()));
            try {
                String.fromCodePoint(-1);
            } catch (error) {
                console.log(error);
            }
            try {
                String.fromCodePoint(1114112);
            } catch (error) {
                console.log(error);
            }
            try {
                String.fromCodePoint(65.5);
            } catch (error) {
                console.log(error);
            }
            try {
                String.fromCodePoint(55296);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_from_code_point"),
        "\nA\nHello\n😀\nA😀\nfirst\nsecond\nA😀\nawaited point\n😀\nInvalid code point\nInvalid code point\nInvalid code point\nInvalid code point\n"
    );
}

#[test]
fn compiles_string_locale_compare() {
    let source = r#"
        function receiver(): string {
            console.log("receiver");
            return "a";
        }
        function other(): string {
            console.log("other");
            return "b";
        }
        async function delayedOther(): Promise<string> {
            console.log("awaited other");
            await sleep(1);
            return "a";
        }
        async function main(): Promise<void> {
            console.log("a".localeCompare("b"));
            console.log("b".localeCompare("a"));
            console.log("a".localeCompare("a"));
            console.log(receiver().localeCompare(other()));
            console.log((await delayedOther()).localeCompare("a"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_locale_compare"),
        "-1\n1\n0\nreceiver\nother\n-1\nawaited other\n0\n"
    );
}

#[test]
fn compiles_string_normalize() {
    let source = r#"
        function value(): string {
            console.log("receiver");
            return "é";
        }
        function form(): string {
            console.log("form");
            return "NFC";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "é";
        }
        async function main(): Promise<void> {
            console.log("é".normalize("NFC"));
            console.log("é".normalize());
            console.log("é".normalize("NFD"));
            console.log("ﬁ".normalize("NFKD"));
            console.log(value().normalize(form()));
            console.log((await delayedValue()).normalize("NFC"));
            try {
                "abc".normalize("bogus");
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_normalize"),
        "é\né\ne\u{301}\nfi\nreceiver\nform\né\nawaited receiver\né\nThe normalization form should be one of NFC, NFD, NFKC, NFKD.\n"
    );
}

#[test]
fn compiles_tagged_template_literals() {
    let source = r#"
        function upper(strings: string[], value: number): string {
            return strings[0] + value + strings[1].toUpperCase();
        }
        function greeting(strings: string[]): string {
            return strings[0].toUpperCase();
        }
        function values(strings: string[], a: number, b: number): number {
            return strings.length + a + b;
        }
        function main(): void {
            console.log(upper`count: ${5} done`);
            console.log(greeting`hello`);
            console.log(values`${1}mid${2}`);
            console.log(String.raw`a\nb${1 + 1}c`);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tagged_template_literals"),
        "count: 5 DONE\nHELLO\n6\na\\nb2c\n"
    );
}

#[test]
fn compiles_tagged_template_evaluation_order_and_async() {
    let source = r#"
        const combine = (strings: string[], a: number, b: number): string => {
            return strings[0] + a + strings[1] + b + strings[2];
        };
        function first(): number {
            console.log("first");
            return 1;
        }
        function second(): number {
            console.log("second");
            return 2;
        }
        async function delayed(): Promise<number> {
            console.log("awaited value");
            await sleep(1);
            return 3;
        }
        async function main(): Promise<void> {
            console.log(combine`[${first()}|${second()}]`);
            console.log(combine`[${await delayed()}|${first()}]`);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "tagged_template_order_and_async"),
        "first\nsecond\n[1|2]\nawaited value\nfirst\n[3|1]\n"
    );
}

#[test]
fn compiles_native_string_split() {
    let source = r#"
        function printAll(parts: string[]): void {
            console.log(parts.length);
            for (const part of parts) {
                console.log(part);
            }
        }
        function value(): string {
            console.log("receiver");
            return "a,b,c";
        }
        function separator(): string {
            console.log("separator");
            return ",";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "x::y::z";
        }
        async function main(): Promise<void> {
            printAll("a,b,c".split(","));
            printAll("abc".split(""));
            printAll("a,b,c".split(",", 2));
            printAll("hello".split());
            printAll("a::b::c".split("::"));
            printAll("abc".split("x"));
            printAll(value().split(separator()));
            printAll((await delayedValue()).split("::"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_string_split"),
        "3\na\nb\nc\n3\na\nb\nc\n2\na\nb\n1\nhello\n3\na\nb\nc\n1\nabc\nreceiver\nseparator\n3\na\nb\nc\nawaited receiver\n3\nx\ny\nz\n"
    );
}

#[test]
fn compiles_native_string_replace() {
    let source = r#"
        function value(): string {
            console.log("receiver");
            return "abc abc";
        }
        function search(): string {
            console.log("search");
            return "a";
        }
        function replacement(): string {
            console.log("replacement");
            return "X";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited receiver");
            await sleep(1);
            return "abc abc";
        }
        async function main(): Promise<void> {
            console.log("abc abc".replace("a", "X"));
            console.log("abc abc".replaceAll("a", "X"));
            console.log("abc".replace("x", "y"));
            console.log("abc".replaceAll("x", "y"));
            console.log("abc".replace("", "X"));
            console.log("abc".replaceAll("", "X"));
            console.log(value().replace(search(), replacement()));
            console.log((await delayedValue()).replaceAll("a", "X"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_string_replace"),
        "Xbc abc\nXbc Xbc\nabc\nabc\nXabc\nXaXbXcX\nreceiver\nsearch\nreplacement\nXbc abc\nawaited receiver\nXbc Xbc\n"
    );
}

#[test]
fn compiles_regex_test() {
    let source = r#"
        function pattern(): RegExp {
            console.log("pattern");
            return /abc/;
        }
        function value(): string {
            console.log("value");
            return "xabcx";
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited value");
            await sleep(1);
            return "xabcx";
        }
        async function main(): Promise<void> {
            console.log(/abc/.test("xabcx"));
            console.log(/abc/.test("xyz"));
            console.log(new RegExp("abc").test("xabcx"));
            console.log(/ABC/i.test("xabcx"));
            console.log(/^abc$/.test("abc"));
            console.log(/^abc$/.test("xabc"));
            console.log(pattern().test(value()));
            console.log(/abc/.test(await delayedValue()));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_test"),
        "true\nfalse\ntrue\ntrue\ntrue\nfalse\npattern\nvalue\ntrue\nawaited value\ntrue\n"
    );
}

#[test]
fn compiles_string_search() {
    let source = r#"
        function value(): string {
            console.log("value");
            return "xabcx";
        }
        function pattern(): RegExp {
            console.log("pattern");
            return /abc/i;
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited value");
            await sleep(1);
            return "xabcx";
        }
        async function main(): Promise<void> {
            console.log("xabcx".search(/abc/));
            console.log("xyz".search(/abc/));
            console.log("xABCx".search(/abc/i));
            console.log(value().search(pattern()));
            console.log((await delayedValue()).search(/abc/));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_search"),
        "1\n-1\n1\nvalue\npattern\n1\nawaited value\n1\n"
    );
}

#[test]
fn compiles_date_getters_and_iso_string() {
    let source = r#"
        function fixed(): Date {
            console.log("fixed");
            return new Date(1704067200500);
        }
        async function main(): Promise<void> {
            const d: Date = new Date(1704067200500);
            console.log(d.getTime());
            console.log(d.valueOf());
            console.log(d.getFullYear());
            console.log(d.getMonth());
            console.log(d.getDate());
            console.log(d.getDay());
            console.log(d.getHours());
            console.log(d.getMinutes());
            console.log(d.getSeconds());
            console.log(d.getMilliseconds());
            console.log(d.getUTCFullYear());
            console.log(d.toISOString());
            console.log(fixed().toISOString());
            d.setTime(0);
            console.log(d.toISOString());
            console.log(Date.now() > 0);
            try {
                new Date(NaN).toISOString();
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_getters"),
        "1704067200500\n1704067200500\n2024\n0\n1\n1\n0\n0\n0\n500\n2024\n2024-01-01T00:00:00.500Z\nfixed\n2024-01-01T00:00:00.500Z\n1970-01-01T00:00:00.000Z\ntrue\nInvalid time value\n"
    );
}

#[test]
fn compiles_performance_now_as_a_monotonic_clock() {
    // Recognized as this exact `performance.now()` call-expression pattern
    // the same way `Math`/`Date`/`JSON` static calls are, backed by a
    // process-start `Instant` in thaw-runtime -- unlike `Date.now()`, it
    // never observes a wall-clock adjustment.
    let source = r#"
        async function main(): Promise<void> {
            const start = performance.now();
            await sleep(20);
            const end = performance.now();
            console.log(end > start);
            console.log(end - start >= 15);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "performance_now"),
        "true\ntrue\n"
    );
}

#[test]
fn compiles_date_setters() {
    let source = r#"
        async function main(): Promise<void> {
            const d: Date = new Date(1704067200500);
            console.log(d.setDate(15));
            console.log(d.toISOString());
            const e: Date = new Date(1704067200500);
            e.setMonth(11);
            console.log(e.toISOString());
            // Month index 12 (one past December) rolls into next January.
            e.setMonth(12);
            console.log(e.toISOString());
            const f: Date = new Date(1704067200500);
            f.setFullYear(2000, 5);
            console.log(f.toISOString());
            const g: Date = new Date(1704067200500);
            // Day 0 of the month moves to the last day of the previous one.
            g.setDate(0);
            console.log(g.toISOString());
            const h: Date = new Date(1704067200500);
            h.setHours(25);
            console.log(h.toISOString());
            const j: Date = new Date(1704067200500);
            j.setSeconds(90);
            console.log(j.toISOString());
            const k: Date = new Date(1704067200500);
            k.setMilliseconds(NaN);
            console.log(k.getTime());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_setters"),
        "1705276800500\n2024-01-15T00:00:00.500Z\n2024-12-01T00:00:00.500Z\n2025-01-01T00:00:00.500Z\n2000-06-01T00:00:00.500Z\n2023-12-31T00:00:00.500Z\n2024-01-02T01:00:00.500Z\n2024-01-01T00:01:30.500Z\nNaN\n"
    );
}

#[test]
fn compiles_date_utc_and_parse() {
    let source = r#"
        function isoText(): string {
            console.log("isoText");
            return "2024-01-01T00:00:00.500Z";
        }
        async function main(): Promise<void> {
            console.log(Date.UTC(2024, 0, 1, 0, 0, 0, 500));
            console.log(Date.UTC(2024));
            console.log(Date.UTC(70));
            console.log(Date.parse("2024-01-01T00:00:00.500Z"));
            console.log(Date.parse("2024-01-01"));
            console.log(Date.parse("not a date"));
            const d: Date = new Date("2024-01-01T00:00:00.500Z");
            console.log(d.toISOString());
            const e: Date = new Date(isoText());
            console.log(e.getTime());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_utc_parse"),
        "1704067200500\n1704067200000\n0\n1704067200500\n1704067200000\nNaN\n2024-01-01T00:00:00.500Z\nisoText\n1704067200500\n"
    );
}

#[test]
fn compiles_date_multi_arg_constructor() {
    let source = r#"
        async function main(): Promise<void> {
            const a: Date = new Date(2024, 0, 1, 0, 0, 0, 500);
            console.log(a.toISOString());
            const b: Date = new Date(2024, 0);
            console.log(b.toISOString());
            // Month index 12 (one past December) rolls into next January.
            const c: Date = new Date(2024, 12, 1);
            console.log(c.toISOString());
            // Two-digit year quirk: 70 means 1970.
            const d: Date = new Date(70, 0, 1);
            console.log(d.getTime());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_multi_arg_constructor"),
        "2024-01-01T00:00:00.500Z\n2024-01-01T00:00:00.000Z\n2025-01-01T00:00:00.000Z\n0\n"
    );
}

#[test]
fn compiles_map_string_key_numeric_value() {
    let source = r#"
        function printNumber(value: number | undefined): void {
            if (value !== undefined) {
                console.log(value);
            } else {
                console.log("undefined");
            }
        }
        async function main(): Promise<void> {
            const m: Map<string, number> = new Map<string, number>();
            console.log(m.size);
            printNumber(m.get("a"));
            m.set("a", 1).set("b", 2);
            console.log(m.size);
            printNumber(m.get("a"));
            printNumber(m.get("b"));
            console.log(m.has("a"));
            console.log(m.has("z"));
            m.set("a", 10);
            printNumber(m.get("a"));
            console.log(m.size);
            console.log(m.delete("a"));
            console.log(m.delete("a"));
            console.log(m.size);
            m.clear();
            console.log(m.size);
            console.log(m.has("b"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_string_key_numeric_value"),
        "0\nundefined\n2\n1\n2\ntrue\nfalse\n10\n2\ntrue\nfalse\n1\n0\nfalse\n"
    );
}

#[test]
fn compiles_map_numeric_key_string_value() {
    let source = r#"
        function printString(value: string | undefined): void {
            if (value !== undefined) {
                console.log(value);
            } else {
                console.log("undefined");
            }
        }
        async function main(): Promise<void> {
            const m: Map<number, string> = new Map<number, string>();
            m.set(1, "one");
            m.set(2, "two");
            printString(m.get(1));
            printString(m.get(3));
            console.log(m.size);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_numeric_key_string_value"),
        "one\nundefined\n2\n"
    );
}

#[test]
fn compiles_set_string() {
    let source = r#"
        async function main(): Promise<void> {
            const s: Set<string> = new Set<string>();
            console.log(s.size);
            console.log(s.has("a"));
            s.add("a").add("b").add("a");
            console.log(s.size);
            console.log(s.has("a"));
            console.log(s.delete("a"));
            console.log(s.delete("a"));
            console.log(s.size);
            s.clear();
            console.log(s.size);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "set_string"),
        "0\nfalse\n2\ntrue\ntrue\nfalse\n1\n0\n"
    );
}

#[test]
fn compiles_map_object_value() {
    let source = r#"
        type Point = { x: number, y: number };
        function printPoint(value: Point | undefined): void {
            if (value !== undefined) {
                console.log(value.x + value.y);
            } else {
                console.log("undefined");
            }
        }
        async function main(): Promise<void> {
            const m: Map<string, Point> = new Map<string, Point>();
            m.set("origin", { x: 0, y: 0 });
            m.set("unit", { x: 1, y: 1 });
            printPoint(m.get("unit"));
            printPoint(m.get("missing"));
        }
    "#;
    assert_eq!(compile_and_run(source, "map_object_value"), "2\nundefined\n");
}

#[test]
fn compiles_map_keys_values_entries() {
    let source = r#"
        async function main(): Promise<void> {
            const m: Map<string, number> = new Map<string, number>();
            m.set("a", 1).set("b", 2).set("c", 3);
            m.delete("b");
            for (const key of m.keys()) {
                console.log(key);
            }
            for (const value of m.values()) {
                console.log(value);
            }
            for (const entry of m.entries()) {
                console.log(entry[0] + "=" + entry[1]);
            }
            for (const [key, value] of m.entries()) {
                console.log(key + ":" + value);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_keys_values_entries"),
        "a\nc\n1\n3\na=1\nc=3\na:1\nc:3\n"
    );
}

#[test]
fn compiles_set_keys_values_entries() {
    let source = r#"
        async function main(): Promise<void> {
            const s: Set<string> = new Set<string>();
            s.add("x").add("y").add("z");
            s.delete("y");
            for (const key of s.keys()) {
                console.log(key);
            }
            for (const value of s.values()) {
                console.log(value);
            }
            for (const [a, b] of s.entries()) {
                console.log(a + b);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "set_keys_values_entries"),
        "x\nz\nx\nz\nxx\nzz\n"
    );
}

#[test]
fn compiles_map_set_for_each() {
    let source = r#"
        async function main(): Promise<void> {
            const m: Map<string, number> = new Map<string, number>();
            m.set("a", 1).set("b", 2);
            m.forEach((value: number, key: string) => {
                console.log(key + "=" + value);
            });
            const s: Set<string> = new Set<string>();
            s.add("p").add("q");
            s.forEach((value: string, key: string) => {
                console.log(value + key);
            });
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_set_for_each"),
        "a=1\nb=2\npp\nqq\n"
    );
}

#[test]
fn compiles_direct_for_of_over_map_and_set() {
    let source = r#"
        async function main(): Promise<void> {
            const m: Map<string, number> = new Map<string, number>();
            m.set("a", 1).set("b", 2);
            for (const [key, value] of m) {
                console.log(key + "=" + value);
            }
            const s: Set<string> = new Set<string>();
            s.add("x").add("y");
            for (const value of s) {
                console.log(value);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "direct_for_of_map_set"),
        "a=1\nb=2\nx\ny\n"
    );
}

#[test]
fn compiles_object_keyed_map() {
    let source = r#"
        type Point = { x: number, y: number };
        function printNumber(value: number | undefined): void {
            if (value !== undefined) {
                console.log(value);
            } else {
                console.log("undefined");
            }
        }
        async function main(): Promise<void> {
            const m: Map<Point, string> = new Map<Point, string>();
            const a: Point = { x: 0, y: 0 };
            const b: Point = { x: 0, y: 0 };
            m.set(a, "origin-a");
            m.set(b, "origin-b");
            console.log(m.size);
            console.log(m.get(a));
            console.log(m.get(b));
            console.log(m.has(a));
            m.delete(a);
            console.log(m.size);
            console.log(m.has(a));

            const counts: Map<number[], number> = new Map<number[], number>();
            const key1: number[] = [1, 2, 3];
            const key2: number[] = [1, 2, 3];
            counts.set(key1, 10);
            counts.set(key2, 20);
            console.log(counts.size);
            printNumber(counts.get(key1));
            printNumber(counts.get(key2));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "object_keyed_map"),
        "2\norigin-a\norigin-b\ntrue\n1\nfalse\n2\n10\n20\n"
    );
}

#[test]
fn compiles_map_keyed_map() {
    let source = r#"
        async function main(): Promise<void> {
            const inner: Map<string, number> = new Map<string, number>();
            const registry: Map<Map<string, number>, string> = new Map<Map<string, number>, string>();
            registry.set(inner, "the inner map");
            console.log(registry.get(inner));
            console.log(registry.has(inner));
            const other: Map<string, number> = new Map<string, number>();
            console.log(registry.has(other));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_keyed_map"),
        "the inner map\ntrue\nfalse\n"
    );
}

#[test]
fn compiles_map_and_set_constructors_with_initial_data() {
    let source = r#"
        async function main(): Promise<void> {
            const entries: [string, number][] = [["a", 1], ["b", 2], ["a", 3]];
            const m: Map<string, number> = new Map<string, number>(entries);
            console.log(m.size);
            console.log(m.get("a"));
            console.log(m.get("b"));
            for (const [key, value] of m) {
                console.log(key + "=" + value);
            }

            const values: string[] = ["x", "y", "x", "z"];
            const s: Set<string> = new Set<string>(values);
            console.log(s.size);
            for (const value of s) {
                console.log(value);
            }

            const empty: Map<string, number> = new Map<string, number>();
            console.log(empty.size);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "map_set_constructors_with_initial_data"),
        "2\n3\n2\na=3\nb=2\n3\nx\ny\nz\n0\n"
    );
}

#[test]
fn compiles_regex_match_all() {
    let source = r#"
        function printMatches(matches: string[][]): void {
            console.log(matches.length);
            for (const match of matches) {
                console.log(match.join(","));
            }
        }
        async function main(): Promise<void> {
            printMatches("a1b2c3".matchAll(/(\w)(\d)/g));
            printMatches("no digits here".matchAll(/\d+/g));
            try {
                "a1b2c3".matchAll(/\d/);
            } catch (error) {
                console.log(error);
            }
            try {
                "abc".matchAll(/(?=x)/g);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_match_all"),
        "3\na1,a,1\nb2,b,2\nc3,c,3\n0\nString.prototype.matchAll must be called with a global RegExp\nString.prototype.matchAll must be called with a global RegExp\n"
    );
}

#[test]
fn compiles_regex_flag_properties() {
    let source = r#"
        async function main(): Promise<void> {
            const re: RegExp = /abc/gims;
            console.log(re.source);
            console.log(re.flags);
            console.log(re.global);
            console.log(re.ignoreCase);
            console.log(re.multiline);
            console.log(re.dotAll);
            console.log(re.sticky);
            console.log(re.unicode);
            console.log(/abc/.global);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_flag_properties"),
        "abc\ngims\ntrue\ntrue\ntrue\ntrue\nfalse\nfalse\nfalse\n"
    );
}

#[test]
fn compiles_string_at() {
    let source = r#"
        function printChar(value: string | undefined): void {
            if (value !== undefined) {
                console.log(value);
            } else {
                console.log("undefined");
            }
        }
        async function main(): Promise<void> {
            printChar("abcde".at(0));
            printChar("abcde".at(4));
            printChar("abcde".at(-1));
            printChar("abcde".at(-5));
            printChar("abcde".at(5));
            printChar("abcde".at(-6));
            printChar("".at(0));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_at"),
        "a\ne\ne\na\nundefined\nundefined\nundefined\n"
    );
}

#[test]
fn compiles_string_char_at() {
    let source = r#"
        async function main(): Promise<void> {
            console.log("abcde".charAt(0));
            console.log("abcde".charAt(4));
            console.log("abcde".charAt(-1));
            console.log("abcde".charAt(5));
            console.log("abcde".charAt(NaN));
            console.log("abcde".charAt());
            console.log("".charAt(0));
            console.log("[" + "abcde".charAt(-1) + "]");
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_char_at"),
        "a\ne\n\n\na\na\n\n[]\n"
    );
}

#[test]
fn compiles_number_to_string_with_radix() {
    let source = r#"
        async function main(): Promise<void> {
            console.log((255).toString(16));
            console.log((8).toString(2));
            console.log((35).toString(36));
            console.log((255).toString(10));
            console.log((255).toString());
            console.log((-255).toString(16));
            console.log((0.5).toString(2));
            console.log((3.14159).toString());
            try {
                (1).toString(1);
            } catch (error) {
                console.log(error);
            }
            try {
                (1).toString(37);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_to_string_radix"),
        "ff\n1000\nz\n255\n255\n-ff\n0.1\n3.14159\ntoString() radix argument must be between 2 and 36\ntoString() radix argument must be between 2 and 36\n"
    );
}

#[test]
fn compiles_weak_map_and_weak_set() {
    let source = r#"
        type Session = { userId: number };
        function printString(value: string | undefined): void {
            if (value !== undefined) {
                console.log(value);
            } else {
                console.log("undefined");
            }
        }
        async function main(): Promise<void> {
            const cache: WeakMap<Session, string> = new WeakMap<Session, string>();
            const s1: Session = { userId: 1 };
            const s2: Session = { userId: 1 };
            cache.set(s1, "alice");
            printString(cache.get(s1));
            printString(cache.get(s2));
            console.log(cache.has(s1));
            cache.delete(s1);
            console.log(cache.has(s1));

            const seen: WeakSet<Session> = new WeakSet<Session>();
            seen.add(s1).add(s2);
            console.log(seen.has(s1));
            console.log(seen.has(s2));
            seen.delete(s2);
            console.log(seen.has(s2));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "weak_map_and_weak_set"),
        "alice\nundefined\ntrue\nfalse\ntrue\ntrue\nfalse\n"
    );
}

#[test]
fn compiles_array_keys_values_entries() {
    let source = r#"
        async function main(): Promise<void> {
            const letters: string[] = ["a", "b", "c"];
            for (const index of letters.keys()) {
                console.log(index);
            }
            for (const value of letters.values()) {
                console.log(value);
            }
            for (const [index, value] of letters.entries()) {
                console.log(index + ":" + value);
            }
            const empty: number[] = [];
            console.log(empty.keys().length);
            // Chaining onto another array method's result.
            for (const [index, value] of letters.filter((s: string) => s !== "b").entries()) {
                console.log(index + "=" + value);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "array_keys_values_entries"),
        "0\n1\n2\na\nb\nc\n0:a\n1:b\n2:c\n0\n0=a\n1=c\n"
    );
}

#[test]
fn compiles_date_to_json() {
    let source = r#"
        function printJson(value: string | null): void {
            if (value !== null) {
                console.log(value);
            } else {
                console.log("null");
            }
        }
        async function main(): Promise<void> {
            const d: Date = new Date(1704067200500);
            printJson(d.toJSON());
            const invalid: Date = new Date(NaN);
            printJson(invalid.toJSON());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_to_json"),
        "2024-01-01T00:00:00.500Z\nnull\n"
    );
}

#[test]
fn compiles_date_string_formatting() {
    let source = r#"
        async function main(): Promise<void> {
            const d: Date = new Date(1704067200500);
            console.log(d.toDateString());
            console.log(d.toTimeString());
            console.log(d.toString());
            console.log(d.toUTCString());
            const invalid: Date = new Date(NaN);
            console.log(invalid.toDateString());
            console.log(invalid.toTimeString());
            console.log(invalid.toString());
            console.log(invalid.toUTCString());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "date_string_formatting"),
        "Mon Jan 01 2024\n00:00:00 GMT+0000 (Coordinated Universal Time)\nMon Jan 01 2024 00:00:00 GMT+0000 (Coordinated Universal Time)\nMon, 01 Jan 2024 00:00:00 GMT\nInvalid Date\nInvalid Date\nInvalid Date\nInvalid Date\n"
    );
}

#[test]
fn compiles_regex_test_last_index_state() {
    let source = r#"
        async function main(): Promise<void> {
            const global = /\d/g;
            const text = "1a2";
            console.log(global.test(text));
            console.log(global.lastIndex);
            console.log(global.test(text));
            console.log(global.lastIndex);
            console.log(global.test(text));
            console.log(global.lastIndex);

            const plain = /\d/;
            plain.lastIndex = 9;
            console.log(plain.test(text));
            console.log(plain.lastIndex);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_test_last_index_state"),
        "true\n1\ntrue\n3\nfalse\n0\ntrue\n9\n"
    );
}

#[test]
fn compiles_regex_exec() {
    let source = r#"
        function printMatch(result: string[] | undefined): void {
            if (result !== undefined) {
                for (const part of result) {
                    console.log(part);
                }
            } else {
                console.log("no match");
            }
        }
        function pattern(): RegExp {
            console.log("pattern");
            return /(\d+)-(\d+)/;
        }
        function value(): string {
            console.log("value");
            return "12-34";
        }
        async function main(): Promise<void> {
            printMatch(/(\d+)-(\d+)/.exec("12-34"));
            printMatch(/(\d+)-(\d+)/.exec("abc"));
            printMatch(pattern().exec(value()));
            printMatch(/(\d+)-(\d+)/g.exec("12-34"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_exec"),
        "12-34\n12\n34\nno match\npattern\nvalue\n12-34\n12\n34\n12-34\n12\n34\n"
    );
}

#[test]
fn compiles_regex_exec_named_groups() {
    let source = r#"
        function main(): void {
            const match = /(?<word>[a-z]+)-(?<count>\d+)/.exec("item-42");
            console.log(match?.groups?.word, match?.groups?.count);
            const partial = /(?<left>a)|(?<right>b)/.exec("a");
            console.log(partial?.groups?.left, partial?.groups?.right);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_exec_named_groups"),
        "item 42\na undefined\n"
    );
}

#[test]
fn compiles_signed_bigint_arithmetic() {
    let source = r#"
        function main(): void {
            const left: bigint = 9007199254740993n;
            const right: bigint = 7n;
            console.log(String(left + right));
            console.log(typeof left, left > right);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "signed_bigint_arithmetic"),
        "9007199254741000\nbigint true\n"
    );
}

#[test]
fn compiles_symbol_keyed_properties() {
    let source = r#"
        function main(): void {
            const key: symbol = Symbol("value");
            const other: symbol = Symbol("value");
            const record = { [key]: 42 };
            console.log(record[key], String(key), key === other, typeof key);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "symbol_keyed_properties"),
        "42 Symbol(value) false symbol\n"
    );
}

#[test]
fn compiles_direct_generator_iteration() {
    let source = r#"
        function* values(): Generator<number> {
            yield 1;
            yield 2;
            yield 3;
        }
        function main(): void {
            let sum: number = 0;
            for (const value of values()) sum += value;
            console.log(sum);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "direct_generator_iteration"),
        "6\n"
    );
}

#[test]
fn compiles_generator_control_flow_and_early_return() {
    let source = r#"
        function* values(limit: number): Generator<number> {
            for (let value: number = 0; value < limit; value++) {
                if (value === 3) return;
                if (value % 2 === 0) yield value;
            }
            yield 99;
        }
        function main(): void {
            let total: number = 0;
            for (const value of values(6)) total += value;
            console.log(total);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_control_flow_and_early_return"),
        "2\n"
    );
}

#[test]
fn compiles_generator_yield_delegate() {
    let source = r#"
        function* values(): Generator<number> {
            yield 1;
            yield* [2, 3];
            yield 4;
        }
        function main(): void {
            let total: number = 0;
            for (const value of values()) total += value;
            console.log(total);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_yield_delegate"),
        "10\n"
    );
}

#[test]
fn suspends_generator_delegation_until_its_batch_is_consumed() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            yield* [1, 2];
            progress += 10;
        }
        function main(): void {
            const iterator = values();
            console.log(iterator.next().value, progress);
            console.log(iterator.next().value, progress);
            console.log(iterator.next().done, progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "suspended_generator_delegation"),
        "1 0\n2 0\ntrue 10\n"
    );
}

#[test]
fn compiles_generator_next_results() {
    let source = r#"
        function* values(): Generator<number> { yield 4; yield 7; }
        function main(): void {
            const iterator = values();
            const first = iterator.next();
            const second = iterator.next();
            const end = iterator.next();
            console.log(first.value, first.done);
            console.log(second.value, second.done);
            console.log(end.value, end.done);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_next_results"),
        "4 false\n7 false\nundefined true\n"
    );
}

#[test]
fn generator_return_stops_future_execution() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            progress += 1;
            yield 4;
            progress += 10;
            yield 7;
        }
        function main(): void {
            const iterator = values();
            console.log(iterator.next().value, progress);
            const stopped = iterator.return(99);
            console.log(stopped.value, stopped.done, progress);
            console.log(iterator.next().done, progress);
            console.log(iterator.next().done, progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_return"),
        "4 1\n99 true 1\ntrue 1\ntrue 1\n"
    );
}

#[test]
fn generator_try_catch_finally_survives_suspension() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            try {
                yield 1;
                progress += 1;
                throw new Error("boom");
            } catch (error) {
                console.log(error);
                progress += 10;
                yield 2;
            } finally {
                progress += 100;
            }
            yield 3;
        }
        function main(): void {
            const iterator = values();
            console.log(iterator.next().value, progress);
            console.log(iterator.next().value, progress);
            console.log(iterator.next().value, progress);
            console.log(iterator.next().done, progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_try_catch_finally"),
        "1 0\nboom\n2 11\n3 111\ntrue 111\n"
    );
}

#[test]
fn generator_return_runs_finally_without_resuming_the_body() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            try {
                yield 1;
                progress += 10;
                yield 2;
            } finally {
                progress += 100;
            }
            progress += 1000;
        }
        function main(): void {
            const iterator = values();
            console.log(iterator.next().value, progress);
            const stopped = iterator.return(9);
            console.log(stopped.value, stopped.done, progress);
            console.log(iterator.next().done, progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_return_finally"),
        "1 0\n9 true 100\ntrue 100\n"
    );
}

#[test]
fn generator_throw_resumes_through_catch_and_finally() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            try {
                yield 1;
            } catch (error) {
                console.log(error);
                yield 2;
            } finally {
                progress += 100;
            }
        }
        function main(): void {
            const iterator = values();
            const first = iterator.next();
            console.log(first.value, first.done);
            const caught = iterator.throw("boom");
            console.log(caught.value, caught.done, progress);
            console.log(iterator.next().done, progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_throw"),
        "1 false\nboom\n2 false 0\ntrue 100\n"
    );
}

#[test]
fn generator_next_passes_a_value_into_the_suspended_yield() {
    let source = r#"
        function* values(): Generator<number, void, number> {
            const first: number = yield 1;
            const second: number = yield first + 1;
            yield second + 1;
        }
        function main(): void {
            const iterator = values();
            console.log(iterator.next().value);
            console.log(iterator.next(10).value);
            console.log(iterator.next(20).value);
            console.log(iterator.next().done);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_next_value"),
        "1\n11\n21\ntrue\n"
    );
}

#[test]
fn generator_next_assigns_a_value_after_the_suspended_yield() {
    let source = r#"
        function* values(): Generator<number, void, number> {
            let received: number = 0;
            received = yield 2;
            yield received * 3;
        }
        function main(): void {
            const iterator = values();
            console.log(iterator.next().value);
            console.log(iterator.next(7).value);
            console.log(iterator.next().done);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_next_assignment"),
        "2\n21\ntrue\n"
    );
}

#[test]
fn defers_generator_body_until_first_consumption() {
    let source = r#"
        let started: number = 0;
        function* values(): Generator<number> {
            started += 1;
            yield 4;
        }
        function main(): void {
            const iterator = values();
            console.log(started);
            console.log(iterator.next().value);
            console.log(started);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "deferred_generator_body"),
        "0\n4\n1\n"
    );
}

#[test]
fn suspends_direct_generator_between_yields() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            progress += 1;
            yield 4;
            progress += 10;
            yield 7;
            progress += 100;
        }
        function main(): void {
            const iterator = values();
            console.log(progress);
            console.log(iterator.next().value, progress);
            console.log(iterator.next().value, progress);
            console.log(iterator.next().done, progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "suspended_direct_generator"),
        "0\n4 1\n7 11\ntrue 111\n"
    );
}

#[test]
fn preserves_generator_locals_between_yields() {
    let source = r#"
        let initialized: number = 0;
        function* values(): Generator<number> {
            let value: number = (initialized += 1);
            yield value;
            value += 2;
            yield value;
        }
        function main(): void {
            const iterator = values();
            console.log(initialized);
            console.log(iterator.next().value, initialized);
            console.log(iterator.next().value, initialized);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_local_state"),
        "0\n1 1\n3 1\n"
    );
}

#[test]
fn suspends_generator_control_flow_between_yields() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            let i: number = 0;
            while (i < 3) {
                progress += 1;
                if (i !== 1) yield i;
                i += 1;
            }
        }
        function main(): void {
            const iterator = values();
            console.log(iterator.next().value, progress);
            console.log(iterator.next().value, progress);
            console.log(iterator.next().done, progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "suspended_generator_control_flow"),
        "0 1\n2 3\ntrue 3\n"
    );
}

#[test]
fn for_of_consumes_generators_one_yield_at_a_time() {
    let source = r#"
        let progress: number = 0;
        function* values(): Generator<number> {
            progress += 1;
            yield 1;
            progress += 10;
            yield 2;
            progress += 100;
        }
        function main(): void {
            for (const value of values()) {
                console.log(value, progress);
                break;
            }
            console.log(progress);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "generator_for_of_interleaving"),
        "1 1\n1\n"
    );
}

#[test]
fn compiles_class_generator_methods() {
    let source = r#"
        class Range {
            start: number;
            constructor(start: number) { this.start = start; }
            *values(): Generator<number> {
                yield this.start;
                yield this.start + 1;
            }
        }
        function main(): void {
            let total: number = 0;
            for (const value of new Range(4).values()) total += value;
            console.log(total);
        }
    "#;
    assert_eq!(compile_and_run(source, "class_generator_method"), "9\n");
}

#[test]
fn compiles_regex_exec_last_index_state() {
    let source = r#"
        function printMatch(result: string[] | undefined): void {
            if (result !== undefined) {
                console.log(result[0]);
            } else {
                console.log("no match");
            }
        }
        async function main(): Promise<void> {
            const global = /\d+/g;
            const text = "12 34 56";
            printMatch(global.exec(text));
            printMatch(global.exec(text));
            printMatch(global.exec(text));
            printMatch(global.exec(text));
            console.log(global.lastIndex);

            const sticky = /\d+/y;
            console.log(sticky.exec("12abc") !== undefined);
            console.log(sticky.exec("12abc") !== undefined);
            sticky.lastIndex = 0;
            console.log(sticky.exec("12abc") !== undefined);

            const plain = /\d+/;
            plain.lastIndex = 5;
            plain.exec("12");
            console.log(plain.lastIndex);

            const empty = /x*/g;
            let count = 0;
            let current = empty.exec("abc");
            while (current !== undefined) {
                count++;
                if (count > 10) {
                    break;
                }
                current = empty.exec("abc");
            }
            console.log(count);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_exec_last_index_state"),
        "12\n34\n56\nno match\n0\ntrue\nfalse\ntrue\n5\n4\n"
    );
}

#[test]
fn compiles_string_match() {
    let source = r#"
        function printMatch(result: string[] | undefined): void {
            if (result !== undefined) {
                for (const part of result) {
                    console.log(part);
                }
            } else {
                console.log("no match");
            }
        }
        function value(): string {
            console.log("value");
            return "a1b2c3";
        }
        function pattern(): RegExp {
            console.log("pattern");
            return /\d/g;
        }
        async function delayedValue(): Promise<string> {
            console.log("awaited value");
            await sleep(1);
            return "abc123def";
        }
        async function main(): Promise<void> {
            printMatch("abc123def".match(/\d+/));
            printMatch("a1b2c3".match(/\d/g));
            printMatch("abc".match(/\d+/));
            printMatch(value().match(pattern()));
            printMatch((await delayedValue()).match(/\d+/));
            printMatch("12-34".match(/(\d+)-(\d+)/));
            printMatch("a=1".match(/(a)=(\d)|(b)=(\d)/));
            printMatch("12-34".match(/(\d+)-(\d+)/g));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_match"),
        "123\n1\n2\n3\nno match\nvalue\npattern\n1\n2\n3\nawaited value\n123\n12-34\n12\n34\na=1\na\n1\n\n\n12-34\n"
    );
}

#[test]
fn compiles_regex_split_and_replace() {
    let source = r#"
        function printAll(parts: string[]): void {
            for (const part of parts) {
                console.log(part);
            }
        }
        async function main(): Promise<void> {
            printAll("a1b2c3".split(/\d/));
            printAll("a b  c".split(/\s+/));
            console.log("a1b2c3".replace(/\d/, "X"));
            console.log("a1b2c3".replace(/\d/g, "X"));
            console.log("a1b2c3".replaceAll(/\d/g, "X"));
            try {
                "a1b2c3".replaceAll(/\d/, "X");
            } catch (error) {
                console.log(error);
            }
            try {
                "abc".split(/(?=x)/);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_split_and_replace"),
        "a\nb\nc\n\na\nb\nc\naXb2c3\naXbXcX\naXbXcX\nreplaceAll must be called with a global RegExp\ninvalid regular expression\n"
    );
}

#[test]
fn compiles_regex_split_with_limit() {
    let source = r#"
        function printAll(parts: string[]): void {
            for (const part of parts) {
                console.log(part);
            }
        }
        async function main(): Promise<void> {
            printAll("a,b,c,d".split(/,/, 2));
            printAll("a,b,c,d".split(/,/, 0));
            printAll("a,b,c,d".split(/,/, 100));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "regex_split_with_limit"),
        "a\nb\na\nb\nc\nd\n"
    );
}

#[test]
fn compiles_well_formed_native_strings() {
    let source = r#"
        function text(): string {
            console.log("receiver");
            return "A😀é";
        }
        async function delayed(): Promise<string> {
            console.log("awaited");
            await sleep(1);
            return "later😀";
        }
        async function main(): Promise<void> {
            console.log("plain".isWellFormed());
            console.log("😀".toWellFormed());
            console.log(text().isWellFormed());
            console.log(text().toWellFormed());
            console.log((await delayed()).isWellFormed());
            console.log((await delayed()).toWellFormed());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "well_formed_strings"),
        "true\n😀\nreceiver\ntrue\nreceiver\nA😀é\nawaited\ntrue\nawaited\nlater😀\n"
    );
}

#[test]
fn compiles_number_predicates_and_aggregate_numeric_conversion() {
    let source = r#"
        function text(): string {
            console.log("strict-predicate-evaluated");
            return "bad";
        }
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-predicate");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Number.isNaN(0 / 0));
            console.log(Number.isNaN(text()));
            console.log(Number.isFinite(42));
            console.log(Number.isFinite(Number("Infinity")));
            console.log(isNaN("bad"));
            console.log(isNaN("12"));
            console.log(isFinite("12"));
            console.log(isFinite("Infinity"));
            const empty: number[] = [];
            const one: number[] = [7];
            const many: number[] = [1, 2];
            console.log(Number(empty));
            console.log(Number(one));
            console.log(isNaN(many));
            console.log(isNaN({ value: 1 }));
            console.log(isNaN(await delayed("9")));
            console.log(Number(...["12"]));
            console.log(String(...[true]));
            console.log(Boolean(...[0]));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "number_predicates"),
        "true\nstrict-predicate-evaluated\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\n0\n7\ntrue\ntrue\nawaited-predicate\nfalse\n12\ntrue\nfalse\n"
    );
}
