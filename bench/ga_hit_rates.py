#!/usr/bin/env python3
"""Oracle cache hit rates on real GA populations.

`docs/dev/oracle-measurements.md` measured the hit rates of tiers 0 to 2
on `ltlgrind` mutants. This script reads the same counters from
search_bench JSON lines, which record every query of a whole search and
its output gate, and prints the share answered by each tier per family.

Inputs:

- RUNS: a directory of search_bench JSON lines named
  <instance>-binary-<seed>.json (`bench/data/baselines/top-runs`).
- CSV: bench/results/graded-runs.csv, for the family of each instance.

Writes OUT, one row per run, and prints a pooled row per family and the
median per-run hit rate.

`--arm` reads another arm, such as the evaluation's `default`.

usage: ga_hit_rates.py RUNS CSV OUT [--arm binary]
"""
import argparse
import csv
import json
import re
import statistics
from collections import defaultdict
from pathlib import Path

TIERS = ('memo_hits', 'lasso_hits', 'implication_hits', 'backend_calls')


def main():
    p = argparse.ArgumentParser()
    p.add_argument('runs')
    p.add_argument('csv')
    p.add_argument('out')
    p.add_argument('--arm', default='binary')
    a = p.parse_args()
    family = {r['instance']: r['family'] for r in csv.DictReader(open(a.csv))}
    rows = []
    for path in sorted(Path(a.runs).glob(f'*-{a.arm}-*.json')):
        m = re.fullmatch(rf'(.+)-{re.escape(a.arm)}-(\d+)\.json', path.name)
        run = json.loads(path.read_text().strip().splitlines()[-1])
        if sum(run[t] for t in TIERS) != run['queries']:
            raise SystemExit(f'{path}: the tiers do not sum to the queries')
        rows.append({'instance': m[1], 'family': family[m[1]], 'seed': int(m[2]),
                     'queries': run['queries'], **{t: run[t] for t in TIERS},
                     'backend_unknowns': run['backend_unknowns'],
                     'hit_rate': f"{1 - run['backend_calls'] / run['queries']:.4f}"})
    with open(a.out, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    groups = defaultdict(list)
    for r in rows:
        groups[r['family']].append(r)
        groups['all'].append(r)
    print('| Family | Runs | Queries | Memo | Lasso | Implication | Back end '
          '| Cache hit rate | Median per run | Unknown |')
    print('|---|---|---|---|---|---|---|---|---|---|')
    for name in sorted(groups, key=lambda g: (g == 'all', g)):
        rs = groups[name]
        q = sum(r['queries'] for r in rs)
        share = [f"{100 * sum(r[t] for r in rs) / q:.1f}%" for t in TIERS]
        hit = 100 * (1 - sum(r['backend_calls'] for r in rs) / q)
        med = 100 * statistics.median(float(r['hit_rate']) for r in rs)
        unknown = sum(r['backend_unknowns'] for r in rs)
        print(f"| {name} | {len(rs)} | {q} | {' | '.join(share)} | {hit:.1f}% | {med:.1f}% "
              f"| {unknown} |")


if __name__ == '__main__':
    main()
