#!/usr/bin/env python3
"""Three-way parity classifier: node (oracle) vs QuickJS vs thaw-js, per `@@ <name>` probe block.

Inputs are three matrix.txt files in diff.py --blocks format. Values are compared after diff.py's
block_normalise (same masks, same THROW rule). A probe a side did not print is value '<no line>' and
never equals node. Two sides that both lack a probe count as equal to each other.

Classes (n = node, q = QuickJS, t = thaw-js):
  SAME-AS-NODE        q == n and t == n
  THAWJS-REGRESSION   q == n, t != n   (thaw-js differs where QuickJS matches node)
  QJS-ONLY-BAD        t == n, q != n   (QuickJS is the outlier, thaw-js matches node)
  BOTH-DIFFER-SAME    q != n, t != n, q == t   (a difference QuickJS also had)
  BOTH-DIFFER         q != n, t != n, q != t
  NODE-MISSING        probe not in node output: not classified (node is the oracle)

Output: --out gets one row per probe: `probe | node | qjs | thaw-js | class` (values flattened, cut
to 60 chars). stdout gets the class counts and the counts per family (prefix before the first '.';
F3/F5/F6 are named, the rest is `other`). Exit 0 always, except 2 = harness error (missing input).
"""
import argparse, collections, os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from diff import load_blocks, block_normalise  # noqa: E402

CLASSES = ['SAME-AS-NODE', 'THAWJS-REGRESSION', 'QJS-ONLY-BAD', 'BOTH-DIFFER-SAME', 'BOTH-DIFFER']
FAMILIES = ['F3', 'F5', 'F6', 'other']


def classify(n, q, t):
    qm = q is not None and q == n
    tm = t is not None and t == n
    if qm and tm:
        return 'SAME-AS-NODE'
    if qm:
        return 'THAWJS-REGRESSION'
    if tm:
        return 'QJS-ONLY-BAD'
    if q == t:
        return 'BOTH-DIFFER-SAME'
    return 'BOTH-DIFFER'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--node', required=True)
    ap.add_argument('--qjs', required=True)
    ap.add_argument('--thaw', required=True)
    ap.add_argument('--out', help='write the per-probe table here')
    args = ap.parse_args()
    for path in (args.node, args.qjs, args.thaw):
        if not os.path.isfile(path):
            sys.stderr.write(f'harness error: missing {path}\n')
            return 2

    node, qjs, thaw = (load_blocks(p) for p in (args.node, args.qjs, args.thaw))
    names = list(dict.fromkeys(list(node) + list(qjs) + list(thaw)))

    def value(side, name):
        return block_normalise(side[name]) if name in side else None

    def cell(v):
        return '<no line>' if v is None else v.replace('\n', '\\n')[:60]

    counts = collections.Counter()
    fam = {f: collections.Counter() for f in FAMILIES}
    rows = []
    for name in names:
        n, q, t = value(node, name), value(qjs, name), value(thaw, name)
        if n is None:
            cls = 'NODE-MISSING'
        else:
            cls = classify(n, q, t)
            counts[cls] += 1
            prefix = name.split('.', 1)[0]
            fam[prefix if prefix in FAMILIES else 'other'][cls] += 1
        rows.append(f'{name} | {cell(n)} | {cell(q)} | {cell(t)} | {cls}')

    classified = sum(counts.values())
    print(f'probes: {classified} classified, {len(names) - classified} node-missing (not classified)')
    for cls in CLASSES:
        print(f'{cls:<20}{counts[cls]:>6}')
    print()
    print(f'{"family":<8}{"probes":>7}' + ''.join(f'{c:>19}' for c in CLASSES))
    for f in FAMILIES:
        total = sum(fam[f].values())
        print(f'{f:<8}{total:>7}' + ''.join(f'{fam[f][c]:>19}' for c in CLASSES))

    if args.out:
        with open(args.out, 'w', encoding='utf-8') as fh:
            fh.write('probe | node | qjs | thaw-js | class\n')
            fh.write('\n'.join(rows) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
