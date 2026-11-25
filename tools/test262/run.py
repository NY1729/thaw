#!/usr/bin/env python3
"""test262 runner for thaw-js-node. See README.md.

usage: python3 -I run.py --out RESULTS.jsonl [--dirs test/built-ins/Array,...] [--exclude test/language/expressions,...]
                         [--files LIST.txt] [--jobs N]
env:   TEST262_DIR (checkout, required), THAW_JS_NODE (engine binary, required)
Output: one JSON line per test {path,status,message}; status is pass|fail|crash|timeout|skip. Resumable.
"""
import sys, os, re, json, subprocess, tempfile, resource, time, argparse
from concurrent.futures import ThreadPoolExecutor

ROOTS = ['test/built-ins', 'test/language', 'test/annexB']  # staging and intl402 are out of scope
TIMEOUT = 20            # seconds per engine run
RLIMIT_AS = 2 << 30     # address-space cap per engine run
SKIP = {'Atomics.waitAsync', 'resizable-arraybuffer', 'Temporal', 'ShadowRealm', 'Intl', 'cross-realm', 'IsHTMLDDA',
        'agent', 'SharedArrayBuffer', 'Atomics', 'tail-call-optimization', 'FinalizationRegistry', 'WeakRef',
        'async-iteration-x', 'host-gc-required', 'import-attributes', 'source-phase-imports', 'json-modules',
        'Intl.DurationFormat'}
SKIP_SRC = ['$262.agent', 'createRealm', 'IsHTMLDDA']
PRE = ("var print=console.log;var $262={global:globalThis,gc:function(){},evalScript:function(s){return (0,eval)(s)},"
       "detachArrayBuffer:function(b){b.transfer()},agent:{},createRealm:function(){throw new Error('no realm')},"
       "IsHTMLDDA:undefined};\n"
       # an unhandled rejection is not a test failure in the test262 host model (Node 22 would exit 1 on it)
       "process.on('unhandledRejection',function(){});\n")
T = os.environ.get('TEST262_DIR')
HC = {}


def harness(n):
    if n not in HC:
        HC[n] = open(f'{T}/harness/{n}', encoding='utf-8').read() + '\n'
    return HC[n]


def meta(src):
    m = re.search(r'/\*---(.*?)---\*/', src, re.S)
    y = m.group(1) if m else ''
    inc = []
    mi = re.search(r'^includes:\s*\[(.*?)\]', y, re.M)
    if mi:
        inc = [s.strip() for s in mi.group(1).split(',') if s.strip()]
    else:
        mb = re.search(r'^includes:\s*\n((?:\s+-.*\n)+)', y, re.M)
        if mb:
            inc = [l.strip()[1:].strip() for l in mb.group(1).splitlines()]
    fl = re.search(r'^flags:\s*\[(.*?)\]', y, re.M)
    fl = [s.strip() for s in fl.group(1).split(',')] if fl else []
    ft = re.search(r'^features:\s*\[(.*?)\]', y, re.M)
    ft = [s.strip() for s in ft.group(1).split(',')] if ft else []
    neg = re.search(r'^negative:\s*\n\s+phase:\s*(\w+)\s*\n\s+type:\s*(\w+)', y, re.M)
    return inc, fl, ft, neg


def lim():
    resource.setrlimit(resource.RLIMIT_AS, (RLIMIT_AS, RLIMIT_AS))


def run(p, eng, tmp):
    src = open(p, encoding='utf-8').read()
    inc, fl, ft, neg = meta(src)
    rel = os.path.relpath(p, T)
    if 'module' in fl:
        return dict(path=rel, status='skip', message='skip:module')
    for f in ft:
        if f in SKIP:
            return dict(path=rel, status='skip', message='skip:feature:' + f)
    for s in SKIP_SRC:
        if s in src:
            return dict(path=rel, status='skip', message='skip:src:' + s)
    modes = ['raw'] if 'raw' in fl else ['strict'] if 'onlyStrict' in fl else ['sloppy'] if 'noStrict' in fl else ['sloppy', 'strict']
    for mode in modes:
        h = '' if mode == 'raw' else PRE
        if mode != 'raw':
            for n in ['assert.js', 'sta.js'] + (['doneprintHandle.js'] if 'async' in fl else []) + inc:
                h += harness(n)
        body = ('"use strict";\n' if mode == 'strict' else '') + h + src
        with tempfile.NamedTemporaryFile('w', suffix='.js', dir=tmp, delete=False, encoding='utf-8') as f:
            f.write(body)
            fn = f.name
        try:
            r = subprocess.run(eng + [fn], capture_output=True, text=True, timeout=TIMEOUT, preexec_fn=lim,
                               stdin=subprocess.DEVNULL, errors='replace')
        except subprocess.TimeoutExpired:
            os.unlink(fn)
            return dict(path=rel, status='timeout', message=mode)
        os.unlink(fn)
        out = r.stdout + r.stderr
        if neg:
            if not (r.returncode != 0 and (neg.group(2) in out or neg.group(1) == 'parse')):
                return dict(path=rel, status='fail', message=mode + ': expected ' + neg.group(2))
            continue
        if r.returncode < 0 or r.returncode >= 124 and r.returncode not in (1, 3):
            return dict(path=rel, status='crash', message=f'{mode} rc={r.returncode}')
        good = 'Test262:AsyncTestComplete' in r.stdout if 'async' in fl else r.returncode == 0
        if not good:
            return dict(path=rel, status='fail', message=mode + ': ' + out.strip()[:160].replace('\n', ' '))
    return dict(path=rel, status='pass', message='')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', required=True)
    ap.add_argument('--dirs', default='')
    ap.add_argument('--exclude', default='')
    ap.add_argument('--files', help='text file with test ids (paths relative to the test262 root) to run')
    ap.add_argument('--jobs', type=int, default=3)
    a = ap.parse_args()
    global T
    if not T or not os.environ.get('THAW_JS_NODE'):
        sys.exit('set TEST262_DIR and THAW_JS_NODE')
    eng = os.environ['THAW_JS_NODE'].split()
    tmp = tempfile.mkdtemp(prefix='t262-')
    done = set()
    if os.path.exists(a.out):
        for line in open(a.out, encoding='utf-8'):
            try:
                done.add(json.loads(line)['path'])
            except Exception:
                pass  # torn last line from a killed run: re-run that test
    if a.files:
        files = [os.path.join(T, l.strip()) for l in open(a.files) if l.strip()]
    else:
        excl = [e.rstrip('/') + '/' for e in a.exclude.split(',') if e]
        files = []
        for root in (a.dirs.split(',') if a.dirs else ROOTS):
            for d, ds, fs in os.walk(os.path.join(T, root)):
                ds.sort()
                for f in sorted(fs):
                    p = os.path.join(d, f)
                    if f.endswith('.js') and '_FIXTURE' not in f and not any(os.path.relpath(p, T).startswith(e) for e in excl):
                        files.append(p)
    todo = [p for p in files if os.path.relpath(p, T) not in done]
    print(f'files={len(files)} done={len(done)} todo={len(todo)} jobs={a.jobs}', file=sys.stderr, flush=True)
    t0 = time.time()
    with open(a.out, 'a', encoding='utf-8') as out, ThreadPoolExecutor(a.jobs) as ex:
        for i, r in enumerate(ex.map(lambda p: run(p, eng, tmp), todo), 1):
            out.write(json.dumps(r, ensure_ascii=False) + '\n')
            out.flush()
            if i % 2000 == 0:
                print(f'{i}/{len(todo)} {(time.time() - t0) / 60:.1f}min', file=sys.stderr, flush=True)
    print(f'finished {len(todo)} tests in {(time.time() - t0) / 60:.1f}min', file=sys.stderr, flush=True)
    os.rmdir(tmp)


main()
