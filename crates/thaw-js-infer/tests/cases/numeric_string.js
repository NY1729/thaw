function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function gcd(a, b) { while (b !== 0) { var t = a % b; a = b; b = t; } return a; }
function sumTo(n) { var s = 0; for (var i = 0; i <= n; i++) { s += i; } return s; }
function rev(str) { var out = ''; for (var i = str.length - 1; i >= 0; i--) { out += str.charAt(i); } return out; }
function hash(str) { var h = 0; for (var i = 0; i < str.length; i++) { h = (h * 31 + str.charCodeAt(i)) | 0; } return h; }
function total(xs) { var s = 0; for (var i = 0; i < xs.length; i++) { s += xs[i]; } return s; }
function pad(n) { return (n < 10 ? '0' : '') + n; }
console.log(fib(20), gcd(1071, 462), sumTo(100));
console.log(rev('hello'), hash('thaw'), pad(7), pad(12));
console.log(total([1, 2, 3, 4.5]));
