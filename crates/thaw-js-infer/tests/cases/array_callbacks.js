function stats(xs) {
  var doubled = xs.map(function (x, i) { return x * 2 + i; });
  var total = xs.reduce(function (a, b) { return a + b; }, 0);
  var big = xs.filter(function (x) { return x > 2; });
  var any = xs.some(function (x) { return x === 3; });
  var all = xs.every(function (x) { return x > 0; });
  var found = xs.find(function (x) { return x > 2; });
  var out = [];
  xs.forEach(function (x) { out.push(x + 1); });
  return doubled.join(',') + ' ' + total + ' ' + big.length + ' ' + any + ' ' + all + ' ' + found + ' ' + out.join('-');
}
function lens(words) {
  return words.map(function (w) { return w.length; }).join(' ');
}
var s = stats([1, 2, 3, 4]);
console.log(s);
console.log(lens(['a', 'bcd', 'ef']));
var a = [3, 1, 2];
a.push(9);
console.log(a.slice(1).concat([7, 8]).indexOf(9), a.join('+'));
