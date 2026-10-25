function acc(n) { var s; var i; s = 0; for (i = 0; i < n; i++) { s += i * 2; } return s; }
function pick(flag) { var v = 1; if (flag) { v = 'one'; } return v; }
function firstOr(xs, d) { var v = xs[0]; return v === undefined ? d : v; }
function idv(x) { return x; }
console.log(acc(5), pick(false), pick(true));
console.log(firstOr([4], 9));
console.log(idv(3), idv('s'));
