const a = 12345678901234567890n, b = 987654321n;
console.log(a * b, a / b, a % b, a ** 3n, -a, a > b, a == 12345678901234567890n, 1n < 2, typeof a);
console.log(BigInt(10) + 5n, BigInt('0xff'), (255n).toString(16), BigInt.asUintN(8, 257n), BigInt.asIntN(8, 255n));
console.log(1n << 70n, (1n << 70n) >> 3n, 5n & 3n, 5n | 3n, 5n ^ 3n, ~5n);
console.log(Number(2n**53n), parseInt('12'), 0n ? 't':'f', 1n ? 't':'f', JSON.stringify({a:String(1n)}));
try { 1n + 1; } catch (e) { console.log(e instanceof TypeError, e.message); }
console.log([3n,1n,2n].sort((x,y)=> x<y?-1:1), `${10n}`, 7n/2n, -7n/2n, -7n%3n);
console.log(BigInt(Number.MAX_SAFE_INTEGER) + 2n, typeof Object(1n), 10n === 10n);
