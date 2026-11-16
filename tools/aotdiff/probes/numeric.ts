// Differential probes (family: numeric). Each p() line is one probe; see tools/aotdiff/README.md.
class P { x: number = 1; get dbl(): number { return this.x * 2; } set dbl(v: number) { this.x = v / 2; } }
class Q extends P { y: string = "q"; toString(): string { return "Q(" + this.y + ")"; } }
function id(v: any): any { return v; }
function p(name: string, f: () => any): void {
  try { const r = f(); console.log(name, "=>", typeof r === "string" ? JSON.stringify(r) : r); }
  catch (e: any) { console.log(name, "THROW", e && e.constructor && e.constructor.name, String(e && e.message).slice(0, 80)); }
}
function main(): void {
  p("num1", () => 0.1 + 0.2);
  p("negzero", () => Object.is(-0, id(-0)));
  p("negzero2", () => 1 / id(-0));
  p("negzeroStr", () => String(id(-0)));
  p("negzeroJson", () => JSON.stringify([id(-0)]));
  p("nan", () => [NaN].includes(id(NaN)));
  p("nanIdx", () => [NaN].indexOf(id(NaN)));
  p("maxSafe", () => id(9007199254740993) + 0);
  p("int32ovf", () => (id(2147483647) + 1) | 0);
  p("uint32", () => id(-1) >>> 0);
  p("shift", () => id(1) << id(33));
  p("mulOvf", () => id(2 ** 31) * id(2 ** 31));
  p("imul", () => Math.imul(id(0xffffffff), 5));
  p("mod", () => id(-7) % 3);
  p("modf", () => id(5.5) % -2);
  p("pow", () => id(2) ** id(0.5));
  p("big1", () => id(2n) ** 100n);
  p("bigNeg", () => (-7n) / 2n);
  p("bigMod", () => (-7n) % 3n);
  p("bigMix", () => { return (id(1n) as any) + (id(1) as any); });
  p("bigTypeof", () => typeof id(1n));
  p("bigAsUint", () => BigInt.asUintN(8, id(-1n)));
  p("bigJson", () => JSON.stringify({ a: 1n } as any));
  p("bigCmp", () => 1n < 2 && 2n == 2);
  p("parseInt", () => parseInt(id("0x1f")));
  p("parseFloat", () => parseFloat(id("1e3x")));
  p("numStr", () => (id(1e21)).toString() + "|" + (id(1e-7)).toString());
  p("toFixed", () => (id(1.005)).toFixed(2));
  p("toPrec", () => (id(123456)).toPrecision(2));
  p("radix", () => (id(255)).toString(16) + (id(-255.5)).toString(2));
  p("floatFmt", () => [1e21, 1e-7, 123456789012345680000, 0.000001, 1.7976931348623157e308, 5e-324, -1e-7].map(String));
  p("mathRound", () => [Math.round(-0.5), Math.round(2.5), Math.round(-2.5), Math.sign(-0), Math.max(), Math.min(), Math.hypot(3, 4), Math.trunc(-0.9), Math.cbrt(27), Math.fround(5.5), Math.clz32(1)]);
  p("numStatic", () => [Number.MAX_SAFE_INTEGER, Number.EPSILON > 0, Number.isInteger(5.0), Number("  "), Number("0b101"), Number(null), Number(undefined), Number([]), Number("1e3")]);
  p("intl", () => (1234567.891).toLocaleString("en-US"));
}
