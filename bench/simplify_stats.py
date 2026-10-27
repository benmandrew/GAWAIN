#!/usr/bin/env python3
"""Measure how much Spot's simplifier shortens GAWAIN's repairs.

The case studies (bench/notes/case-studies.md) found repairs that carry
unsimplified subterms such as `a_p -> !a_p`. This reads the repairs of
every run JSON under the given directories, simplifies each distinct one
with `ltlfilt -r` (Spot's highest level, equivalence-preserving), checks
the equivalence both ways with Spot, and reports the formula size (`%s`,
the number of nodes) before and after, per family and overall.

usage: simplify_stats.py OUT RUN_DIR... [--instances TSV ...]
"""
import argparse
import csv
import json
import statistics
import subprocess
from collections import defaultdict
from pathlib import Path

from graded_stats import implications


def spot(formulas, *flags):
    r = subprocess.run(['ltlfilt', *flags, '-F', '-'], input=''.join(f + '\n' for f in formulas),
                       capture_output=True, text=True, check=True)
    out = r.stdout.splitlines()
    if len(out) != len(formulas):
        raise SystemExit(f'ltlfilt returned {len(out)} lines for {len(formulas)} formulas')
    return out


def main():
    p = argparse.ArgumentParser()
    p.add_argument('out')
    p.add_argument('dirs', nargs='+', type=Path)
    a = p.parse_args()
    family = {}
    repairs = defaultdict(set)
    for d in a.dirs:
        tsv = d.parent / 'instances.tsv'
        fam = {r['name']: r['family'] for r in csv.DictReader(open(tsv), delimiter='\t')}
        for f in d.glob('*.json'):
            run = json.loads(f.read_text().strip().splitlines()[-1])
            name = next(n for n in fam if f.name.startswith(n + '-'))
            for r in run['repairs']:
                repairs[r['formula']].add(name)
                family[r['formula']] = fam[name]
    formulas = sorted(repairs)
    simple = spot(formulas, '-r', '--format=%f')
    before = [int(s) for s in spot(formulas, '--format=%s')]
    after = [int(s) for s in spot(simple, '--format=%s')]
    pairs = [(x, y) for f, s in zip(formulas, simple) for x, y in ((f, s), (s, f))]
    imp = implications(pairs)
    bad = [f for i, f in enumerate(formulas) if not (imp[2 * i] and imp[2 * i + 1])]
    if bad:
        raise SystemExit(f'{len(bad)} simplifications not proved equivalent, e.g. {bad[0]}')
    with open(a.out, 'w', newline='') as f:
        w = csv.writer(f)
        w.writerow(['family', 'size', 'simplified_size', 'formula', 'simplified'])
        for row in zip(formulas, simple, before, after):
            w.writerow([family[row[0]], row[2], row[3], row[0], row[1]])
    groups = defaultdict(list)
    for f, b, s in zip(formulas, before, after):
        groups[family[f]].append((b, s))
        groups['all'].append((b, s))
    for g, xs in sorted(groups.items(), key=lambda kv: kv[0] == 'all'):
        bs, ss = [b for b, _ in xs], [s for _, s in xs]
        print(f'{g}: {len(xs)} repairs, median size {statistics.median(bs)} -> '
              f'{statistics.median(ss)}, {sum(s < b for b, s in xs)} shorter, '
              f'total {sum(bs)} -> {sum(ss)} ({1 - sum(ss) / sum(bs):.1%} fewer nodes)')


if __name__ == '__main__':
    main()
