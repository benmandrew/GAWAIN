#!/usr/bin/env python3
"""Found rate and cost of the two evaluation arms across search budgets.

Reads the `default` and `mutation` runs of each BUDGET=RUNS pair under DIR
(search_bench JSON lines from bench/graded_run.py) and keeps the instance
and seed pairs that gave a result at every budget under both arms, up to
seed --max-seed, so every row covers the same runs. Prints, per budget and
arm, the share of runs that found a repair and the median wall time and
back-end calls ("Budget" in docs/dev/evaluation.md).

usage: budget_curve.py DIR BUDGET=RUNS... [--max-seed 2]
"""
import argparse
import json
import re
import statistics
from pathlib import Path

ARMS = ('default', 'mutation')


def main():
    p = argparse.ArgumentParser()
    p.add_argument('dir', type=Path)
    p.add_argument('budgets', nargs='+')
    p.add_argument('--max-seed', type=int, default=2)
    a = p.parse_args()
    budgets = dict(b.split('=') for b in a.budgets)
    data = {}
    for b, runs in budgets.items():
        for f in (a.dir / runs).glob('*.json'):
            m = re.fullmatch(r'(.+)-(default|mutation)-(\d+)\.json', f.name)
            if not m or int(m[3]) > a.max_seed:
                continue
            r = json.loads(f.read_text().strip().splitlines()[-1])
            data[(b, m[2], m[1], int(m[3]))] = (bool(r['repairs']), r['wall'], r['backend_calls'])
    keys = {k[2:] for k in data}
    common = [k for k in keys if all((b, arm) + k in data for b in budgets for arm in ARMS)]
    print(f'{len(common)} instance and seed pairs with a result at every budget and arm')
    print('| Budget | Arm | Found rate | Median wall (s) | Median back-end calls |')
    print('|---|---|---|---|---|')
    for b in budgets:
        for arm in ARMS:
            v = [data[(b, arm) + k] for k in common]
            print(f'| {b} | {arm} | {sum(x[0] for x in v) / len(v):.3f} '
                  f'| {statistics.median(x[1] for x in v):.1f} '
                  f'| {statistics.median(x[2] for x in v):.0f} |')


if __name__ == '__main__':
    main()
