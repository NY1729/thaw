class Shape {
  constructor(name) { this.name = name; }
  area() { return 0; }
  describe() { return this.name + ':' + this.area(); }
}
class Rect extends Shape {
  constructor(w, h) { super('rect'); this.w = w; this.h = h; }
  area() { return this.w * this.h; }
}
class Sq extends Rect {
  constructor(s) { super(s, s); }
}
console.log(new Rect(2, 3).describe(), new Sq(4).describe(), new Shape('x').describe());
