#!/usr/bin/env python3
"""Summarise the status ablation of bench/graded_run.py.

Reads DIR/instances.tsv and DIR/runs/*.json (one search_bench line each) and
writes two CSVs into OUT:

- graded-runs.csv: one row per run (instance, family, arm, seed, found, the
  number of repairs, the best semantic similarity, wall time, back-end
  calls and the grading counters).
- graded-instances.csv: one row per instance and arm against the baseline,
  over the seeds both have run: the found-repair rate and the medians of
  the other measures for both, the sizes of the two pooled maximal sets and
  the net subsumption share.

It then prints, for each arm against the baseline, over all instances and
per family: the median of each measure for both arms (bootstrap 95% CI), the
median paired difference (bootstrap 95% CI), the Wilcoxon signed-rank
p-value, and the number of instances where the arm is higher, lower and
equal.

Strength follows PEREDUR. Each arm's repairs of an instance are pooled over
its seeds and reduced to a maximal set: the strongest repairs, those that no
other pooled repair strictly implies, one per equivalence class. The net
subsumption share of A against B is the share of B's maximal set strictly
implied by some repair in A's, minus the converse; it is positive when A's
repairs are the stronger. Implication is plain LTL implication, decided by
`ltlfilt --satisfiable` (Spot 2.15.1 on PATH). The share is defined only
where both arms found a repair on some seed; the found-repair rate covers
the rest.

usage: graded_stats.py DIR OUT [--baseline binary] [--runs runs]
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
MEASURES = [('found rate', 'found_rate'), ('best semantic', 'best_semantic'),
            ('repairs', 'repairs'), ('holding', 'holding'), ('wall (s)', 'wall'),
            ('back-end calls', 'backend_calls')]


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


def strength(a, b):
    """The sizes of the maximal sets of repair lists a and b, and a's net share."""
    formulas = sorted(set(a) | set(b))
    pairs = [(x, y) for x in formulas for y in formulas if x != y]
    imp = dict(zip(pairs, implications(pairs)))
    ma, mb = maximal(sorted(set(a)), imp), maximal(sorted(set(b)), imp)
    return len(ma), len(mb), net_share(ma, mb, imp)


def summarise(rs):
    """The measures of one instance and arm over the runs rs."""
    sem = [r['best_semantic'] for r in rs if r['best_semantic'] is not None]
    return {'found_rate': sum(r['found'] for r in rs) / len(rs),
            'best_semantic': statistics.median(sem) if sem else None,
            'repairs': statistics.median(r['repairs'] for r in rs),
            'holding': statistics.median(r['holding'] for r in rs),
            'wall': statistics.median(r['wall'] for r in rs),
            'backend_calls': statistics.median(r['backend_calls'] for r in rs),
            'backend_unknowns': statistics.median(r['backend_unknowns'] for r in rs),
            'refuting': statistics.median(r['refuting'] for r in rs),
            'grade_share': statistics.median(r['grade_time'] / r['search_wall']
                                             for r in rs if r['search_wall'] > 0)
            if any(r['search_wall'] > 0 for r in rs) else None}


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


def report(title, arm, baseline, pairs, rng):
    """One block: per measure, both medians and the paired difference."""
    print(f'\n## {arm} against {baseline}: {title} ({len(pairs)} instances)')
    print(f'{"measure":16} {baseline:>26} {arm:>26} {"difference":>26} {"p":>7} {"+/-/0":>9}')
    for label, key in MEASURES + [('net subsumption', 'net_share')]:
        if key == 'net_share':
            got = [(0.0, p['net_share']) for p in pairs if p['net_share'] is not None]
        else:
            got = [(p['b'][key], p['a'][key]) for p in pairs
                   if p['b'][key] is not None and p['a'][key] is not None]
        if not got:
            continue
        d = [y - x for x, y in got]
        cells = []
        for xs in ([g[0] for g in got], [g[1] for g in got], d):
            lo, hi = bootstrap_median(xs, rng)
            cells.append(f'{fmt(statistics.median(xs))} [{fmt(lo)}, {fmt(hi)}]')
        if key == 'net_share':
            cells[0] = cells[1] = ''
        signs = f'{sum(x > 0 for x in d)}/{sum(x < 0 for x in d)}/{sum(x == 0 for x in d)}'
        print(f'{label:16} {cells[0]:>26} {cells[1]:>26} {cells[2]:>26} '
              f'{wilcoxon(d):7.4f} {signs:>9}  mean {fmt(statistics.mean(d))}')


def main(argv):
    d, out = Path(argv[0]), Path(argv[1])
    baseline = argv[argv.index('--baseline') + 1] if '--baseline' in argv else 'binary'
    runs_dir = d / (argv[argv.index('--runs') + 1] if '--runs' in argv else 'runs')
    family = {r['name']: r['family']
              for r in csv.DictReader(open(d / 'instances.tsv'), delimiter='\t')}
    runs = []
    for path in sorted(runs_dir.glob('*.json')):
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
                     'grade_time': r['grade_time'], 'gate_errors': len(r['gate_errors']),
                     'formulas': [x['formula'] for x in repairs]})
    errors = sorted(p.stem for p in runs_dir.glob('*.err') if not p.with_suffix('.json').exists())
    arms = sorted({r['arm'] for r in runs} - {baseline})
    by = defaultdict(dict)
    for r in runs:
        by[(r['instance'], r['arm'])][r['seed']] = r

    # Each arm meets the baseline on the seeds both have run, so a partial
    # sweep stays paired.
    rows, compared = [], defaultdict(list)
    for arm in arms:
        for name in sorted({n for n, a in by if a == arm}):
            seeds = sorted(set(by[(name, arm)]) & set(by.get((name, baseline), {})))
            if not seeds:
                continue
            a = summarise([by[(name, arm)][s] for s in seeds])
            b = summarise([by[(name, baseline)][s] for s in seeds])
            ma, mb, share = strength(
                [f for s in seeds for f in by[(name, arm)][s]['formulas']],
                [f for s in seeds for f in by[(name, baseline)][s]['formulas']])
            rows.append({'instance': name, 'family': family[name], 'arm': arm,
                         'seeds': len(seeds), 'maximal': ma, 'baseline_maximal': mb,
                         'net_share': share, **a,
                         **{f'baseline_{k}': v for k, v in b.items()}})
            compared[arm].append({'name': name, 'a': a, 'b': b, 'net_share': share})

    out.mkdir(parents=True, exist_ok=True)
    for fname, data in (('graded-runs.csv', [{k: v for k, v in r.items() if k != 'formulas'}
                                             for r in runs]),
                        ('graded-instances.csv', rows)):
        with open(out / fname, 'w', newline='') as f:
            w = csv.DictWriter(f, fieldnames=list(data[0]))
            w.writeheader()
            w.writerows(data)

    print(f'{len(runs)} runs, {len(errors)} without a result')
    for e in errors:
        print(f'  no result: {e}')
    rng = random.Random(SEED)
    for arm in arms:
        pairs = compared[arm]
        report('all', arm, baseline, pairs, rng)
        for fam in sorted({family[p['name']] for p in pairs}):
            report(fam, arm, baseline, [p for p in pairs if family[p['name']] == fam], rng)
        mine = [r for r in runs if r['arm'] == arm and r['search_wall'] > 0]
        share = [r['grade_time'] / r['search_wall'] for r in mine]
        print(f'\n{arm}: grading time / search wall over {len(share)} runs: '
              f'median {statistics.median(share):.5f}, max {max(share):.5f}; '
              f'refuting lassos per run: median {statistics.median(r["refuting"] for r in mine)}, '
              f'max {max(r["refuting"] for r in mine)}')


if __name__ == '__main__':
    main(sys.argv[1:])
