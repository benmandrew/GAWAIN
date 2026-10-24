#!/usr/bin/env python3
"""Summarise a bench/count_replay.sh directory ("Lasso count time" in
docs/dev/search.md).

For every dump replayed under each arm, compares the counts of the arms
with the first arm's, formula by formula: equal means the same printed
log2 count, so the same f64, or both failed. Prints one CSV row per dump and
arm that ran it, with the count time, the ltl2tgba time, the failures,
the slowest single count and the most steps of one count (`work`, when
the build reports it), and the formulas whose count differs from the
first arm's. The last lines total each arm.

usage: count_replay_stats.py DIR ARM...
"""
import csv
import re
import sys
from pathlib import Path


def counts(path):
    out = {}
    for line in open(path):
        log, _, formula = line.rstrip('\n').partition('\t')
        out[formula] = log
    return out


def summary(path):
    text = open(path).read()
    m = re.search(r'(\d+) calls, (\d+) formulas, (\d+) failures, count ([0-9.]+) s, '
                  r'ltl2tgba ([0-9.]+) s', text)
    if not m:
        return None
    slowest = max((float(t) for t in re.findall(r' ok=\w+ t=([0-9.]+)', text)), default=0.0)
    work = max((int(w) for w in re.findall(r' work=(\d+)', text)), default=None)
    return {'formulas': int(m[2]), 'failures': int(m[3]), 'count': float(m[4]),
            'ltl2tgba': float(m[5]), 'slowest': slowest, 'work': work}


def main():
    d, arms = Path(sys.argv[1]), sys.argv[2:]
    dumps = sorted({p.name.rsplit('.', 2)[0] for p in d.glob('*.err')})
    w = csv.writer(sys.stdout)
    w.writerow(['dump', 'arm', 'formulas', 'failures', 'count_s', 'ltl2tgba_s',
                'slowest_s', 'max_work', 'differ'])
    totals = {a: [0.0, 0.0, 0, 0, 0] for a in arms}
    for dump in dumps:
        base = d / f'{dump}.{arms[0]}.out'
        if not base.exists() or summary(d / f'{dump}.{arms[0]}.err') is None:
            continue
        ref = counts(base)
        for arm in arms:
            if not (d / f'{dump}.{arm}.err').exists():
                continue
            s = summary(d / f'{dump}.{arm}.err')
            if s is None:
                w.writerow([dump, arm, '', '', '', '', '', '', 'unfinished'])
                continue
            got = counts(d / f'{dump}.{arm}.out')
            differ = sorted(f for f in ref.keys() | got.keys() if ref.get(f) != got.get(f))
            w.writerow([dump, arm, s['formulas'], s['failures'], s['count'], s['ltl2tgba'],
                        s['slowest'], s['work'] if s['work'] is not None else '', len(differ)])
            for f in differ:
                print(f'#   {dump} {arm}: {ref.get(f)} -> {got.get(f)} for {f}', file=sys.stderr)
            t = totals[arm]
            t[0] += s['count']
            t[1] += s['ltl2tgba']
            t[2] += s['failures']
            t[3] += len(differ)
            t[4] += 1
    for arm, (count, tool, failures, differ, n) in totals.items():
        print(f'# {arm}: {n} dumps, count {count:.1f} s, ltl2tgba {tool:.1f} s, '
              f'{failures} failures, {differ} counts differ from {arms[0]}')


if __name__ == '__main__':
    main()
