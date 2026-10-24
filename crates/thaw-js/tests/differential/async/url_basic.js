const inputs = [
  ['https://user:pa%20ss@Example.COM:8080/a/./b/../c d?x=1&y=2#frag ment'],
  ['http://example.com'], ['http://example.com:80/'], ['https://example.com:443'], ['http://EXAMPLE.com:81/%7Efoo/bar%2e/..'],
  ['/path/only', 'http://base.org/dir/file?q#h'], ['../up?x', 'http://base.org/a/b/c'], ['?only=query', 'http://base.org/a/b?old#h'], ['#frag', 'http://base.org/a/b?q'],
  ['//other.org/x', 'https://base.org/y'], ['rel/path', 'http://base.org/dir/'], ['', 'http://base.org/a?b#c'], ['./x/../y', 'http://b.org/p/q/r'],
  ['mailto:someone@example.com?subject=Hi'], ['data:text/plain;base64,aGk='], ['javascript:alert(1)'], ['urn:isbn:0451450523'],
  ['ftp://ftp.example.org/pub/file.txt'], ['ws://example.com/chat'], ['wss://example.com:8443/'], ['file:///usr/local/bin/x'], ['file://localhost/etc/hosts'],
  ['http://192.168.0.1:3000/'], ['http://0x7f.1/'], ['http://[::1]:8080/p'], ['http://[2001:DB8::1]/'], ['http://münchen.de/ü?ä=ö#ß'], ['http://xn--mnchen-3ya.de/'],
  ['http://a.b/path with spaces/ü'], ['http://a.b/?q=a b&r=ü#h h'], ['custom://host/path?q#f'], ['custom:/just/path'], ['custom:opaque path?q'],
  ['http://a.b\\c\\d'], ['HTTP://A.B/'], ['  http://trim.me/  '], ['http://a.b/\t\nx'], ['http://user@host/'], ['http://:pw@host/'],
  ['not a url'], ['http://'], ['http://a b/'], ['http://a.b:99999/'], ['http://a.b:xx/'], ['//no.scheme'], ['/rel', 'notabase'],
];
for (const [i, b] of inputs) {
  try {
    const u = new URL(i, b);
    console.log(JSON.stringify(i), u.href, '|', u.origin, u.protocol, u.username, u.password, u.host, u.hostname, u.port, u.pathname, u.search, u.hash);
  } catch (e) { console.log(JSON.stringify(i), e.name, e.code, e.message); }
}
const u = new URL('https://example.com/p?a=1&b=2&a=3#h');
console.log(u.searchParams.get('a'), u.searchParams.getAll('a'), u.searchParams.has('b'), u.searchParams.has('zz'), [...u.searchParams.keys()].join(), u.searchParams.toString(), u.searchParams.size);
u.searchParams.append('c', 'x y&z'); u.searchParams.set('a', 'new'); u.searchParams.delete('b');
console.log(u.href, u.search);
u.search = '?q=1'; console.log(u.searchParams.get('q'), u.searchParams.get('a'), u.href);
u.hash = 'x'; u.pathname = '/new path'; u.port = '8080'; u.hostname = 'other.org'; u.protocol = 'http'; u.username = 'me'; u.password = 'pw';
console.log(u.href, u.toString() === u.href, JSON.stringify({ u }), String(u));
u.port = '80'; console.log(u.href); u.href = 'https://reset.org/'; console.log(u.href, u.searchParams.toString() === '');
const sp = new URLSearchParams('a=1&b=%20x+y&a=2&&c');
console.log(sp.toString(), [...sp].join('|'), sp.get('b'), sp.get('c'), new URLSearchParams({ x: '1', y: 'ü' }).toString(), new URLSearchParams([['k', 'v'], ['k', 'w']]).toString(), new URLSearchParams('?z=9').get('z'));
sp.sort(); console.log(sp.toString()); sp.forEach((v, k) => console.log(k, v));
console.log(URL.canParse('http://x.y'), URL.canParse('nope'), URL.canParse('/p', 'http://b'), new URL('http://x.y') instanceof URL, typeof URLSearchParams, new URL('HTTP://X.Y/%41').pathname);
console.log(encodeURIComponent('a b&ü'), new URL('http://x.y/?a=b%20c').searchParams.get('a'), new URLSearchParams('a=%zz').get('a'));
