import Handlebars from "handlebars";
// Handlebars is CommonJS (module.exports = object): default import, not a named one.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  Handlebars.registerHelper("shout", (s: string) => String(s).toUpperCase() + "!");
  const tpl = Handlebars.compile("Hello {{name}} {{shout city}} {{#each items}}[{{@index}}:{{this}}]{{/each}} {{#if ok}}yes{{else}}no{{/if}} {{{raw}}}");
  line("render", tpl({ name: "Ada", city: "tokyo", items: ["a", "b"], ok: true, raw: "<b>&</b>" }));
  line("missing", Handlebars.compile("x{{nope.deep}}y")({}));
  line("escape", Handlebars.escapeExpression("<a href='x'>&</a>"));
  // WORKLOAD: N=3000 renders of one precompiled template (estimate).
  const N = 3000;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    acc = (acc * 31 + tpl({ name: "u" + i, city: "c", items: [i, i + 1], ok: i % 2 === 0, raw: "<" + i + ">" }).length) | 0;
  }
  line("checksum", acc);
}
