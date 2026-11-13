// Navigator cannot be constructed by user code.
try {
  new Navigator();
} catch (e) {
  console.log(e.name, e.message, e.code, e.constructor === TypeError);
}
try {
  Reflect.construct(Navigator, []);
} catch (e) {
  console.log(e.name, e.message);
}
console.log(navigator.constructor === Navigator, Navigator.prototype.constructor === Navigator);
