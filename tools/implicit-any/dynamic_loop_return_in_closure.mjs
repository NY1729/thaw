// `return` out of a for-of over an untyped route table, inside a closure with no annotated result.
const routes = [];
routes.push({ method: 'GET', handler: (c) => ({ user: c.id }) });
routes.push({ method: 'POST', handler: (c) => c.id * 2 });
const mw = [(c, next) => next()];
function dispatch(method) {
  const ctx = { id: 21, trace: [] };
  mw[0](ctx, () => 0);
  const final = () => {
    for (const r of routes) {
      if (r.method === method) return { status: 200, body: r.handler(ctx) };
    }
    return { status: 404, body: null };
  };
  return final();
}
console.log(JSON.stringify([dispatch('GET'), dispatch('POST'), dispatch('PUT')]));
