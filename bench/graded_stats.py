#!/usr/bin/env python3
"""Summarise the binary-against-graded ablation of bench/graded_run.py.

Reads DIR/instances.tsv and DIR/runs/*.json (one search_bench line each) and
writes three CSVs into OUT:

- graded-runs.csv: one row per run (instance, family, arm, seed, found, the
  number of repairs, the best semantic similarity, wall time, back-end
  calls and the grading counters).
- graded-instances.csv: one row per instance and arm, with the found-repair
  rate over seeds and the medians of the other measures.
- graded-strength.csv: one row per instance and arm against binary, with
  the sizes of the two pooled maximal sets and the net subsumption share.

It then prints, for each arm against binary, over all instances and per
family: the median of each measure (bootstrap 95% CI), the median paired
difference (bootstrap 95% CI) and the Wilcoxon signed-rank p-value.

Strength follows PEREDUR. Each arm's repairs of an instance are pooled over
its seeds and reduced to a maximal set: the strongest repairs, those that no
other pooled repair strictly implies, one per equivalence class. The net
subsumption share of A against B is the share of B's maximal set strictly
implied by some repair in A's, minus the converse; it is positive when A's
repairs are the stronger. Implication is plain LTL implication, decided by
`ltlfilt --satisfiable` (Spot 2.15.1 on PATH). The share is defined only
where both arms found a repair on some seed; the found-repair rate covers
the rest.

usage: graded_stats.py DIR OUT [--baseline binary]
"""
import csv
import json
import math
import random
import statistics
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

BOOT = 10000
SEED = 20261024


def implications(pairs):
    """For each (x, y), whether x implies y, in one ltlfilt call."""
    if not pairs:
        return []
    text = ''.join(f'({x}) & !({y})\n' for x, y in pairs)
    r = subprocess.run(['ltlfilt', '-F', '-', '-v', '--satisfiable', '--format=%L'],
                       input=text, capture_output=True, text=True)
    # ltlfilt exits 1 when it prints nothing, that is when no pair implies.
    if r.returncode not in (0, 1) or r.stderr:
        raise RuntimeError(f'ltlfilt failed: {r.stderr}')
    unsat = {int(line) for line in r.stdout.split()}
    return [i + 1 in unsat for i in range(len(pairs))]


def maximal(formulas, imp):
    """The strongest formulas: none strictly implied by another, one per class."""
    keep = []
    for f in formulas:
        if any(imp[(g, f)] and (not imp[(f, g)] or g in keep) for g in formulas if g != f):
            continue
        keep.append(f)
    return keep


def net_share(a, b, imp):
    """Share of b strictly implied by some of a, minus the converse."""
    def strict(x, y):
        return x != y and imp[(x, y)] and not imp[(y, x)]
    if not a or not b:
        return None
    ab = sum(any(strict(x, y) for x in a) for y in b) / len(b)
    ba = sum(any(strict(y, x) for y in b) for x in a) / len(a)
    return ab - ba


def strength(pools, arms, baseline):
    """Per (instance, arm): the maximal set and its net share against baseline."""
    rows = []
    for name, by_arm in sorted(pools.items()):
        formulas = sorted({f for arm in arms for f in by_arm.get(arm, ())})
        pairs = [(x, y) for x in formulas for y in formulas if x != y]
        imp = dict(zip(pairs, implications(pairs)))
        sets = {arm: maximal(sorted(set(by_arm.get(arm, ()))), imp) for arm in arms}
        for arm in arms:
            if arm == baseline:
                continue
            rows.append({'instance': name, 'arm': arm, 'maximal': len(sets[arm]),
                         'baseline_maximal': len(sets[baseline]),
                         'net_share': net_share(sets[arm], sets[baseline], imp)})
    return rows


def bootstrap_median(xs, rng):
    if not xs:
        return None, None
    ms = sorted(statistics.median(rng.choices(xs, k=len(xs))) for _ in range(BOOT))
    return ms[int(0.025 * BOOT)], ms[int(0.975 * BOOT) - 1]


def wilcoxon(ds):
    """Two-sided Wilcoxon signed-rank p-value, zeros dropped; exact without ties."""
    ds = [d for d in ds if d != 0]
    n = len(ds)
    if n == 0:
        return 1.0
    order = sorted(range(n), key=lambda i: abs(ds[i]))
    ranks = [0.0] * n
    i = 0
    while i < n:
        j = i
        while j + 1 < n and abs(ds[order[j + 1]]) == abs(ds[order[i]]):
            j += 1
        for k in range(i, j + 1):
            ranks[order[k]] = (i + j) / 2 + 1
        i = j + 1
    w = sum(r for r, d in zip(ranks, ds) if d > 0)
    ties = len(set(abs(d) for d in ds)) < n
    if not ties and n <= 60:
        top = n * (n + 1) // 2
        counts = [1] + [0] * top
        for r in range(1, n + 1):
            for s in range(top, r - 1, -1):
                counts[s] += counts[s - r]
        total = 2 ** n
        lo = sum(counts[:int(w) + 1]) / total
        hi = sum(counts[int(w):]) / total
        return min(1.0, 2 * min(lo, hi))
    mean = n * (n + 1) / 4
    groups = defaultdict(int)
    for r in ranks:
        groups[r] += 1
    var = n * (n + 1) * (2 * n + 1) / 24 - sum(t ** 3 - t for t in groups.values()) / 48
    z = (abs(w - mean) - 0.5) / math.sqrt(var) if var > 0 else 0.0
    return min(1.0, math.erfc(z / math.sqrt(2)))


def fmt(x, digits=3):
    if x is None:
        return '-'
    return f'{x:.{digits}f}'


def report(title, instances, arms, baseline, shares, rng):
    """Print one block: per measure, the arm medians and the paired test."""
    print(f'\n## {title} ({len(instances)} instances)')
    measures = [('found rate', 'found_rate'), ('best semantic', 'best_semantic'),
                ('wall (s)', 'wall'), ('back-end calls', 'backend_calls')]
    for arm in arms:
        if arm == baseline:
            continue
        print(f'\n{arm} against {baseline}')
        print(f'{"measure":16} {baseline:>24} {arm:>24} {"difference":>24} {"p":>7} {"+/-/0":>9}')
        for label, key in measures:
            pairs = [(rows[baseline][key], rows[arm][key]) for rows in instances.values()
                     if rows[baseline][key] is not None and rows[arm][key] is not None]
            if not pairs:
                continue
            b = [p[0] for p in pairs]
            a = [p[1] for p in pairs]
            d = [y - x for x, y in pairs]
            cells = []
            for xs in (b, a, d):
                lo, hi = bootstrap_median(xs, rng)
                cells.append(f'{fmt(statistics.median(xs))} [{fmt(lo)}, {fmt(hi)}]')
            signs = f'{sum(x > 0 for x in d)}/{sum(x < 0 for x in d)}/{sum(x == 0 for x in d)}'
            print(f'{label:16} {cells[0]:>24} {cells[1]:>24} {cells[2]:>24} '
                  f'{wilcoxon(d):7.4f} {signs:>9}')
        s = [r['net_share'] for r in shares if r['arm'] == arm and r['instance'] in instances
             and r['net_share'] is not None]
        if s:
            lo, hi = bootstrap_median(s, rng)
            signs = f'{sum(x > 0 for x in s)}/{sum(x < 0 for x in s)}/{sum(x == 0 for x in s)}'
            print(f'{"net subsumption":16} {"":>24} {"":>24} '
                  f'{fmt(statistics.median(s)) + f" [{fmt(lo)}, {fmt(hi)}]":>24} '
                  f'{wilcoxon(s):7.4f} {signs:>9}  mean {fmt(statistics.mean(s))}')


def main(argv):
    d, out = Path(argv[0]), Path(argv[1])
    baseline = argv[argv.index('--baseline') + 1] if '--baseline' in argv else 'binary'
    family = {r['name']: r['family']
              for r in csv.DictReader(open(d / 'instances.tsv'), delimiter='\t')}
    runs = []
    pools = defaultdict(lambda: defaultdict(list))
    for path in sorted((d / 'runs').glob('*.json')):
        r = json.loads(path.read_text())
        name, arm, seed = path.stem.rsplit('-', 2)
        repairs = r['repairs']
        runs.append({'instance': name, 'family': family[name], 'arm': arm, 'seed': int(seed),
                     'found': int(bool(repairs)), 'repairs': len(repairs),
                     'best_semantic': max((x['semantic'] for x in repairs), default=None),
                     'wall': r['wall'], 'search_wall': r['search_wall'],
                     'backend_calls': r['backend_calls'],
                     'backend_unknowns': r['backend_unknowns'], 'holding': r['holding'],
                     'lassos': r['lassos'], 'refuting': r['refuting'],
                     'grade_time': r['grade_time'], 'gate_errors': len(r['gate_errors'])})
        pools[name][arm].extend(x['formula'] for x in repairs)
    errors = sorted(p.stem for p in (d / 'runs').glob('*.err'))
    arms = sorted({r['arm'] for r in runs}, key=lambda a: (a != baseline, a))

    by = defaultdict(list)
    for r in runs:
        by[(r['instance'], r['arm'])].append(r)
    instances = defaultdict(dict)
    rows = []
    for (name, arm), rs in sorted(by.items()):
        sem = [r['best_semantic'] for r in rs if r['best_semantic'] is not None]
        row = {'instance': name, 'family': family[name], 'arm': arm, 'runs': len(rs),
               'found_rate': sum(r['found'] for r in rs) / len(rs),
               'best_semantic': statistics.median(sem) if sem else None,
               'repairs': statistics.median(r['repairs'] for r in rs),
               'wall': statistics.median(r['wall'] for r in rs),
               'backend_calls': statistics.median(r['backend_calls'] for r in rs),
               'backend_unknowns': statistics.median(r['backend_unknowns'] for r in rs),
               'refuting': statistics.median(r['refuting'] for r in rs),
               'grade_share': statistics.median(r['grade_time'] / r['search_wall']
                                                for r in rs if r['search_wall'] > 0)}
        rows.append(row)
        instances[name][arm] = row
    # Keep only instances with every arm, so every comparison is paired.
    complete = {n: v for n, v in instances.items() if all(a in v for a in arms)}
    shares = strength({n: pools[n] for n in complete}, arms, baseline)

    out.mkdir(parents=True, exist_ok=True)
    for fname, data in (('graded-runs.csv', runs), ('graded-instances.csv', rows),
                        ('graded-strength.csv', shares)):
        with open(out / fname, 'w', newline='') as f:
            w = csv.DictWriter(f, fieldnames=list(data[0]))
            w.writeheader()
            w.writerows(data)

    print(f'{len(runs)} runs, {len(errors)} without a result, '
          f'{len(complete)} of {len(instances)} instances with every arm')
    for e in errors:
        print(f'  no result: {e}')
    rng = random.Random(SEED)
    report('all', complete, arms, baseline, shares, rng)
    for fam in sorted(set(family[n] for n in complete)):
        report(fam, {n: v for n, v in complete.items() if family[n] == fam}, arms, baseline,
               shares, rng)
    graded = [r for r in runs if r['arm'] != baseline]
    if graded:
        share = [r['grade_time'] / r['search_wall'] for r in graded if r['search_wall'] > 0]
        print(f'\nrescoring time / search wall over {len(share)} non-baseline runs: '
              f'median {statistics.median(share):.5f}, max {max(share):.5f}')
        print(f'refuting lassos per run: median {statistics.median(r["refuting"] for r in graded)}, '
              f'max {max(r["refuting"] for r in graded)}')


if __name__ == '__main__':
    main(sys.argv[1:])
