#!/usr/bin/env python3
"""Differential parity diff: node (oracle) output vs thaw-js output, per probe.

Inputs are two directories holding one `<corpus>.txt` per corpus. Each probe is a
line `name = value`; a line without ` = ` is a raw probe keyed by the whole line.

Gaps:
  MISSING    probe in node output, absent from thaw output
  DIFFERENT  present in both, normalised values differ
  EXTRA      present only in thaw output (informational, never gates)

Gate: exits 1 only when a gap id is NOT in --baseline (gaps may only shrink).
--write-baseline records the current gap ids into --baseline and exits 0.
Exit 2 = harness error (missing node output, duplicate probe name).
"""
import argparse, glob, json, os, re, sys

NORMALISERS = [
    (re.compile(r'^THROW (\S+?)(: .*)?$'), r'THROW \1'),   # messages differ by engine; keep the error name
    (re.compile(r'/home/[^\s\'"]*'), '<path>'),
    (re.compile(r'\b0x[0-9a-fA-F]+\b'), '<addr>'),
    (re.compile(r'\b\d+(\.\d+)?ms\b'), '<ms>'),
]

def normalise(value):
    v = ' '.join(value.split())
    for rx, rep in NORMALISERS:
        v = rx.sub(rep, v)
    return v

def load(path):
    probes = {}
    with open(path, encoding='utf-8', errors='replace') as fh:
        for raw in fh:
            line = raw.rstrip('\n')
            if not line.strip():
                continue
            if ' = ' in line:
                name, value = line.split(' = ', 1)
            else:
                name, value = line, ''
            if name in probes:
                sys.stderr.write(f'harness error: duplicate probe {name!r} in {path}\n')
                sys.exit(2)
            probes[name] = value
    return probes

def load_skips(path):
    if not path or not os.path.exists(path):
        return []
    pats = []
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if line and not line.startswith('#'):
                pats.append(re.compile(line))
    return pats

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--node-dir', required=True)
    ap.add_argument('--thaw-dir', required=True)
    ap.add_argument('--baseline')
    ap.add_argument('--write-baseline', action='store_true')
    ap.add_argument('--gaps', help='where to write the gaps.json report')
    ap.add_argument('--skip', help='file of regexes; matching probe names are dropped from both sides')
    args = ap.parse_args()

    skips = load_skips(args.skip)
    node_files = sorted(glob.glob(os.path.join(args.node_dir, '*.txt')))
    if not node_files:
        sys.stderr.write(f'harness error: no node outputs in {args.node_dir}\n')
        return 2

    gaps, counts, skipped = [], {}, 0
    for npath in node_files:
        corpus = os.path.basename(npath)[:-4]
        node = load(npath)
        tpath = os.path.join(args.thaw_dir, corpus + '.txt')
        thaw = load(tpath) if os.path.exists(tpath) else {}
        c = {'probes': 0, 'MISSING': 0, 'DIFFERENT': 0, 'EXTRA': 0}
        for name in node:
            if any(p.search(name) for p in skips):
                skipped += 1
                continue
            c['probes'] += 1
            if name not in thaw:
                kind, t = 'MISSING', '<no line>'
            elif normalise(thaw[name]) != normalise(node[name]):
                kind, t = 'DIFFERENT', thaw[name]
            else:
                continue
            c[kind] += 1
            gaps.append({'id': f'{corpus}::{name}', 'corpus': corpus, 'probe': name, 'kind': kind,
                         'node': node[name], 'thaw': t})
        for name in thaw:
            if name not in node and not any(p.search(name) for p in skips):
                c['EXTRA'] += 1
        counts[corpus] = c

    current = {g['id']: g for g in gaps}
    baseline_ids = set()
    if args.baseline and os.path.exists(args.baseline) and not args.write_baseline:
        with open(args.baseline) as fh:
            baseline_ids = {g['id'] for g in json.load(fh)['gaps']}
    new = sorted(set(current) - baseline_ids)
    fixed = sorted(baseline_ids - set(current))

    print(f'{"corpus":<18}{"probes":>8}{"MISSING":>9}{"DIFFERENT":>11}{"EXTRA":>7}')
    for corpus, c in counts.items():
        print(f'{corpus:<18}{c["probes"]:>8}{c["MISSING"]:>9}{c["DIFFERENT"]:>11}{c["EXTRA"]:>7}')
    print(f'skipped by {os.path.basename(args.skip) if args.skip else "-"}: {skipped}')
    print()
    print(f'{"status":<8}{"kind":<11}{"probe":<60}{"node":<40}thaw')
    for g in gaps:
        status = 'known' if g['id'] in baseline_ids else 'NEW'
        if args.write_baseline:
            status = '-'
        print(f'{status:<8}{g["kind"]:<11}{g["id"][:58]:<60}{g["node"][:38]:<40}{g["thaw"][:60]}')
    print()
    print(f'gaps: {len(gaps)}  baseline: {len(baseline_ids)}  new: {len(new)}  fixed since baseline: {len(fixed)}')
    if fixed:
        print('fixed (rerun with --write-baseline to ratchet): ' + ', '.join(fixed[:20]) + (' ...' if len(fixed) > 20 else ''))

    if args.gaps:
        with open(args.gaps, 'w') as fh:
            json.dump({'counts': counts, 'gaps': gaps}, fh, indent=1, ensure_ascii=False)
            fh.write('\n')

    if args.write_baseline:
        if not args.baseline:
            sys.stderr.write('harness error: --write-baseline needs --baseline\n')
            return 2
        with open(args.baseline, 'w') as fh:
            json.dump({'note': 'gaps may only shrink; regenerate with --write-baseline only after a fix',
                       'gaps': [{'id': g['id'], 'kind': g['kind']} for g in gaps]}, fh, indent=1)
            fh.write('\n')
        print(f'wrote baseline {args.baseline} with {len(gaps)} gaps')
        return 0

    if new:
        print('NEW gaps not in baseline (fail):')
        for i in new:
            print('  ' + i)
        return 1
    return 0

if __name__ == '__main__':
    sys.exit(main())
