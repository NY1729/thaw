#!/usr/bin/env python3
"""Ratchet: compare run.py results against tools/node-tests/baseline.json + failing.txt.gz.

usage: python3 -I compare.py RESULTS.jsonl [more.jsonl ...] [--update-baseline] [--commit SHA]
Exit 1 if a test that is not in the baseline failing list now fails. Only the modules (test-<module>-*) present in
the results are compared, so a partial run can be checked on its own; --update-baseline rewrites those modules.
"""
import sys, os, json, gzip, subprocess, argparse, collections

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = os.path.join(HERE, 'baseline.json')
FAILING = os.path.join(HERE, 'failing.txt.gz')
BAD = ('build-fail', 'runtime-fail', 'crash', 'timeout')


def module(p):  # test-http-abort.js -> http
    return p.split('-')[1].split('.')[0]


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
    a = collections.defaultdict(lambda: collections.Counter())
    for p, r in res.items():
        a[module(p)][r['status']] += 1
    return {k: {'pass': v['pass'], 'fail': sum(v[s] for s in BAD), **{s: v[s] for s in BAD}} for k, v in sorted(a.items())}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('results', nargs='+')
    ap.add_argument('--update-baseline', action='store_true')
    ap.add_argument('--commit')
    a = ap.parse_args()
    res = load(a.results)
    base = json.load(open(BASE)) if os.path.exists(BASE) else {'modules': {}}
    old_fail = set(gzip.open(FAILING, 'rt').read().split()) if os.path.exists(FAILING) else set()
    mods = {module(p) for p in res}
    cov = {p for p in old_fail if module(p) in mods}
    regress = sorted(p for p, r in res.items() if r['status'] in BAD and p not in old_fail)
    improved = sorted(p for p in cov if res.get(p, {}).get('status') == 'pass')
    new = counts(res)
    tot = lambda d: {k: sum(v[k] for v in d.values()) for k in ('pass', 'fail')}
    print(f'## node own-tests ratchet\n\nmodules: {len(mods)}; this run: {tot(new)}')
    print(f'baseline for these modules: {tot({k: v for k, v in base["modules"].items() if k in mods})}')
    print(f'\n### regressions: {len(regress)}')
    for p in regress:
        print(f'- {p}: {res[p]["status"]} {res[p]["message"][:120]}')
    print(f'\n### improvements: {len(improved)}')
    for p in improved[:200]:
        print(f'- {p}')
    print('\n| module | pass | baseline pass | delta |\n|---|---|---|---|')
    for k, v in new.items():
        b = base['modules'].get(k, {}).get('pass', 0)
        if v['pass'] != b:
            print(f'| {k} | {v["pass"]} | {b} | {v["pass"] - b:+d} |')
    if a.update_baseline:
        commit = a.commit or subprocess.check_output(['git', '-C', HERE, 'rev-parse', 'HEAD'], text=True).strip()
        fail = {p for p in old_fail if module(p) not in mods} | {p for p, r in res.items() if r['status'] in BAD}
        base['modules'] = dict(sorted((base['modules'] | new).items()))
        base['commit'] = commit
        base['node'] = open(os.path.join(HERE, 'TEST_COMMIT')).read().strip()
        base['totals'] = tot(base['modules'])
        json.dump(base, open(BASE, 'w'), indent=1)
        with open(FAILING, 'wb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', mtime=0, filename='') as g:
            g.write(('\n'.join(sorted(fail)) + '\n').encode())
        print(f'\nbaseline updated: {base["totals"]} at {commit}')
        return 0
    return 1 if regress else 0


sys.exit(main())
