"use strict";
exports.area = function (r) { return r.w * r.h; };
exports.label = function (p) { return p.name + ":" + p.tags.join(","); };
exports.longest = function (words) {
  var best = "";
  for (var i = 0; i < words.length; i++) {
    if (words[i].length > best.length) { best = words[i]; }
  }
  return best;
};
exports.allTrue = function (flags) {
  for (var i = 0; i < flags.length; i++) { if (!flags[i]) { return false; } }
  return true;
};
exports.grow = function (xs) { xs.push(1); return xs.length; };
