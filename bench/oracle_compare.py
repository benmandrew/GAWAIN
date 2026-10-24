#!/usr/bin/env python3
"""Compare two oracle_bench runs over the same candidate streams.

usage: oracle_compare.py BEFORE_CSV AFTER_CSV

Rows are matched on (bench, property, seq). For each match it checks that the candidate
is the same, and that the verdicts agree wherever both runs decided (holds or fails).
Then, per group (each bench, and each back end pooled), it prints the queries, the share
answered by each tier before and after, the median and mean query time, the total query
time, the back-end calls and their total time, and the queries that spent 1 s or more
outside the back end. Exits 1 on any candidate or verdict mismatch.
"""
import csv, statistics, sys
from collections import defaultdict

TIERS = ['memo', 'lasso', 'implication', 'backend']


def load(path):
    with open(path, newline='') as fh:
        return {(r['bench'], r['property'], int(r['seq'])): r for r in csv.DictReader(fh)}


def pooled(bench):
    return 'SPIN pooled' if bench.startswith('rers') else 'nuXmv pooled'


def stats(rows):
    q = [float(r['query_s']) for r in rows]
    b = [float(r['backend_s']) for r in rows if r['backend_s']]
    overhead = sum(1 for r in rows if float(r['query_s']) - float(r['backend_s'] or 0) >= 1)
    tiers = {t: sum(r['tier'] == t for r in rows) / len(rows) for t in TIERS}
    return tiers, statistics.median(q), statistics.mean(q), sum(q), len(b), sum(b), overhead


def main(before_path, after_path):
    before, after = load(before_path), load(after_path)
    common = sorted(before.keys() & after.keys())
    bad = 0
    decided = agree = 0
    for key in common:
        old, new = before[key], after[key]
        if old['candidate'] != new['candidate']:
            print('candidate differs', key, old['candidate'], new['candidate'])
            bad += 1
            continue
        if old['verdict'] != 'unknown' and new['verdict'] != 'unknown':
            decided += 1
            if old['verdict'] == new['verdict']:
                agree += 1
            else:
                print('verdict differs', key, old['verdict'], new['verdict'], new['candidate'])
                bad += 1
    print(f'{len(before)} rows before, {len(after)} after, {len(common)} matched; '
          f'{agree} of {decided} decided both times agree; {bad} mismatches')
    unknown = {name: sum(r['verdict'] == 'unknown' for r in rows.values()) for name, rows in
               (('before', before), ('after', after))}
    print(f'unknown verdicts: {unknown}')
    groups = defaultdict(lambda: ([], []))
    for key in common:
        for side, rows in ((0, before), (1, after)):
            groups[key[0]][side].append(rows[key])
            groups[pooled(key[0])][side].append(rows[key])
    print('| Group | Run | Queries | Memo | Lasso | Implication | Back end | Median query (s) '
          '| Mean query (s) | Total query (s) | Back-end calls | Back-end total (s) | Queries with ≥1 s outside the back end |')
    print('|' + '---|' * 13)
    for name in sorted(groups, key=lambda g: (g.endswith('pooled'), g)):
        for side, label in ((0, 'before'), (1, 'after')):
            rows = groups[name][side]
            tiers, med, mean, total, n_b, t_b, overhead = stats(rows)
            shares = ' | '.join(f'{100 * tiers[t]:.1f}%' for t in TIERS)
            print(f'| {name} | {label} | {len(rows)} | {shares} | {med:.3g} | {mean:.3g} | '
                  f'{total:.0f} | {n_b} | {t_b:.0f} | {overhead} |')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(*sys.argv[1:]))
