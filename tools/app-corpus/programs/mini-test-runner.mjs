// A tiny test runner built on node:assert, with failures reported and an exit code.
import assert from 'node:assert/strict';

const tests = [];
const test = (name, fn) => tests.push({ name, fn });

test('equal', () => assert.equal(1 + 1, 2));
test('deepEqual', () => assert.deepEqual({ a: [1, 2, { b: 3 }] }, { a: [1, 2, { b: 3 }] }));
test('notDeepEqual fails', () => assert.deepEqual({ a: 1 }, { a: 2 }));
test('throws', () => assert.throws(() => { throw new TypeError('bad'); }, TypeError));
test('throws message', () => assert.throws(() => { throw new Error('abc'); }, /abd/));
test('rejects', async () => assert.rejects(Promise.reject(new Error('r')), { message: 'r' }));
test('async fail', async () => { await null; assert.ok(false, 'nope'); });
test('strictEqual', () => assert.strictEqual('a', 'a'));
test('match', () => assert.match('hello world', /wor/));

let failed = 0;
for (const { name, fn } of tests) {
  try {
    await fn();
    console.log('ok  ', name);
  } catch (e) {
    failed++;
    console.log('FAIL', name, '-', e.name, e.code ?? '', e.message.split('\n')[0]);
  }
}
console.log(`${tests.length - failed} passed, ${failed} failed`);
process.exitCode = failed ? 1 : 0;
