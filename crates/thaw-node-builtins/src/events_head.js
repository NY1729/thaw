var kRejection = Symbol.for('nodejs.rejection'), kCapture = Symbol('kCapture'), kShapeMode = Symbol('shapeMode'), ObjectGetPrototypeOf = Object.getPrototypeOf;
function nodeTypeName(value) {
  if (value == null) return ' Received ' + value;
  if (typeof value === 'function') return ' Received function ' + value.name;
  if (typeof value === 'object') return value.constructor && value.constructor.name ? ' Received an instance of ' + value.constructor.name : ' Received [Object: null prototype]';
  var text = typeof value === 'string' ? "'" + value + "'" : typeof value === 'bigint' ? String(value) + 'n' : Object.is(value, -0) ? '-0' : String(value);
  if (text.length > 28) text = text.slice(0, 25) + '...';
  return ' Received type ' + typeof value + ' (' + text + ')';
}
function argTypeError(name, type, value) { var error = new TypeError('The "' + name + '" argument must be ' + type + '.' + nodeTypeName(value)); error.code = 'ERR_INVALID_ARG_TYPE'; return error; }
function validateBoolean(value, name) { if (typeof value !== 'boolean') { var error = argTypeError(name, 'of type boolean', value); if (name.indexOf('.') >= 0) error.message = error.message.replace('"' + name + '" argument', '"' + name + '" property'); throw error; } }
function validateNumber(value, name, min) {
  if (typeof value !== 'number') throw argTypeError(name, 'of type number', value);
  if (value < min || value !== value) { var error = new RangeError('The value of "' + name + '" is out of range. It must be >= ' + min + '. Received ' + String(value)); error.code = 'ERR_OUT_OF_RANGE'; throw error; }
}
function spliceOne(list, index) { for (; index + 1 < list.length; index++) list[index] = list[index + 1]; list.pop(); }
function checkListener(listener) { if (typeof listener !== 'function') throw argTypeError('listener', 'of type function', listener); }
function EventEmitter(opts) {
  if (!(this instanceof EventEmitter)) return new EventEmitter(opts);
  EventEmitter.init.call(this, opts);
}
Object.defineProperty(EventEmitter.prototype, kCapture, { value: false, writable: true, enumerable: false });
EventEmitter.defaultMaxListeners = 10; EventEmitter.captureRejections = false;
