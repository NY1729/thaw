const h = new Headers({ 'Content-Type': 'text/plain', 'X-A': '1' });
h.append('x-a', '2');
console.log(h.get('x-a'), h.has('content-type'), [...h.keys()].join());
const r = new Response('hello', { status: 201, headers: { 'x-k': 'v' } });
console.log(r.status, r.ok, r.headers.get('content-type'), r.headers.get('x-k'));
r.text().then(t => console.log(t, r.bodyUsed));
const j = Response.json({ a: [1, 2] });
j.json().then(o => console.log(JSON.stringify(o), j.headers.get('content-type')));
