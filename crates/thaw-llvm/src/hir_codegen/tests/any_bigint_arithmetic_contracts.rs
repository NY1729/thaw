// Arithmetic and comparisons on `any` values holding BigInts stay exact (outputs recorded from node 22).

#[test]
fn any_bigint_arithmetic_is_exact() {
    let source = r##"function t(label: string, f: () => any): void {
  try { console.log(label, f()); } catch (e: any) { console.log(label, 'ERR', e.name, e.message); }
}
function main(): void {
  const a: any = BigInt("9007199254740993");
  const b: any = BigInt("3");
  const n: any = 10;
  const m: any = 4;
  const s: any = 'x';
  t('a+b', () => a + b);
  t('a-b', () => a - b);
  t('a*b', () => a * b);
  t('a/b', () => a / b);
  t('a%b', () => a % b);
  t('a**b', () => a ** b);
  t('a>b', () => a > b);
  t('a<b', () => a < b);
  t('a>=a', () => a >= a);
  t('a<=b', () => a <= b);
  t('a<<b', () => a << b);
  t('a>>b', () => a >> b);
  t('a&b', () => a & b);
  t('a|b', () => a | b);
  t('a^b', () => a ^ b);
  t('a-1n', () => a - 1n);
  t('1n-a', () => 1n - a);
  t('a*2n', () => a * 2n);
  t('a<1n', () => a < 1n);
  t('a-n', () => a - n);
  t('n-a', () => n - a);
  t('n-m', () => n - m);
  t('n*m', () => n * m);
  t('n/m', () => n / m);
  t('n<m', () => n < m);
  t('n**m', () => n ** m);
  t('n<<m', () => n << m);
  t('-a', () => -a);
  t('a+lit', () => a + 9007199254740993n);
  t('lit+a', () => 9007199254740993n + a);
  t('a+0n', () => a + 0n);
  t('str+1n', () => s + 1n);
  t('n+1n', () => n + 1n);
  t('-n', () => -n);
  t('-0', () => -(n - n));
}
"##;
    assert_eq!(
        compile_and_run(source, "any_bigint_arithmetic_is_exact"),
        "a+b 9007199254740996n\na-b 9007199254740990n\na*b 27021597764222979n\na/b 3002399751580331n\na%b 0n\na**b 730750818665451702490757660178213618792745926657n\na>b true\na<b false\na>=a true\na<=b false\na<<b 72057594037927944n\na>>b 1125899906842624n\na&b 1n\na|b 9007199254740995n\na^b 9007199254740994n\na-1n 9007199254740992n\n1n-a -9007199254740992n\na*2n 18014398509481986n\na<1n false\na-n ERR TypeError Cannot mix BigInt and other types, use explicit conversions\nn-a ERR TypeError Cannot mix BigInt and other types, use explicit conversions\nn-m 6\nn*m 40\nn/m 2.5\nn<m false\nn**m 10000\nn<<m 160\n-a -9007199254740993n\na+lit 18014398509481986n\nlit+a 18014398509481986n\na+0n 9007199254740993n\nstr+1n x1\nn+1n ERR TypeError Cannot mix BigInt and other types, use explicit conversions\n-n -10\n-0 -0\n"
    );
}

#[test]
fn any_bigint_arithmetic_on_hrtime_bigint_is_exact() {
    let source = r##"function main(): void {
    const h: any = process.hrtime.bigint();
    console.log(typeof (h - h), h - h, h > 0n, (h - h + 9007199254740993n) - 1n);
}
"##;
    assert_eq!(
        compile_and_run(source, "any_bigint_arithmetic_on_hrtime_bigint_is_exact"),
        "bigint 0n true 9007199254740992n\n"
    );
}
