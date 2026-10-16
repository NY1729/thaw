var contexts = globalThis.__thaw_vm_contexts || (globalThis.__thaw_vm_contexts = new WeakMap());
var contextReleaser = typeof FinalizationRegistry === 'function' && typeof __thaw_vm_free === 'function' ? new FinalizationRegistry(function (id) { __thaw_vm_free(id); }) : null;
function describeReceived(value) {
  if (value === null || value === undefined) return ' Received ' + value;
  if (typeof value === 'function') return ' Received function ' + value.name;
  if (typeof value === 'object') return value.constructor && value.constructor.name ? ' Received an instance of ' + value.constructor.name : ' Received [Object: null prototype] {}';
  var shown = typeof value === 'string' ? "'" + value + "'" : typeof value === 'bigint' ? String(value) + 'n' : String(value);
  if (shown.length > 28) shown = shown.slice(0, 25) + '...';
  return ' Received type ' + typeof value + ' (' + shown + ')';
}
function invalidArgument(message, value) { var error = new TypeError(message + describeReceived(value)); error.code = 'ERR_INVALID_ARG_TYPE'; return error; }
function validateObject(value, name) { if (value === null || (typeof value !== 'object' && typeof value !== 'function')) throw invalidArgument('The "' + name + '" argument must be of type object.', value); }
function createContext(object, options) {
  var context = object === undefined || object === Symbol.for('vm_context_no_contextify') ? {} : object;
  validateObject(context, 'object');
  if (contexts.has(context)) return context;
  var id = __thaw_vm_create();
  contexts.set(context, id);
  if (contextReleaser) contextReleaser.register(context, id);
  return context;
}
function isContext(value) { validateObject(value, 'object'); return contexts.has(value); }
function vmTimeout(options) {
  var timeout = options && options.timeout;
  if (timeout === undefined) return 0;
  if (typeof timeout !== 'number') throw invalidArgument('The "options.timeout" property must be of type number.', timeout);
  if (!Number.isInteger(timeout) || timeout < 1 || timeout > 4294967295) { var rangeError = new RangeError('The value of "options.timeout" is out of range. It must be ' + (Number.isInteger(timeout) ? '>= 1 && <= 4294967295' : 'an integer') + '. Received ' + String(timeout)); rangeError.code = 'ERR_OUT_OF_RANGE'; throw rangeError; }
  return timeout;
}
function vmRun(id, code, sandbox, options) {
  if (typeof options === 'string') options = { filename: options };
  var timeout = vmTimeout(options);
  var filename = options && options.filename !== undefined ? String(options.filename) : 'evalmachine.<anonymous>';
  return __thaw_vm_run(id, String(code), filename, timeout, sandbox, Object.prototype);
}
function runInContext(code, context, options) {
  validateObject(context, 'object');
  if (!contexts.has(context)) throw invalidArgument('The "contextifiedObject" argument must be an vm.Context.', context);
  return vmRun(contexts.get(context), code, context, options);
}
function runInNewContext(code, context, options) { return runInContext(code, createContext(context === undefined ? {} : context), options); }
function runInThisContext(code, options) {
  if (typeof options === 'string') options = { filename: options };
  return typeof __thaw_vm_run === 'function' ? vmRun(-1, code, undefined, options) : (0, eval)(String(code));
}
