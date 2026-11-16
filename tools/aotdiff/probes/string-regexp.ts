// Differential probes (family: string-regexp). Each p() line is one probe; see tools/aotdiff/README.md.
class P { x: number = 1; get dbl(): number { return this.x * 2; } set dbl(v: number) { this.x = v / 2; } }
class Q extends P { y: string = "q"; toString(): string { return "Q(" + this.y + ")"; } }
function id(v: any): any { return v; }
function p(name: string, f: () => any): void {
  try { const r = f(); console.log(name, "=>", typeof r === "string" ? JSON.stringify(r) : r); }
  catch (e: any) { console.log(name, "THROW", e && e.constructor && e.constructor.name, String(e && e.message).slice(0, 80)); }
}
function main(): void {
  p("strUni", () => id("a😀b").length);
  p("strIter", () => [...id("a😀b")].length);
  p("strCodePt", () => id("😀").codePointAt(0));
  p("strCharAt", () => id("😀").charCodeAt(1));
  p("strNorm", () => id("é").normalize("NFD").length);
  p("strUpper", () => id("ß").toUpperCase());
  p("strLoc", () => id("İ").toLowerCase().length);
  p("strPad", () => id("ab").padStart(5, "xy"));
  p("strSplit", () => id("a,b,,c").split(","));
  p("strSplitRe", () => id("a1b22c").split(/(\d)/));
  p("strReplace", () => id("aaa").replace("a", "$&$&"));
  p("strReplaceAll", () => id("a.b.c").replaceAll(".", "-"));
  p("strLone", () => JSON.stringify(id("\ud800")));
  p("strWell", () => id("a\ud800").isWellFormed());
  p("strAt", () => id("abc").at(-1));
  p("strCmp", () => id("a") < id("B"));
  p("strLocCmp", () => id("a").localeCompare(id("B")));
  p("re1", () => "2020-01-02".match(/(?<y>\d+)-(?<m>\d+)/)?.groups);
  p("re2", () => [..."a1b2".matchAll(/\d/g)].map(m => m.index));
  p("re3", () => { const r = /a/g; r.test("aa"); return r.lastIndex; });
  p("re4", () => "aBc".replace(/b/i, m => m.toUpperCase() + "!"));
  p("re5", () => /(?<=\$)\d+/.exec("cost $42")?.[0]);
  p("re6", () => /\p{L}+/u.exec("héllo wörld")?.[0]);
  p("re7", () => String(/a\/b/gi));
  p("re8", () => new RegExp(id("[")).source);
  p("re9", () => /a.c/s.test("a\nc"));
  p("re10", () => "x".replace(/(y)?x/, "[$1]"));
  p("reSticky", () => { const r = /a/y; r.lastIndex = 1; return r.test("ba"); });
  p("tpl", () => `${1}${"a"}${[1, 2]}${{}}${null}${undefined}`);
  p("tagged", () => { const t = (s: TemplateStringsArray, ...v: any[]) => s.raw.join("|") + v.join(","); return t`a\n${1}b${2}`; });
  p("urlSP", () => new URLSearchParams("a=1&b=%20x&a=2").getAll("a"));
  p("url", () => new URL("../x?y=1#z", "http://h.com/a/b/c").href);
  p("textEnc", () => Array.from(new TextEncoder().encode("é😀")));
  p("textDec", () => new TextDecoder().decode(new Uint8Array([0xff, 0x41])));
  p("btoa", () => btoa("hi") + atob("aGk="));
  p("encodeURI", () => encodeURIComponent("a b&é😀") + encodeURI("a b/é"));
}
