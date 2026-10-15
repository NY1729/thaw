// Wave UC: tagged-template invalid escapes and explicit resource management.
// Expected output recorded from node 22 (`using` from a tsc-downleveled reference).

#[test]
fn uc_contract_invalid_escape_cooked_entries_are_undefined() {
    let source = r##"function tag(s: TemplateStringsArray, ...v: number[]): string {
  const parts: string[] = [];
  for (let i = 0; i < s.length; i++) parts.push(s[i] === undefined ? 'U' : s[i]);
  return parts.join('|') + '#' + s.raw.join('|');
}
function probe(s: TemplateStringsArray): string {
  const u0 = s[0] === undefined;
  const u1 = s[1] === undefined;
  const v1 = s[1] !== undefined;
  const s1 = s[1];
  const w1 = s1 === undefined;
  console.log(s.length, JSON.stringify(s), s[1]);
  console.log(u0, u1, v1, w1);
  console.log(typeof s[1], typeof s[0]);
  console.log(s[0] ?? 'dflt');
  console.log(s[1] ?? 'dflt');
  return 'x';
}
async function main(): Promise<void> {
  console.log(tag`a\unicode${1}b`);
  console.log(tag`ok${2}\xz`);
  console.log(probe`a${1}\unicode`);
}
"##;
    assert_eq!(
        compile_and_run(source, "uc_contract_invalid_escape_cooked_entries_are_undefined"),
        "U|b#a\\unicode|b\nok|U#ok|\\xz\n2 [\"a\",null] undefined\nfalse true false true\nundefined string\na\ndflt\nx\n"
    );
}

#[test]
fn uc_contract_invalid_escape_template_object_stays_frozen_and_stable() {
    let source = r##"function id(s: TemplateStringsArray): TemplateStringsArray { return s; }
const same = (): TemplateStringsArray => id`a\unicode${1}`;
async function main(): Promise<void> {
  console.log(same() === same(), Object.isFrozen(same()), same().raw[0], same()[0] === undefined);
  console.log(String.raw`a\unicode${1}b`);
}
"##;
    assert_eq!(
        compile_and_run(source, "uc_contract_invalid_escape_template_object_stays_frozen_and_stable"),
        "true true a\\unicode true\na\\unicode1b\n"
    );
}


#[test]
fn uc_contract_using_matches_the_downleveled_reference() {
    // Expected output: tsc 5.9 downlevel of this program, run on node 22.
    let source = r##"const log: string[] = [];
function res(name: string): Disposable {
  log.push('open ' + name);
  return { [Symbol.dispose]() { log.push('close ' + name); } };
}
class Conn {
  constructor(public id: number) { log.push('conn ' + id); }
  [Symbol.dispose](): void { log.push('conn-close ' + this.id); }
}
function early(flag: boolean): string {
  using a = res('a');
  if (flag) return 'early';
  using b = res('b');
  return 'late';
}
function nothing(): void {
  using n = null;
  using u = undefined;
  log.push('body');
}
function loop(): void {
  for (let i = 0; i < 4; i++) {
    using r = res('L' + i);
    if (i === 1) continue;
    if (i === 2) break;
    log.push('iter ' + i);
  }
}
function sw(k: number): void {
  switch (k) {
    case 1: {
      using s = res('s1');
      log.push('case1');
      break;
    }
    default:
      log.push('default');
  }
}
function thrower(): void {
  try {
    using a = res('t1');
    using c = new Conn(7);
    throw new Error('boom');
  } catch (e) {
    log.push('caught ' + (e as Error).message);
  }
}
async function main(): Promise<void> {
  log.push(early(true), early(false));
  nothing();
  loop();
  sw(1); sw(2);
  thrower();
  console.log(log.join('\n'));
}
"##;
    assert_eq!(
        compile_and_run(source, "uc_contract_using_matches_the_downleveled_reference"),
        "open a\nclose a\nopen a\nopen b\nclose b\nclose a\nearly\nlate\nbody\nopen L0\niter 0\nclose L0\nopen L1\nclose L1\nopen L2\nclose L2\nopen s1\ncase1\nclose s1\ndefault\nopen t1\nconn 7\nconn-close 7\nclose t1\ncaught boom\n"
    );
}


#[test]
fn uc_contract_using_nullish_resources_and_await_using_match_the_reference() {
    // Expected output: tsc 5.9 downlevel of this program, run on node 22.
    let source = r##"const log: string[] = [];
function res(name: string): Disposable {
  log.push('open ' + name);
  return { [Symbol.dispose]() { log.push('close ' + name); } };
}
function maybe(on: boolean, name: string): Disposable | null {
  return on ? res(name) : null;
}
function maybeU(on: boolean, name: string): Disposable | undefined {
  return on ? res(name) : undefined;
}
function opt(on: boolean): void {
  using a = maybe(on, 'n');
  using b = maybeU(on, 'u');
  log.push('opt ' + on);
}
class Handle implements AsyncDisposable {
  constructor(private tag: string) { log.push('h-open ' + tag); }
  async [Symbol.asyncDispose](): Promise<void> {
    await Promise.resolve();
    log.push('h-close ' + this.tag);
  }
}
async function work(fail: boolean): Promise<string> {
  await using h1 = new Handle('one');
  using s = res('sync');
  await using h2 = new Handle('two');
  await Promise.resolve();
  if (fail) throw new Error('work failed');
  return 'done';
}
const arrow = async (): Promise<number> => {
  using r = res('arrow');
  await Promise.resolve();
  return 7;
};
async function main(): Promise<void> {
  opt(true); opt(false);
  log.push(await work(false));
  try { await work(true); } catch (e) { log.push('caught ' + (e as Error).message); }
  log.push('arrow ' + (await arrow()));
  console.log(log.join('\n'));
}

"##;
    assert_eq!(
        compile_and_run(source, "uc_contract_using_nullish_resources_and_await_using_match_the_reference"),
        "open n\nopen u\nopt true\nclose u\nclose n\nopt false\nh-open one\nopen sync\nh-open two\nh-close two\nclose sync\nh-close one\ndone\nh-open one\nopen sync\nh-open two\nh-close two\nclose sync\nh-close one\ncaught work failed\nopen arrow\nclose arrow\narrow 7\n"
    );
}
