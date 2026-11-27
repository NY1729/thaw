#!/usr/bin/env python3
"""Node-own-tests runner (nodejs/node test/parallel) for thaw AOT. See README.md.

  python3 -I run.py select --tree TREE --work WORK            # writes eligible.txt + skip.txt
  python3 -I run.py run --tree TREE --work WORK --out R.jsonl [--thaw BIN] [--prefix test-path-,test-url-] [--files LIST] [--jobs N]
env: NODE_TESTS_DIR (= --tree), NODE_TESTS_WORK, THAW (thaw binary), NODE_TESTS_CACHE (result cache dir)

A test is the upstream file, mechanically rewritten from CommonJS to ESM (`const x = require('m')` ->
`import x from 'm'`, `require('../common')` -> the shim in shim/), run as WORK/test/parallel/<name>.js.
Output: one JSON line per test {path,status,message,node}; status is pass|build-fail|runtime-fail|crash|timeout.
"""
import sys, os, re, json, subprocess, shutil, hashlib, time, argparse, resource
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
SHIM = os.path.join(HERE, 'shim')
SHIM_NAMES = {'expectsError', 'getArrayBufferViews', 'getBufferSources', 'hasIntl', 'hasCrypto', 'invalidArgTypeHelper',
              'isDebug', 'isLinux', 'isMacOS', 'isWindows', 'mustCall', 'mustCallAtLeast', 'mustNotCall', 'mustSucceed',
              'PIPE', 'platformTimeout', 'printSkipMessage', 'skip', 'isFreeBSD', 'isOpenBSD', 'isSunOS', 'isAIX',
              'isIBMi', 'isASan', 'hasIPv6', 'localhostIPv4'}
SHIM_MODS = {'../common': 'index.ts', '../common/countdown': 'countdown.ts', '../common/fixtures': 'fixtures.ts'}
NODE_TIMEOUT, BUILD_TIMEOUT, RUN_TIMEOUT = 30, 600, 30
RLIMIT_AS = 8 << 30

# first match wins; (reason, regex) applied to the upstream source
STATIC = [
    ('flags-comment', r'^// Flags:'),
    ('internal-module', r"require\(\s*['\"](internal/|_[a-z_]+['\"])|internalBinding|process\.binding|--expose[-_]internals"),
    ('v8-natives', r'%[A-Z][A-Za-z]+\('),
    ('child-process-or-self-spawn', r"child_process|process\.execPath|process\.argv\[0\]|process\.argv\[1\]|\bfork\(|common\.spawnPromisified"),
    ('cluster/worker/inspector/repl', r"require\(\s*['\"](node:)?(cluster|worker_threads|inspector|repl|domain|trace_events|v8|diagnostics_channel|async_hooks|perf_hooks|wasi|vm|readline)['\"]"),
    ('tls/https/http2/crypto-keys', r"require\(\s*['\"](node:)?(tls|https|http2|crypto|dgram|dns)['\"]|fixtures\.readKey|common\.hasCrypto|hasCrypto"),
    ('node:test-runner', r"require\(\s*['\"]node:test['\"]|from\s+['\"]node:test['\"]"),
    ('cjs-only-api', r"module\.exports|\bexports\.|require\.main|require\.resolve|require\.cache|require\.extensions|\bmodule\.(?:filename|paths|parent|children|require)"),
    ('process-global-mutation', r"process\.(chdir|setuid|setgid|umask|kill|abort|reallyExit|emitWarning|setUncaughtExceptionCaptureCallback)|--abort|process\.stdin|process\.stdout\.(fd|isTTY)"),
    ('external-network', r"https?://(?!localhost|127\.0\.0\.1|\[::1\]|example\.(com|org)|test)[A-Za-z0-9.-]+\.[a-z]{2,}|common\.hasMultiLocalhost|internet"),
    ('wpt-or-snapshot-harness', r"common/wpt|assertSnapshot|common/(?!countdown|fixtures|index)"),
]


def static_reason(src):
    for why, rx in STATIC:
        if re.search(rx, src, re.M):
            return why
    props = set(re.findall(r'\bcommon\.([A-Za-z_]\w*)', src))
    for m in re.finditer(r'(?:const|let|var)\s*\{([^}]*)\}\s*=\s*require\(\s*[\'"]\.\./common[\'"]\s*\)', src):
        for part in m.group(1).split(','):
            n = part.split(':')[0].strip()
            if n:
                props.add(n)
    bad = sorted(p for p in props if p not in SHIM_NAMES)
    if bad:
        return 'shim-lacks:' + bad[0]
    return None


IMP_DEF = re.compile(r"^(const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*require\(\s*(['\"])([^'\"]+)\3\s*\)\s*;?[ \t]*$", re.M)
IMP_DES = re.compile(r"^(?:const|let|var)\s*\{([^{}=]*)\}\s*=\s*require\(\s*(['\"])([^'\"]+)\2\s*\)\s*;?[ \t]*$", re.M)
IMP_BARE = re.compile(r"^require\(\s*(['\"])([^'\"]+)\1\s*\)\s*;?[ \t]*$", re.M)


def transform(src):
    """CommonJS -> ESM, top-level statement forms only. Returns (code, residual_require)."""
    def mod(m):
        return '../common/' + SHIM_MODS[m] if m in SHIM_MODS else m

    # `const x = require(...)` of a builtin must stay a default import; a `let`/`var` target could be reassigned.
    src = IMP_DEF.sub(lambda m: f"import {m.group(2)} from '{mod(m.group(4))}';", src)

    def des(m):
        names = []
        for part in m.group(1).split(','):
            part = part.strip()
            if not part:
                continue
            if ':' in part:
                a, b = [x.strip() for x in part.split(':', 1)]
                names.append(f'{a} as {b}')
            else:
                names.append(part)
        return f"import {{ {', '.join(names)} }} from '{mod(m.group(3))}';"
    src = IMP_DES.sub(des, src)
    src = IMP_BARE.sub(lambda m: f"import '{mod(m.group(2))}';", src)
    head = ''
    if re.search(r'\b__filename\b|\b__dirname\b', src):
        head = ("import { fileURLToPath as __fu } from 'url';\nimport { dirname as __dn } from 'path';\n"
                "const __filename = __fu(import.meta.url);\nconst __dirname = __dn(__filename);\n")
    # imports must precede nothing in particular in ESM, so the header may go after 'use strict'
    m = re.match(r"\s*(['\"]use strict['\"];?)", src)
    if m and head:
        src = src[:m.end()] + '\n' + head + src[m.end():]
    else:
        src = head + src
    residual = bool(re.search(r'\brequire\s*\(', src))
    return src, residual


def setup_work(work, tree):
    os.makedirs(f'{work}/test/parallel', exist_ok=True)
    os.makedirs(f'{work}/test/common', exist_ok=True)
    for f in os.listdir(SHIM):
        shutil.copyfile(f'{SHIM}/{f}', f'{work}/test/common/{f}')
    fx = f'{work}/test/fixtures'
    if not os.path.islink(fx):
        os.symlink(f'{tree}/test/fixtures', fx)
    open(f'{work}/package.json', 'w').write('{"type":"module"}\n')


def lim():
    resource.setrlimit(resource.RLIMIT_AS, (RLIMIT_AS, RLIMIT_AS))


def sh(cmd, cwd, timeout, env=None, limit=True):
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, errors='replace', timeout=timeout,
                           stdin=subprocess.DEVNULL, preexec_fn=lim if limit else None, env=env)
        return r.returncode, r.stdout + r.stderr
    except subprocess.TimeoutExpired as e:
        return 'timeout', ''


def node_run(path, cwd):
    return sh(['node', path], cwd, NODE_TIMEOUT, limit=False)  # wasm type-stripping needs address space


def list_tests(tree):
    return sorted(f for f in os.listdir(f'{tree}/test/parallel') if f.startswith('test-') and f.endswith('.js'))


def cmd_select(a):
    tree, work = a.tree, a.work
    setup_work(work, tree)
    names = [n for n in list_tests(tree) if not a.prefix or n.startswith(tuple(a.prefix.split(',')))]

    def one(n):
        src = open(f'{tree}/test/parallel/{n}', encoding='utf-8', errors='replace').read()
        why = static_reason(src)
        if why:
            return n, why
        out, residual = transform(src)
        if residual:
            return n, 'residual-require'
        for _ in range(2):
            rc, o = node_run(f'test/parallel/{n}', tree)
            if rc == 'timeout':
                return n, 'node-original:timeout'
            if rc != 0:
                return n, 'node-original:fails-standalone'
            if 'Skipped' in o and '1..0' in o:
                return n, 'self-skip'
        open(f'{work}/test/parallel/{n}', 'w', encoding='utf-8').write(out)
        for _ in range(2):
            rc, o = node_run(f'test/parallel/{n}', work)
            if rc != 0:
                return n, 'node-transformed:fails'
        return n, None
    with ThreadPoolExecutor(a.jobs) as ex:
        res = list(ex.map(one, names))
    elig = [n for n, w in res if w is None]
    skip = [(n, w) for n, w in res if w]
    counts = {}
    for _, w in skip:
        k = w.split(':')[0] if w.startswith('shim-lacks') else w
        counts[k] = counts.get(k, 0) + 1
    with open(f'{HERE}/eligible.txt', 'w') as f:
        f.write('\n'.join(elig) + '\n')
    with open(f'{HERE}/skip.txt', 'w') as f:
        f.write(f'# {len(names)} candidates, {len(elig)} eligible, {len(skip)} skipped\n')
        for k, v in sorted(counts.items(), key=lambda x: -x[1]):
            f.write(f'# {v:5d} {k}\n')
        for n, w in skip:
            f.write(f'{n}\t{w}\n')
    print(f'{len(names)} candidates, {len(elig)} eligible; skip reasons: {counts}')


def tool_id(thaw):
    h = hashlib.sha256()
    with open(thaw, 'rb') as f:
        for c in iter(lambda: f.read(1 << 22), b''):
            h.update(c)
    return h.hexdigest()[:16]


def norm(msg):
    msg = re.sub(r'/[\w./-]+\.(js|ts|mjs)(:\d+(:\d+)?)?', '<file>', msg)
    msg = re.sub(r'\b\d+\b', 'N', msg)
    return msg.strip()[:200]


def build_msg(out):
    lines = [l for l in out.splitlines() if l.startswith('error:')]
    return norm(lines[-1][6:] if lines else (out.strip().splitlines() or ['?'])[-1])


def run_msg(out):
    lines = [l.strip() for l in out.splitlines() if l.strip()]
    for l in lines:
        if re.search(r'Error|Mismatched|panicked|assert', l):
            return norm(l)
    return norm(lines[-1]) if lines else ''


def run_one(n, a, tid, shimhash, bindir):
    work = a.work
    src = open(f'{work}/test/parallel/{n}', encoding='utf-8').read()
    key = hashlib.sha256((tid + shimhash + src).encode()).hexdigest()
    cf = f'{a.cache}/{key}.json'
    if os.path.exists(cf):
        return json.load(open(cf))
    r = dict(path=n)
    rc, o = node_run(f'test/parallel/{n}', work)
    r['node'] = 'pass' if rc == 0 else 'fail'
    exe = f'{bindir}/{n[:-3]}.bin'
    rc, o = sh([a.thaw, 'build', '--no-install', f'test/parallel/{n}', '-o', exe], work, BUILD_TIMEOUT)
    if rc == 'timeout':
        r.update(status='timeout', message='build')
    elif rc != 0 or not os.path.exists(exe):
        r.update(status='build-fail', message=build_msg(o))
    else:
        rc, o = sh([exe], work, RUN_TIMEOUT)
        if rc == 'timeout':
            r.update(status='timeout', message='run')
        elif rc < 0 or rc >= 128:
            r.update(status='crash', message=f'rc={rc}')
        elif rc != 0:
            r.update(status='runtime-fail', message=run_msg(o))
        else:
            r.update(status='pass', message='')
    if os.path.exists(exe):
        os.unlink(exe)
    os.makedirs(a.cache, exist_ok=True)
    json.dump(r, open(cf, 'w'))
    return r


def cmd_run(a):
    tree, work = a.tree, a.work
    setup_work(work, tree)
    elig = [l.strip() for l in open(f'{HERE}/eligible.txt') if l.strip()]
    if a.files:
        want = {l.strip() for l in open(a.files)}
        elig = [n for n in elig if n in want]
    if a.prefix:
        elig = [n for n in elig if n.startswith(tuple(a.prefix.split(',')))]
    done = set()
    if os.path.exists(a.out):
        for l in open(a.out):
            try:
                done.add(json.loads(l)['path'])
            except Exception:
                pass
    for n in elig:  # (re)write the transformed sources the cache key is computed from
        src = open(f'{tree}/test/parallel/{n}', encoding='utf-8', errors='replace').read()
        open(f'{work}/test/parallel/{n}', 'w', encoding='utf-8').write(transform(src)[0])
    todo = [n for n in elig if n not in done]
    tid = tool_id(a.thaw)
    shimhash = hashlib.sha256(b''.join(open(f'{SHIM}/{f}', 'rb').read() for f in sorted(os.listdir(SHIM)))).hexdigest()
    bindir = f'{work}/bin'
    os.makedirs(bindir, exist_ok=True)
    print(f'eligible={len(elig)} done={len(done)} todo={len(todo)} jobs={a.jobs} thaw={tid}', file=sys.stderr, flush=True)
    t0 = time.time()
    with open(a.out, 'a') as out, ThreadPoolExecutor(a.jobs) as ex:
        for i, r in enumerate(ex.map(lambda n: run_one(n, a, tid, shimhash, bindir), todo), 1):
            out.write(json.dumps(r) + '\n')
            out.flush()
            if i % 50 == 0:
                print(f'{i}/{len(todo)} {(time.time() - t0) / 60:.1f}min', file=sys.stderr, flush=True)
    print(f'finished {len(todo)} tests in {(time.time() - t0) / 60:.1f}min', file=sys.stderr, flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('cmd', choices=['select', 'run'])
    ap.add_argument('--tree', default=os.environ.get('NODE_TESTS_DIR'))
    ap.add_argument('--work', default=os.environ.get('NODE_TESTS_WORK'))
    ap.add_argument('--thaw', default=os.environ.get('THAW'))
    ap.add_argument('--cache', default=os.environ.get('NODE_TESTS_CACHE', os.path.expanduser('~/.cache/thaw-node-tests')))
    ap.add_argument('--out')
    ap.add_argument('--prefix', default='')
    ap.add_argument('--files')
    ap.add_argument('--jobs', type=int, default=2)
    a = ap.parse_args()
    if not a.tree or not a.work:
        sys.exit('set --tree/NODE_TESTS_DIR and --work/NODE_TESTS_WORK')
    a.tree, a.work = os.path.abspath(a.tree), os.path.abspath(a.work)
    if a.cmd == 'run':
        if not a.thaw or not a.out:
            sys.exit('run needs --thaw/THAW and --out')
        a.thaw = os.path.abspath(a.thaw)
        cmd_run(a)
    else:
        cmd_select(a)


main()
