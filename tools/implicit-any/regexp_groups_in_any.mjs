// exec on a RegExp held in an untyped record keeps groups / index / input; groups spread into a field.
const routes = [];
const route = (method, pattern) =>
  routes.push({ method, re: new RegExp('^' + pattern.replace(/:(\w+)/g, '(?<$1>[^/]+)') + '/?$') });
route('GET', '/users/:id');
route('GET', '/users/:id/posts/:post');
const make = () => ({ params: {}, hits: 0 });
const handlers = [(ctx) => { ctx.hits += 1; return Object.keys(ctx.params).join(','); }];
function dispatch(method, path) {
  const ctx = make();
  for (const r of routes) {
    const m = r.method === method && r.re.exec(path);
    if (m) {
      ctx.params = { ...m.groups };
      return [handlers[0](ctx), JSON.stringify(ctx.params), m.index, m.input, m[0]];
    }
  }
  return ['none'];
}
console.log(dispatch('GET', '/users/42'));
console.log(dispatch('GET', '/users/7/posts/abc'));
console.log(dispatch('POST', '/users/1'));
