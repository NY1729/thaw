"use strict";
exports.evens = function (n) {
  var out = [];
  for (var i = 0; i < n; i++) { if (i % 2 === 0) { out.push(i); } }
  return out;
};
exports.scale = function (xs, k) {
  var out = [];
  for (var i = 0; i < xs.length; i++) { out.push(xs[i] * k); }
  return out;
};
exports.words = function (s) { return s.split(" "); };
exports.same = function (xs) { return xs; };
