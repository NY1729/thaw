import { marked } from "marked";
// fixed markdown in, HTML out (sync). N = 3k small documents (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const DOC = "# Title\n\nSome *emphasis* and **strong** text.\n\n- one\n- two\n\n[link](https://example.com/x)\n\n`code`\n";
async function main(): Promise<void> {
  const N = 300;
  line("doc", marked.parse(DOC, { async: false }) as string);
  line("inline", marked.parseInline("a **b** `c`", { async: false }) as string);
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const html = marked.parse("# H" + i + "\n\n- item " + i + "\n- *x*\n", { async: false }) as string;
    acc = (acc * 31 + html.length) | 0;
  }
  line("checksum", acc);
}
