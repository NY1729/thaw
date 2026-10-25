var api = {
  add: function (a, b) { return a + b; },
  greet: function (name) { return 'hi ' + name.toUpperCase(); },
  sum: function (xs) { var s = 0; for (var i = 0; i < xs.length; i++) { s += xs[i]; } return s; }
};
console.log(api.add(2, 3), api.greet('thaw'), api.sum([1, 2, 3]));
