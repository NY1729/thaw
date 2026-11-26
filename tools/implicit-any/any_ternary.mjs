// A ternary joining an object literal with an `any` parameter is `any`.

const make = (opts) => (typeof opts === 'string' ? { name: opts } : opts);
console.log(make('n').name, make({ name: 'm' }).name);
function norm(input) {
  const o = typeof input === 'string' ? { path: input } : input;
  return o.path + '|' + (o.method || 'GET');
}
console.log(norm('/a'), norm({ path: '/b', method: 'POST' }));
