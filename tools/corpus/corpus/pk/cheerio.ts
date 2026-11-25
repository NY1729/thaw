import { load } from "cheerio";
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const html = '<ul id="list"><li class="item" data-n="1">One</li><li class="item" data-n="2">Two &amp; more</li><li class="skip">Skip</li></ul><a href="/x?y=1">link</a>';
  const $ = load(html);
  line("count", $("li.item").length);
  line("texts", $("li.item").toArray().map((el) => $(el).text()));
  line("attr", $("a").attr("href"));
  line("dataN", $("li.item").last().attr("data-n"));
  line("inner", $("#list").children().first().html());
  // WORKLOAD: N=271 small documents parsed and queried (estimate).
  const N = 271;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const d = load('<div id="x' + i + '"><span class="v">' + i + "</span></div>");
    acc = (acc * 31 + Number(d("span.v").text()) + String(d("div").attr("id")).length) | 0;
  }
  line("checksum", acc);
}
