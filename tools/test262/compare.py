#!/usr/bin/env python3
"""Ratchet: compare test262 results (run.py jsonl) against tools/test262/baseline.json + failing.txt.gz.

usage: python3 -I compare.py RESULTS.jsonl [more.jsonl ...] [--update-baseline] [--no-rerun] [--markdown]
Exit 1 if a test that is not in the baseline failing list now fails (after one rerun). Only the areas present in
the results are compared, so a shard can be checked on its own. --update-baseline rewrites those areas.
"""
import sys, os, json, gzip, subprocess, tempfile, argparse, collections

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = os.path.join(HERE, 'baseline.json')
FAILING = os.path.join(HERE, 'failing.txt.gz')
BAD = ('fail', 'crash', 'timeout')


def area(p):  # test/built-ins/Array/from/x.js -> built-ins/Array
    return '/'.join(p.split('/')[1:3])


def load(files):
    d = {}
    for f in files:
        for l in open(f, encoding='utf-8'):
            try:
                r = json.loads(l)
                d[r['path']] = r
            except Exception:
                pass
    return d


def counts(res):
    a = collections.defaultdict(lambda: dict(pass_=0, fail=0, skip=0))
    for p, r in res.items():
        k = {'pass': 'pass_', 'skip': 'skip'}.get(r['status'], 'fail')
        a[area(p)][k] += 1
    return {k: {'pass': v['pass_'], 'fail': v['fail'], 'skip': v['skip']} for k, v in sorted(a.items())}


def read_failing():
    if not os.path.exists(FAILING):
        return set()
    return set(gzip.open(FAILING, 'rt', encoding='utf-8').read().split())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('results', nargs='+')
    ap.add_argument('--update-baseline', action='store_true')
    ap.add_argument('--no-rerun', action='store_true')
    ap.add_argument('--commit', help='engine commit recorded in baseline.json (default: git HEAD)')
    a = ap.parse_args()
    res = load(a.results)
    base = json.load(open(BASE)) if os.path.exists(BASE) else {'areas': {}}
    old_fail = read_failing()
    areas = {area(p) for p in res}
    # Tests of the covered areas that failed in the baseline.
    old_fail_cov = {p for p in old_fail if area(p) in areas}

    regress = sorted(p for p, r in res.items() if r['status'] in BAD and p not in old_fail)
    if regress and not a.no_rerun and not a.update_baseline:
        lst = tempfile.NamedTemporaryFile('w', suffix='.txt', delete=False)
        lst.write('\n'.join(regress) + '\n'); lst.close()
        out = lst.name + '.jsonl'
        subprocess.run([sys.executable, '-I', os.path.join(HERE, 'run.py'), '--files', lst.name, '--out', out,
                        '--jobs', '2'], check=True)
        again = load([out])
        flaky = [p for p in regress if again.get(p, {}).get('status') == 'pass']
        for p in flaky:
            res[p] = again[p]
        regress = [p for p in regress if p not in flaky]
        os.unlink(lst.name); os.unlink(out)
        if flaky:
            print(f'(rerun cleared {len(flaky)} flaky test(s): {", ".join(flaky[:5])})')
    improved = sorted(p for p in old_fail_cov if res.get(p, {}).get('status') == 'pass')
    new = counts(res)
    tot = lambda d: {k: sum(v[k] for v in d.values()) for k in ('pass', 'fail', 'skip')}

    print(f'## test262 ratchet\n\ncovered areas: {len(areas)}; this run: {tot(new)}')
    print(f'baseline for these areas: {tot({k: v for k, v in base["areas"].items() if k in areas})}')
    print(f'\n### regressions: {len(regress)}')
    for p in regress:
        print(f'- {p}: {res[p]["status"]} {res[p]["message"][:120]}')
    print(f'\n### improvements: {len(improved)}')
    for p in improved[:200]:
        print(f'- {p}')
    if len(improved) > 200:
        print(f'- ... {len(improved) - 200} more')
    print('\n| area | pass | baseline pass | delta |\n|---|---|---|---|')
    for k, v in new.items():
        b = base['areas'].get(k, {}).get('pass', 0)
        if v['pass'] != b:
            print(f'| {k} | {v["pass"]} | {b} | {v["pass"] - b:+d} |')

    if a.update_baseline:
        commit = a.commit or subprocess.check_output(['git', '-C', HERE, 'rev-parse', 'HEAD'], text=True).strip()
        keep = {p for p in old_fail if area(p) not in areas}
        fail = keep | {p for p, r in res.items() if r['status'] in BAD}
        base['areas'] = {k: v for k, v in base['areas'].items() if k not in areas} | new
        base['areas'] = dict(sorted(base['areas'].items()))
        base['commit'] = commit
        base['test262'] = open(os.path.join(HERE, 'TEST262_COMMIT')).read().strip()
        base['totals'] = tot(base['areas'])
        json.dump(base, open(BASE, 'w'), indent=1)
        with open(FAILING, 'wb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', mtime=0, filename='') as g:
            g.write(('\n'.join(sorted(fail)) + '\n').encode())
        print(f'\nbaseline updated: {base["totals"]} at {commit}')
        return 0
    return 1 if regress else 0


sys.exit(main())
