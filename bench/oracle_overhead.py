#!/usr/bin/env python3
"""Split oracle_bench query time into back-end time and the rest.

usage: oracle_overhead.py CSV...

The rest is the time a query spends outside `Backend::check`: the memo, the lasso cache,
the fingerprints and every `ltlfilt` exec of tier 2 and of eviction. For each back end
(benches named rers* ran SPIN, lmcs* nuXmv) it prints the total query time, the share
outside the back end, the `ltlfilt` execs, and the median and mean time of a query that
tier 2 answered, which is one or two execs plus the in-process tiers.
"""
import csv, statistics, sys
from collections import defaultdict


def main(paths):
    groups = defaultdict(list)
    for path in paths:
        with open(path, newline='') as fh:
            for r in csv.DictReader(fh):
                groups['SPIN' if r['bench'].startswith('rers') else 'nuXmv'].append(r)
    print('| Back end | Queries | Total query (s) | Outside the back end (s) | Share | ltlfilt execs '
          '| Tier 2 answer, median (ms) | Tier 2 answer, mean (ms) |')
    print('|' + '---|' * 8)
    for name, rows in sorted(groups.items()):
        total = sum(float(r['query_s']) for r in rows)
        outside = sum(float(r['query_s']) - float(r['backend_s'] or 0) for r in rows)
        execs = sum(int(r['spot_calls']) for r in rows)
        hits = [1000 * float(r['query_s']) for r in rows if r['tier'] == 'implication']
        print(f'| {name} | {len(rows)} | {total:.0f} | {outside:.1f} | {100 * outside / total:.2f}% | '
              f'{execs} | {statistics.median(hits):.1f} | {statistics.mean(hits):.1f} |')


if __name__ == '__main__':
    main(sys.argv[1:])
