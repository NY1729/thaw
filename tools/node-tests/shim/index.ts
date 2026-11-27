// Minimal ESM port of nodejs/node test/common/index.js (v22.23.1). Same contract: function bodies are copied
// from upstream; the only changes are ESM syntax and omitting helpers that need child processes / internals
// (those tests are excluded by select.py, never papered over here).
import assert from 'assert';
import path from 'path';
import { inspect } from 'util';

const isWindows = process.platform === 'win32';
const isLinux = process.platform === 'linux';
const isMacOS = process.platform === 'darwin';
const isDebug = false;
const hasCrypto = Boolean(process.versions.openssl) && !process.env.NODE_SKIP_CRYPTO;
const hasIntl = !!process.config.variables.v8_enable_i18n_support;

const mustCallChecks: any[] = [];

function runCallChecks(exitCode: number) {
  if (exitCode !== 0) return;
  const failed = mustCallChecks.filter(function(context: any) {
    if ('minimum' in context) {
      context.messageSegment = `at least ${context.minimum}`;
      return context.actual < context.minimum;
    }
    context.messageSegment = `exactly ${context.exact}`;
    return context.actual !== context.exact;
  });
  failed.forEach(function(context: any) {
    console.log('Mismatched %s function calls. Expected %s, actual %d.',
                context.name, context.messageSegment, context.actual);
    console.log(context.stack.split('\n').slice(2).join('\n'));
  });
  if (failed.length) process.exit(1);
}

// ponytail: AOT (thaw) types the callback statically. Calls with a non-function first argument
// (mustCall(), mustCall(n)) are not representable; a test using them is reported as a build-fail.
// `this` is not forwarded (a `this`-forwarding wrapper segfaults in AOT, fn.apply/call spreads are rejected).
type Fn = (...a: any[]) => any;

function mustCall(fn: Fn, exact?: any): any {
  return _mustCallInner(fn, exact, 'exact');
}

function mustSucceed(fn: Fn, exact?: any): any {
  return mustCall(function(err: any, ...args: any[]) {
    assert.ifError(err);
    return fn(...args);
  }, exact);
}

function mustCallAtLeast(fn: Fn, minimum?: any): any {
  return _mustCallInner(fn, minimum, 'minimum');
}

function _mustCallInner(fn: Fn, crit: any, field: string): any {
  if (process._exiting)
    throw new Error('Cannot use common.mustCall*() in process exit handler');
  const criteria: any = crit === undefined ? 1 : crit;
  if (typeof criteria !== 'number')
    throw new TypeError(`Invalid ${field} value: ${criteria}`);
  const context: any = {
    [field]: criteria as number,
    actual: 0,
    stack: inspect(new Error()),
    name: fn.name || '<anonymous>',
  };
  if (mustCallChecks.length === 0) process.on('exit', runCallChecks);
  mustCallChecks.push(context);
  const _return: any = function(...a: any[]) {
    context.actual++;
    return fn(...a);
  };
  // not replicated: name/length (AOT dynamic objects cannot store writable:false/configurable attributes)
  return _return;
}

function mustNotCall(msg?: any): any {
  const callSite = String(new Error().stack).split('\n')[2] || '';
  return function mustNotCall(...args: any[]) {
    const argsInfo = args.length > 0 ?
      `\ncalled with arguments: ${args.map((arg: any) => inspect(arg)).join(', ')}` : '';
    assert.fail(`${msg || 'function should not have been called'} at ${callSite.trim()}` + argsInfo);
  };
}

function printSkipMessage(msg: any) {
  console.log(`1..0 # Skipped: ${msg}`);
}

function skip(msg: any) {
  printSkipMessage(msg);
  process.exit(0);
}

function platformTimeout(ms: any): any {
  return ms;
}

// expectWarning: omitted (dispatch through a dynamic table of callbacks is not expressible in AOT); select.py excludes users.

function expectsError(validator: any, exact?: any): any {
  return mustCall((...args: any[]) => {
    if (args.length !== 1) {
      assert.fail(`Expected one argument, got ${inspect(args)}`);
    }
    const error = args.pop();
    assert.strictEqual(Object.prototype.propertyIsEnumerable.call(error, 'message'), false);
    assert.throws(() => { throw error; }, validator);
    return true;
  }, exact);
}

function getArrayBufferViews(buf: any): any[] {
  const { buffer, byteOffset, byteLength } = buf;
  const out: any[] = [];
  const arrayBufferViews = [
    Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array,
    Float32Array, Float64Array, BigInt64Array, BigUint64Array, DataView,
  ];
  for (const type of arrayBufferViews) {
    const { BYTES_PER_ELEMENT = 1 } = type;
    if (byteLength % BYTES_PER_ELEMENT === 0) {
      out.push(new type(buffer, byteOffset, byteLength / BYTES_PER_ELEMENT));
    }
  }
  return out;
}

function getBufferSources(buf: any): any[] {
  return [...getArrayBufferViews(buf), new Uint8Array(buf).buffer];
}

function invalidArgTypeHelper(input: any): string {
  if (input == null) {
    return ` Received ${input}`;
  }
  if (typeof input === 'function') {
    return ` Received function ${input.name}`;
  }
  if (typeof input === 'object') {
    if (input.constructor?.name) {
      return ` Received an instance of ${input.constructor.name}`;
    }
    return ` Received ${inspect(input, { depth: -1 })}`;
  }
  let inspected = inspect(input, { colors: false });
  // upstream passes (inspected, 0, 25) to slice(), which yields "" (kept; AOT rejects the 3-arg call)
  if (inspected.length > 28) { inspected = `${inspected.slice(0, 0)}...`; }
  return ` Received type ${typeof input} (${inspected})`;
}

const PIPE = path.join(path.relative(process.cwd(), `${process.env.TMPDIR || '/tmp'}/`), `node-test.${process.pid}.sock`);

const common: any = {
  expectsError, getArrayBufferViews, getBufferSources, hasIntl, hasCrypto,
  invalidArgTypeHelper, isDebug, isLinux, isMacOS, isWindows, mustCall, mustCallAtLeast, mustNotCall,
  mustSucceed, PIPE, platformTimeout, printSkipMessage, skip,
  isFreeBSD: false, isOpenBSD: false, isSunOS: false, isAIX: false, isIBMi: false, isASan: false,
  hasIPv6: true,
  localhostIPv4: '127.0.0.1',
};

export default common;
export { expectsError, getArrayBufferViews, getBufferSources, hasIntl, hasCrypto, invalidArgTypeHelper, isDebug,
  isLinux, isMacOS, isWindows, mustCall, mustCallAtLeast, mustNotCall, mustSucceed, PIPE, platformTimeout,
  printSkipMessage, skip };
