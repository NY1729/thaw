// Differential probes (family: json-date). Each p() line is one probe; see tools/aotdiff/README.md.
class P { x: number = 1; get dbl(): number { return this.x * 2; } set dbl(v: number) { this.x = v / 2; } }
class Q extends P { y: string = "q"; toString(): string { return "Q(" + this.y + ")"; } }
function id(v: any): any { return v; }
function p(name: string, f: () => any): void {
  try { const r = f(); console.log(name, "=>", typeof r === "string" ? JSON.stringify(r) : r); }
  catch (e: any) { console.log(name, "THROW", e && e.constructor && e.constructor.name, String(e && e.message).slice(0, 80)); }
}
function main(): void {
  p("jsonRound", () => JSON.stringify(JSON.parse(id('{"a":[1,2,{"b":null}],"c":"d"}'))));
  p("jsonUndef", () => JSON.stringify({ a: undefined, b: () => 1, c: Symbol("x"), d: NaN }));
  p("jsonDate", () => JSON.stringify({ d: new Date(0) }));
  p("jsonIndent", () => JSON.stringify({ a: [1, { b: 2 }] }, null, 2));
  p("jsonReplacer", () => JSON.stringify({ a: 1, b: 2 }, ["a"]));
  p("jsonReviver", () => JSON.parse('{"a":1,"b":[2]}', (k, v) => typeof v === "number" ? v * 2 : v));
  p("jsonBad", () => JSON.parse(id("{bad")));
  p("jsonBad2", () => JSON.parse(id("")));
  p("jsonProto", () => Object.keys(JSON.parse('{"__proto__":1}')));
  p("jsonBigNum", () => JSON.parse("12345678901234567890"));
  p("jsonCyc", () => { const o: any = {}; o.o = o; return JSON.stringify(o); });
  p("jsonToJSON", () => JSON.stringify({ toJSON() { return 5; } }));
  p("date1", () => new Date(Date.UTC(2020, 1, 29, 12)).toISOString());
  p("date2", () => new Date("2020-02-30").getTime());
  p("date3", () => new Date(8.64e15 + 1).getTime());
  p("date4", () => new Date(2020, 0, 31).getMonth());
  p("date5", () => String(new Date(NaN)));
  p("date6", () => Date.UTC(99, 0));
  p("date7", () => new Date(0).toUTCString());
  p("dateInvalid", () => new Date(NaN).toISOString());
  p("dateLoc", () => new Date(0).toLocaleDateString("en-US", { timeZone: "UTC" }));
  p("structClone", () => { const o: any = { d: new Date(0), m: new Map([[1, 2]]), a: [1] }; const c = structuredClone(o); return [c.d instanceof Date, c.m.get(1), c !== o]; });
}
