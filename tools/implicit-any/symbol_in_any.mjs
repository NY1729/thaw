const s = Symbol('d');
const k = Symbol.for('shared');
const holder = { sym: s, other: k };
const box = [s, k];
console.log(typeof holder.sym, typeof box[1], holder.sym === s, box[1] === Symbol.for('shared'), s === k);
console.log(box[0], holder.other);
function kind(v) { return typeof v; }
console.log(kind(Symbol('arg')), kind(s), kind(1));
