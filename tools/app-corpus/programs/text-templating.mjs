// Text templating: {{var}} / {{#each}} / {{#if}} renderer, escaping and padding helpers.
const escapeHtml = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

function lookup(ctx, path) {
  return path.split('.').reduce((o, k) => (o == null ? undefined : o[k]), ctx);
}

function render(tpl, ctx) {
  tpl = tpl.replace(/\{\{#each (\w+)\}\}([\s\S]*?)\{\{\/each\}\}/g, (_, key, body) =>
    (lookup(ctx, key) ?? []).map((item, i) => render(body, { ...ctx, this: item, '@index': i })).join(''));
  tpl = tpl.replace(/\{\{#if ([\w.]+)\}\}([\s\S]*?)(?:\{\{else\}\}([\s\S]*?))?\{\{\/if\}\}/g, (_, key, a, b = '') =>
    lookup(ctx, key) ? render(a, ctx) : render(b, ctx));
  tpl = tpl.replace(/\{\{\{([\w.@]+)\}\}\}/g, (_, k) => String(lookup(ctx, k) ?? ''));
  return tpl.replace(/\{\{([\w.@]+)\}\}/g, (_, k) => escapeHtml(lookup(ctx, k) ?? ''));
}

const tpl = `<h1>{{title}}</h1>
{{#if user.admin}}admin: {{user.name}}{{else}}guest{{/if}}
<ul>{{#each items}}
  <li>{{@index}}: {{this}}</li>{{/each}}
</ul>
raw: {{{html}}} escaped: {{html}}
`;
console.log(render(tpl, { title: 'Report <1>', user: { admin: true, name: 'Ann' }, items: ['a&b', 'c', "d'e"], html: '<b>x</b>' }));
console.log(render('{{#if user.admin}}A{{else}}B{{/if}} {{missing.deep}}!', { user: { admin: false } }));
console.log('name'.padEnd(8, '.') + '|' + '42'.padStart(6, ' ') + '|' + `${1234.5.toFixed(1)}`.padStart(8, '*'));
