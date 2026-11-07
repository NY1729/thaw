import ejs from "ejs";
// ejs is CommonJS: default import (ejs.render / ejs.compile). Templates are inline strings, no file includes.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const TPL = '<h1><%= title %></h1><% items.forEach(function (it, i) { %><li data-i="<%= i %>"><%- it.name %></li><% }) %><%# a comment %>';
async function main(): Promise<void> {
  line("render", ejs.render(TPL, { title: "A & B", items: [{ name: "x" }, { name: "<y>" }] }));
  line("escaped", ejs.render("<%= s %>|<%- s %>", { s: "<&>'\"" }));
  line("empty", ejs.render("[<%= missing || 'dflt' %>]", { missing: null }));
  // WORKLOAD: N=2000 renders of one compiled template (estimate).
  const fn = ejs.compile("<%= n %>:<%= s.length %>|<% for (var k = 0; k < n % 3; k++) { %>*<% } %>");
  const N = 2000;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    acc = (acc * 31 + fn({ n: i, s: "ab" + i }).length) | 0;
  }
  line("checksum", acc);
}
