const req = new Request('http://localhost/p?q=1', { method: 'post', body: 'abc', headers: { a: 'b' } });
console.log(req.method, req.url, req.headers.get('content-type'));
const fd = new FormData();
fd.append('k', 'v');
fd.append('k', 'w');
console.log(fd.get('k'), fd.getAll('k').join(), fd.has('z'));
(async () => {
  console.log('clone', await req.clone().text());
  console.log('text', await req.text());
  console.log([...await new Response(fd).formData()].join('|'));
})();
