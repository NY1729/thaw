// pkg: validator
const v = require('validator');
const cases = {
  isEmail: ['a@b.co', 'bad@', 'x.y+z@sub.example.org', 'üser@exämple.com', '"quoted"@x.com', 'a@b'],
  isURL: ['http://example.com', 'https://a.b/c?d=e#f', 'ftp://x.y', 'not a url', 'example.com', 'http://localhost:3000/path', 'http://1.2.3.4'],
  isInt: ['12', '-3', '1.5', 'abc', '0123', ''],
  isFloat: ['1.5', '-.5', 'x', '1e5'],
  isAlpha: ['abc', 'ab1', ''],
  isAlphanumeric: ['ab1', 'a b'],
  isNumeric: ['123', '-1.5', 'x'],
  isUUID: ['a8098c1a-f86e-11da-bd1a-00112444be1e', 'nope'],
  isIP: ['127.0.0.1', '::1', '999.1.1.1', 'fe80::1'],
  isHexColor: ['#fff', '#ggg', 'abcdef'],
  isJSON: ['{"a":1}', '{a:1}', '[]'],
  isBase64: ['aGVsbG8=', 'hello!'],
  isLength: ['abc'],
  isLowercase: ['abc', 'aBc'],
  isUppercase: ['ABC', 'aBC'],
  isEmpty: ['', ' '],
  isMACAddress: ['01:23:45:67:89:ab', 'xyz'],
  isPort: ['80', '70000'],
  isCreditCard: ['4111111111111111', '1234'],
  isISBN: ['0-306-40615-2', '123'],
  isMobilePhone: ['+4915112345678', 'abc'],
  isPostalCode: ['12345'],
  isSlug: ['hello-world', 'Hello World'],
  isDate: ['2020-01-02', '2020-13-45', '01/02/2020'],
  isISO8601: ['2020-01-02T03:04:05Z', '2020-01-02', 'nope', '2020-W01-1'],
  isRFC3339: ['2020-01-02T03:04:05Z', '2020-01-02'],
  isTaxID: ['123456789'],
  isIdentityCard: ['123456789'],
  isStrongPassword: ['Abcdef1!x', 'weak'],
  isMailtoURI: ['mailto:a@b.co', 'x'],
  isLuhnNumber: ['4111111111111111'],
  isSemVer: ['1.2.3', '1.2'],
  isMD5: ['d41d8cd98f00b204e9800998ecf8427e', 'x'],
  isDataURI: ['data:text/plain;base64,aGk=', 'x'],
  isFQDN: ['example.com', 'a_b.com'],
};
for (const [fn, inputs] of Object.entries(cases)) {
  console.log(fn, inputs.map((x) => { try { return v[fn](x); } catch (e) { return 'E:' + e.message; } }).join());
}
console.log(v.isLength('abc', { min: 2, max: 4 }), v.isInt('5', { min: 1, max: 4 }), v.isEmail('a@b.co', { allow_display_name: true }), v.isIn('a', ['a', 'b']), v.isAfter('2021-01-01', '2020-01-01'), v.isBefore('2019-01-01', '2020-01-01'));
console.log(v.escape('<a href="x">&\'</a>'), v.unescape('&lt;b&gt;&amp;'), v.trim('  x  '), v.ltrim('xxabc', 'x'), v.rtrim('abcxx', 'x'), v.stripLow('a\u0001b'), v.whitelist('abc123', 'a-z'), v.blacklist('abc123', 'a-z'));
console.log(v.normalizeEmail('Foo.Bar+tag@GMAIL.com'), v.normalizeEmail('A@Example.COM'), v.toInt('42px'), v.toFloat('1.5x'), v.toBoolean('yes'), v.toBoolean('0'), v.contains('abc', 'b'), v.equals('a', 'a'), v.matches('abc', /^a/), v.isByteLength('abc', { min: 1 }));
console.log(v.toDate('2020-01-02T03:04:05Z').toISOString(), v.toDate('nope'), v.isDate('2020-02-30'), v.isAfter('2020-01-01'), v.version);
