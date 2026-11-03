#!/usr/bin/env python3
"""Score the repairs of the rejected-side ablation on one common scale.

A run's `semantic` field holds the score of the side it searched on, so
`best_semantic` cannot be compared between `default` and `rejected`
("Rejected side" in docs/dev/search.md). This reads the runs of both arms
under DIR/RUNS, rescores every distinct repair with the `score` binary, and
gives each run that found a repair two numbers: its best closed share on
the accepted side, (best - top) / (1 - top) for ⊤'s score top, and its best
rejection strength. Per instance it takes the median over seeds, and per
measure it prints, over all instances and per family, the medians of both
arms, the median paired difference (rejected minus default), the Wilcoxon
p-value and the signs, over the instances where both arms found a repair.

Writes OUT (one row per instance and arm).

usage: side_stats.py DIR OUT [--runs runs-semantic] [--score PATH]
"""
import argparse
import csv
import json
import random
import re
import statistics
from collections import defaultdict
from pathlib import Path

from baselines_top import scores
from graded_stats import bootstrap_median, fmt, wilcoxon

ARMS = ('default', 'rejected')
SEED = 20261024


def main():
    p = argparse.ArgumentParser()
    p.add_argument('dir', type=Path)
    p.add_argument('out')
    p.add_argument('--runs', default='runs-semantic')
    p.add_argument('--score', default='target/release/score')
    a = p.parse_args()
    family = {r['name']: r['family']
              for r in csv.DictReader(open(a.dir / 'instances.tsv'), delimiter='\t')}
    runs = defaultdict(list)
    for f in sorted((a.dir / a.runs).glob('*.json')):
        m = re.fullmatch(r'(.+)-(default|rejected)-(\d+)\.json', f.name)
        runs[m[1], m[2]].append(json.loads(f.read_text().strip().splitlines()[-1]))
    names = sorted({n for n, _ in runs})
    phis = {n: runs[n, ARMS[0]][0]['phi'] for n in names}
    top = dict(zip(names, (round(s[0], 6) for s in
                           scores(a.score, [(phis[n], 'true') for n in names]))))
    rows = []
    for n in names:
        repairs = sorted({r['formula'] for arm in ARMS for run in runs[n, arm]
                          for r in run['repairs']})
        scored = dict(zip(repairs, scores(a.score, [(phis[n], f) for f in repairs]))) \
            if repairs else {}
        for arm in ARMS:
            found = [run for run in runs[n, arm] if run['repairs']]
            closed = [(max(scored[r['formula']][0] for r in run['repairs']) - top[n]) / (1 - top[n])
                      for run in found] if top[n] < 1 else []
            rejection = [max(scored[r['formula']][2] for r in run['repairs']) for run in found]
            rows.append({'instance': n, 'family': family[n], 'arm': arm,
                         'seeds': len(runs[n, arm]), 'found': len(found),
                         'closed_share': f'{statistics.median(closed):.6f}' if closed else '',
                         'rejection': f'{statistics.median(rejection):.6f}' if rejection else ''})
    with open(a.out, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    by = {(r['instance'], r['arm']): r for r in rows}
    rng = random.Random(SEED)
    for group in sorted(set(family[n] for n in names)) + ['all']:
        ns = [n for n in names if group in ('all', family[n])]
        print(f'== {group} ({len(ns)} instances)')
        for key in ('closed_share', 'rejection'):
            pairs = [(float(by[n, 'default'][key]), float(by[n, 'rejected'][key]))
                     for n in ns if by[n, 'default'][key] and by[n, 'rejected'][key]]
            if not pairs:
                continue
            d = [y - x for x, y in pairs]
            meds = []
            for xs in ([x for x, _ in pairs], [y for _, y in pairs], d):
                lo, hi = bootstrap_median(xs, rng)
                meds.append(f'{fmt(statistics.median(xs))} [{fmt(lo)}, {fmt(hi)}]')
            signs = f'{sum(x > 0 for x in d)}/{sum(x < 0 for x in d)}/{sum(x == 0 for x in d)}'
            print(f'{key:<13} {len(pairs):3} {meds[0]:>24} {meds[1]:>24} {meds[2]:>26} '
                  f'{wilcoxon(d):7.4f} {signs:>9}')


if __name__ == '__main__':
    main()
