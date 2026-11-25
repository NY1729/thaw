// URL / URLSearchParams based router with params, query parsing and middleware chain.
const routes = [];
const route = (method, pattern, handler) =>
  routes.push({ method, re: new RegExp('^' + pattern.replace(/:(\w+)/g, '(?<$1>[^/]+)') + '/?$'), handler });

route('GET', '/users/:id', ({ params }) => ({ user: Number(params.id) }));
route('GET', '/users/:id/posts/:post', ({ params, query }) => ({ ...params, page: Number(query.get('page') ?? 1) }));
route('POST', '/echo', ({ query }) => Object.fromEntries(query));

const middleware = [
  (ctx, next) => { ctx.trace.push('a'); const r = next(); ctx.trace.push('a-after'); return r; },
  (ctx, next) => { ctx.trace.push('b'); return next(); },
];

function dispatch(method, href) {
  const url = new URL(href, 'http://example.test');
  const ctx = { method, url, query: url.searchParams, params: {}, trace: [] };
  const final = () => {
    for (const r of routes) {
      const m = r.method === method && r.re.exec(url.pathname);
      if (m) {
        ctx.params = { ...m.groups };
        return { status: 200, body: r.handler(ctx) };
      }
    }
    return { status: 404, body: null };
  };
  const run = (i) => (i < middleware.length ? middleware[i](ctx, () => run(i + 1)) : final());
  const res = run(0);
  return { ...res, trace: ctx.trace.join('>') };
}

for (const [m, h] of [['GET', '/users/42'], ['GET', '/users/7/posts/abc?page=3'], ['POST', '/echo?x=1&y=two&x=3'], ['GET', '/nope'], ['DELETE', '/users/1']]) {
  console.log(m, h, JSON.stringify(dispatch(m, h)));
}
const u = new URL('https://user:pw@Example.COM:8443/a/b/../c?q=1&q=2#frag');
console.log(u.href, u.host, u.pathname, u.searchParams.getAll('q'), u.hash, u.origin);
u.searchParams.append('z', 'a b&c');
u.pathname = '/new path';
console.log(u.toString(), new URLSearchParams({ a: '1', b: 'x y' }).toString());
console.log(new URL('../up', 'http://h/a/b/c').href, URL.canParse('nope'));
