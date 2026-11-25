// Class-heavy domain model with inheritance, private fields, getters, toJSON and revivers.
class Money {
  constructor(cents, currency = 'USD') {
    this.cents = cents;
    this.currency = currency;
  }
  add(o) {
    return new Money(this.cents + o.cents, this.currency);
  }
  toString() {
    return `${(this.cents / 100).toFixed(2)} ${this.currency}`;
  }
  toJSON() {
    return { $money: this.cents, c: this.currency };
  }
}

class Entity {
  static #count = 0;
  #id;
  constructor() {
    this.#id = ++Entity.#count;
  }
  get id() {
    return this.#id;
  }
  static get count() {
    return Entity.#count;
  }
  toJSON() {
    return { id: this.#id, type: this.constructor.name, ...this.fields() };
  }
  fields() {
    return {};
  }
}

class Product extends Entity {
  constructor(name, price) {
    super();
    this.name = name;
    this.price = price;
  }
  fields() {
    return { name: this.name, price: this.price };
  }
}

class Order extends Entity {
  lines = [];
  add(product, qty) {
    this.lines.push({ product, qty });
    return this;
  }
  get total() {
    return this.lines.reduce((s, l) => s.add(new Money(l.product.price.cents * l.qty)), new Money(0));
  }
  fields() {
    return { lines: this.lines, total: this.total };
  }
}

const widget = new Product('widget', new Money(1999));
const gadget = new Product('gadget', new Money(550));
const order = new Order().add(widget, 2).add(gadget, 3);
const json = JSON.stringify(order, null, 1);
console.log(json);
console.log(String(order.total), Entity.count, order instanceof Entity, Object.keys(order));
const back = JSON.parse(json, (k, v) => (v && v.$money !== undefined ? new Money(v.$money, v.c) : v));
console.log(back.total instanceof Money, back.lines[0].product.price.toString());
