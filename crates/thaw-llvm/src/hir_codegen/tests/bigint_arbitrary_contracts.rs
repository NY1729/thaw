// Arbitrary-precision native bigint. Expected output recorded from node 22.

#[test]
fn bigint_contract_values_beyond_64_bits_do_not_wrap() {
    let source = r##"function main(): void {
  console.log(2n ** 64n, 2n ** 100n, 10n ** 20n, 1n << 70n);
  let x = 1n; let i = 1n;
  while (i <= 25n) { x *= i; i++; }
  console.log(x);
  console.log(BigInt.asUintN(64, -1n), BigInt.asIntN(64, 18446744073709551615n), BigInt.asIntN(8, 255n));
  console.log(BigInt(1e21), BigInt("123456789012345678901234567890") + 1n);
  console.log(9223372036854775807n * 9223372036854775807n);
}
"##;
    assert_eq!(
        compile_and_run(source, "bigint_contract_no_wrap"),
        "18446744073709551616n 1267650600228229401496703205376n 100000000000000000000n 1180591620717411303424n\n15511210043330985984000000n\n18446744073709551615n -1n -1n\n1000000000000000000000n 123456789012345678901234567891n\n85070591730234615847396907784232501249n\n"
    );
}

#[test]
fn bigint_contract_division_shift_compare_and_conversion() {
    let source = r##"function main(): void {
  const big = 2n ** 80n;
  console.log(big / 3n, big % 1000n, -big / 7n, -7n / 2n, -7n % 2n, big >> 70n, -big >> 79n);
  console.log(big > big - 1n, big === 2n ** 80n, big !== big + 1n, big < 5n, -big < 5n);
  console.log(big.toString(), big.toString(16), (-big).toString(36), String(big), `${big}`, Number(big));
  console.log(~big, big & 255n, (big + 5n) | 2n, big ^ big, typeof big);
  console.log(1n < 1.5, 2n == 2, 3n > 2.5);
}
"##;
    assert_eq!(
        compile_and_run(source, "bigint_contract_ops"),
        "402975273204876391568725n 176n -172703688516375596386596n -3n -1n 1024n -2n\ntrue true true false true\n1208925819614629174706176 100000000000000000000 -5gv2rma270x9hhj4 1208925819614629174706176 1208925819614629174706176 1.2089258196146292e+24\n-1208925819614629174706177n 0n 1208925819614629174706183n 0n bigint\ntrue true true\n"
    );
}

#[test]
fn bigint_contract_errors_json_and_catch() {
    let source = r##"function main(): void {
  try { console.log(1n / 0n); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  try { console.log(2n ** -1n); } catch (e) { console.log((e as Error).name); }
  try { console.log(BigInt("12x")); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  try { throw 2n ** 70n; } catch (e) { console.log(e); }
  console.log(BigInt("0xff"), BigInt("  42 "), BigInt(""), [2n ** 65n], { v: 2n ** 65n });
}
"##;
    assert_eq!(
        compile_and_run(source, "bigint_contract_errors"),
        "RangeError Division by zero\nRangeError\nSyntaxError Cannot convert 12x to a BigInt\n1180591620717411303424n\n255n 42n 0n [ 36893488147419103232n ] { v: 36893488147419103232n }\n"
    );
}
