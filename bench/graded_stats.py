#!/usr/bin/env python3
"""Summarise the status ablation of bench/graded_run.py.

Reads DIR/instances.tsv and DIR/runs/*.json (one search_bench line each) and
writes two CSVs into OUT:

- graded-runs.csv: one row per run (instance, family, arm, seed, found, the
  number of repairs, the best semantic similarity, wall time, back-end
  calls and the grading counters).
- graded-instances.csv: one row per instance and arm against the baseline,
  over the seeds both have run: the found-repair rate and the medians of
  the other measures for both, the sizes of the two pooled maximal sets,
  the net subsumption share and the number of implication pairs Spot left
  undecided.

It then prints, for each arm against the baseline, over all instances and
per family: the median of each measure for both arms (bootstrap 95% CI), the
median paired difference (bootstrap 95% CI), the Wilcoxon signed-rank
p-value, and the number of instances where the arm is higher, lower and
equal. To the right follow PEREDUR's paper statistics: the mean difference,
the Hodges-Lehmann shift with its exact 95% interval, the exact signed-rank
p (exact under ties too), that p Holm-adjusted over the measures of the
block, and the rank-biserial r. graded-instances.csv appends
net_share_non and net_share_imp, the net share under PEREDUR's two
readings of an undecided pair (net_share_readings), and the report gives
both readings and whether the verdict holds under each.

Strength follows PEREDUR. Each arm's repairs of an instance are pooled over
its seeds and reduced to a maximal set: the strongest repairs, those that no
other pooled repair strictly implies, one per equivalence class. The net
subsumption share of A against B is the share of B's maximal set strictly
implied by some repair in A's, minus the converse; it is positive when A's
repairs are the stronger. Implication is plain LTL implication, decided by
`ltlfilt --satisfiable` (Spot 2.15.1 on PATH), and LTLf implication for an
instance whose `semantics` column in instances.tsv is `finite` (a trace set
of finite traces). LTLf pairs go to `ltlf2dfa`: x implies y when the DFA of
x & !y has no accepting transition. `ltlfilt --from-ltlf` left about a fifth
of them undecided, by time-out or past Spot's 32 acceptance sets, and a DFA
has neither problem. The share is defined only
where both arms found a repair on some seed; the found-repair rate covers
the rest.

Every implication verdict is cached in OUT/implications.json, so a rerun
over a growing sweep asks Spot only about new pairs and undecided LTLf
pairs, which earlier runs asked through `ltlfilt --from-ltlf`. `--jobs J` (default 1)
compares J instances at once.

usage: graded_stats.py DIR OUT [--baseline binary] [--runs runs] [--jobs J]
"""
import concurrent.futures as cf
import csv
import json
import math
import random
import statistics
import subprocess
import sys
import threading
import time
from collections import defaultdict
from pathlib import Path

BOOT = 10000
SEED = 20261024
MEASURES = [('found rate', 'found_rate'), ('best semantic', 'best_semantic'),
            ('repairs', 'repairs'), ('holding', 'holding'), ('wall (s)', 'wall'),
            ('back-end calls', 'backend_calls')]


UNDECIDED = set()
# (finite, x, y) -> whether x implies y; None when undecided.
CACHE = {}
LOCK = threading.Lock()


def load_cache(path):
    if path.exists():
        for finite, x, y, v in json.loads(path.read_text()):
            if v is not None or not finite:
                CACHE[(finite, x, y)] = v


SAVED = [0.0]


def save_cache(path, every=0):
    """Writes CACHE to path, at most once per `every` seconds."""
    with LOCK:
        if time.time() - SAVED[0] < every:
            return
        SAVED[0] = time.time()
        items = [[*k, v] for k, v in CACHE.items()]
    path.write_text(json.dumps(items))


def implications(pairs, finite=False):
    """implications_uncached, through CACHE."""
    with LOCK:
        todo = [p for p in pairs if (finite, *p) not in CACHE]
    got = implications_uncached(todo, finite)
    with LOCK:
        for p, v in zip(todo, got):
            CACHE[(finite, *p)] = v
        values = [CACHE[(finite, *p)] for p in pairs]
    with LOCK:
        UNDECIDED.update((finite, *p) for p, v in zip(pairs, values) if v is None)
    return [bool(v) for v in values]


def implications_uncached(pairs, finite=False):
    """For each (x, y), whether x implies y, in one ltlfilt call; under LTLf
    when finite. A batch that runs past BATCH_LIMIT seconds or fails is
    retried one pair per call, and a pair that runs past PAIR_LIMIT seconds
    or fails is None, read as not implied and listed in UNDECIDED, as in
    bench/traces_run.py: some LTLf pairs never finish, and some exceed
    Spot's 32 acceptance sets."""
    try:
        return implications_batch(pairs, finite, BATCH_LIMIT)
    except (subprocess.TimeoutExpired, RuntimeError):
        out = []
        for pair in pairs:
            try:
                out += implications_batch([pair], finite, PAIR_LIMIT)
            except (subprocess.TimeoutExpired, RuntimeError):
                out.append(None)
        return out


BATCH_LIMIT, PAIR_LIMIT = 30, 2


def implications_batch(pairs, finite, limit):
    if not pairs:
        return []
    text = ''.join(f'({x}) & !({y})\n' for x, y in pairs)
    if finite:
        return dfa_empty(text, len(pairs), limit)
    r = subprocess.run(['ltlfilt', '-F', '-', '-v', '--satisfiable', '--format=%L'],
                       input=text, capture_output=True, text=True, timeout=limit)
    # ltlfilt exits 1 when it prints nothing, that is when no pair implies.
    if r.returncode not in (0, 1) or r.stderr:
        raise RuntimeError(f'ltlfilt failed: {r.stderr}')
    unsat = {int(line) for line in r.stdout.split()}
    return [i + 1 in unsat for i in range(len(pairs))]


def dfa_empty(text, n, limit):
    """For each of the n LTLf formulas in text, whether no finite word
    satisfies it: its minimal DFA, one HOA line each (`-Hl`), marks no
    transition accepting."""
    r = subprocess.run(['ltlf2dfa', '-Hl', '-F', '-'], input=text, capture_output=True,
                       text=True, timeout=limit)
    lines = r.stdout.splitlines()
    if r.returncode != 0 or r.stderr or len(lines) != n:
        raise RuntimeError(f'ltlf2dfa failed: {r.stderr}')
    return ['{' not in line.split('--BODY--', 1)[1] for line in lines]


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


def net_share_readings(a, b, imp, undecided):
    """net_share under PEREDUR's two readings of an undecided pair, as
    (non, imp), with `undecided` the pairs either of whose directions is
    undecided. PEREDUR's comparison decides a pair whole, so under `non`
    such a pair is incomparable even where one direction is decided, and
    under `imp` it counts as subsuming both of its members (score.py in
    PEREDUR's repair-diversity experiment). The maximal sets themselves
    read undecided as not implied, as there."""
    if not a or not b:
        return None, None
    out = []
    for loose in (False, True):
        a_sub, b_sub = set(), set()
        for x in a:
            for y in b:
                if x == y:
                    continue
                if (x, y) in undecided:
                    if loose:
                        a_sub.add(x)
                        b_sub.add(y)
                elif imp[(x, y)] and not imp[(y, x)]:
                    b_sub.add(y)
                elif imp[(y, x)] and not imp[(x, y)]:
                    a_sub.add(x)
        out.append(len(b_sub) / len(b) - len(a_sub) / len(a))
    return tuple(out)


def strength_readings(a, b, finite=False):
    """strength, then net_share_readings: the two maximal set sizes, a's
    net share, the undecided pair count, and the share under PEREDUR's
    `non` and `imp` readings."""
    formulas = sorted(set(a) | set(b))
    pairs = [(x, y) for x in formulas for y in formulas if x != y]
    imp = dict(zip(pairs, implications(pairs, finite)))
    with LOCK:
        open_pairs = {p for p in pairs if CACHE[(finite, *p)] is None}
    undecided = {(x, y) for x, y in pairs if (x, y) in open_pairs or (y, x) in open_pairs}
    ma, mb = maximal(sorted(set(a)), imp), maximal(sorted(set(b)), imp)
    return (len(ma), len(mb), net_share(ma, mb, imp), len(open_pairs),
            *net_share_readings(ma, mb, imp, undecided))


def strength(a, b, finite=False):
    """The sizes of the maximal sets of repair lists a and b, a's net share,
    and how many of the pairs compared were undecided."""
    return strength_readings(a, b, finite)[:4]


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


def wilcoxon_exact(ds):
    """Two-sided exact signed-rank p, as PEREDUR's paper computes it
    (`wilcoxon_exact_p` in its data/stats.py): zeros drop, tied magnitudes
    take the mean rank, and the tail is counted over subsets of the doubled
    ranks, so ties stay exact."""
    nz = [d for d in ds if d != 0]
    if not nz:
        return 1.0
    ranks = mean_ranks([abs(d) for d in nz])
    w_plus = sum(r for r, d in zip(ranks, nz) if d > 0)
    doubled = [int(round(2 * r)) for r in ranks]
    total = sum(doubled)
    counts = [1] + [0] * total
    for rank in doubled:
        for value in range(total, rank - 1, -1):
            counts[value] += counts[value - rank]
    lower = min(int(round(2 * w_plus)), total - int(round(2 * w_plus)))
    tail = sum(counts[:lower + 1]) + sum(counts[total - lower:])
    return min(1.0, tail / 2 ** len(doubled))


def mean_ranks(xs):
    """The ranks of xs from 1, ties at their mean rank."""
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    ranks = [0.0] * len(xs)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        for k in range(i, j + 1):
            ranks[order[k]] = (i + j + 2) / 2
        i = j + 1
    return ranks


HL_MISMATCH = [0]


def hodges_lehmann(ds):
    """The Hodges-Lehmann shift and its exact 95% interval, (est, lo, hi),
    as PEREDUR's paper computes them (`hodges_lehmann` in its
    data/stats.py): zeros drop, the estimate is the median of the Walsh
    averages, and the interval is the signed-rank test inverted. Its bounds
    are the Walsh averages at 0-based index t from either end, t the largest
    value with P(W <= t) <= 2.5% under the untied null, so that the lower
    bound is positive exactly when W- <= t. PEREDUR's helper takes index
    t + 1, one place narrower: on 1..10 it gives [3.5, 7.5] where the
    textbook interval is [3.0, 8.0], and on (1..8, -9, 10) it excludes zero
    at p = 0.064, which trips its own assertion. With n <= 5 no t exists
    and the interval is unbounded. With tied magnitudes the untied null is
    approximate, and PEREDUR asserts that the interval excludes zero
    exactly when wilcoxon_exact is below 0.05; here a mismatch is counted
    in HL_MISMATCH instead, so a report never dies on it. All None when
    every difference is zero."""
    nz = [d for d in ds if d != 0]
    n = len(nz)
    if n == 0:
        return None, None, None
    walsh = sorted((nz[i] + nz[j]) / 2 for i in range(n) for j in range(i, n))
    top = n * (n + 1) // 2
    counts = [1] + [0] * top
    for rank in range(1, n + 1):
        for value in range(top, rank - 1, -1):
            counts[value] += counts[value - rank]
    t, tail = -1, 0
    while (tail + counts[t + 1]) / 2 ** n <= 0.025:
        t += 1
        tail += counts[t]
    est = statistics.median(walsh)
    lo, hi = (walsh[t], walsh[-1 - t]) if t >= 0 else (-math.inf, math.inf)
    if (lo > 0 or hi < 0) != (wilcoxon_exact(ds) < 0.05):
        HL_MISMATCH[0] += 1
    return est, lo, hi


def holm(ps):
    """Holm's step-down adjustment of the p-values ps, in their order."""
    order = sorted(range(len(ps)), key=lambda i: ps[i])
    out, running = [0.0] * len(ps), 0.0
    for rank, i in enumerate(order):
        running = max(running, min(1.0, (len(ps) - rank) * ps[i]))
        out[i] = running
    return out


def rank_biserial(ds):
    """The matched-pairs rank-biserial correlation (R+ - R-) / (R+ + R-),
    zeros dropped and tied magnitudes at their mean rank, as PEREDUR's
    effect_sizes.py computes it. None when every difference is zero."""
    nz = [d for d in ds if d != 0]
    if not nz:
        return None
    ranks = mean_ranks([abs(d) for d in nz])
    plus = sum(r for r, d in zip(ranks, nz) if d > 0)
    return (2 * plus - sum(ranks)) / sum(ranks)


def fmt(x, digits=3):
    if x is None:
        return '-'
    return f'{x:.{digits}f}'


def report(title, arm, baseline, pairs, rng):
    """One block: per measure, both medians and the paired difference."""
    print(f'\n## {arm} against {baseline}: {title} ({len(pairs)} instances)')
    print(f'{"measure":16} {baseline:>26} {arm:>26} {"difference":>26} {"p":>7} {"+/-/0":>9}'
          f'  {"mean":>12} {"HL shift [exact 95% CI]":>26} {"p exact":>8} {"p Holm":>7} {"r_rb":>6}')
    lines, ps = [], []
    readings = [('net subs (non)', 'net_share_non'), ('net subs (imp)', 'net_share_imp')]
    for label, key in MEASURES + [('net subsumption', 'net_share')] + readings:
        if key.startswith('net_share'):
            got = [(0.0, p[key]) for p in pairs if p.get(key) is not None]
        else:
            got = [(p['b'][key], p['a'][key]) for p in pairs
                   if p['b'][key] is not None and p['a'][key] is not None]
        if not got:
            continue
        d = [y - x for x, y in got]
        cells = []
        # The reading rows draw from their own stream, so the bootstrap
        # intervals of every other row match the reports made before them.
        draw = random.Random(SEED) if (label, key) in readings else rng
        for xs in ([g[0] for g in got], [g[1] for g in got], d):
            lo, hi = bootstrap_median(xs, draw)
            cells.append(f'{fmt(statistics.median(xs))} [{fmt(lo)}, {fmt(hi)}]')
        if key.startswith('net_share'):
            cells[0] = cells[1] = ''
        signs = f'{sum(x > 0 for x in d)}/{sum(x < 0 for x in d)}/{sum(x == 0 for x in d)}'
        est, lo, hi = hodges_lehmann(d)
        exact = wilcoxon_exact(d)
        # The two readings of undecided pairs restate net subsumption, so
        # they stay out of the Holm family.
        if (label, key) not in readings:
            ps.append(exact)
        lines.append((label, key, cells, d, signs, est, lo, hi, exact))
    adjusted = iter(holm(ps))
    verdicts = {}
    for label, key, cells, d, signs, est, lo, hi, exact in lines:
        held = '' if key in ('net_share_non', 'net_share_imp') else f'{next(adjusted):7.4f}'
        r = rank_biserial(d)
        print(f'{label:16} {cells[0]:>26} {cells[1]:>26} {cells[2]:>26} '
              f'{wilcoxon(d):7.4f} {signs:>9}  {"mean " + fmt(statistics.mean(d)):12} '
              f'{fmt(est) + " [" + fmt(lo) + ", " + fmt(hi) + "]":>26} {exact:8.4f} {held:>7} '
              f'{fmt(r, 2):>6}')
        if key in ('net_share_non', 'net_share_imp'):
            verdicts[key] = 0 if lo is None or lo <= 0 <= hi else (1 if lo > 0 else -1)
    if len(verdicts) == 2:
        both = verdicts['net_share_non'] == verdicts['net_share_imp']
        word = {1: f'{arm} stronger', -1: f'{baseline} stronger', 0: 'no difference'}
        print(f'net subsumption under both readings of undecided pairs: '
              f'{word[verdicts["net_share_non"]] if both else "the readings disagree"}')


def parse_run_name(stem, names):
    """(instance, arm, seed) of a run file's stem `<instance>-<arm>-<seed>`.
    Instance and arm names may both hold hyphens, so the instance is the
    longest name in `names` that the stem starts with."""
    head, seed = stem.rsplit('-', 1)
    name = max((n for n in names if head.startswith(n + '-')), key=len, default=None)
    if name is None:
        raise KeyError(f'{stem}: no instance of instances.tsv')
    return name, head[len(name) + 1:], seed


def main(argv):
    d, out = Path(argv[0]), Path(argv[1])
    baseline = argv[argv.index('--baseline') + 1] if '--baseline' in argv else 'binary'
    runs_dir = d / (argv[argv.index('--runs') + 1] if '--runs' in argv else 'runs')
    instances = list(csv.DictReader(open(d / 'instances.tsv'), delimiter='\t'))
    family = {r['name']: r['family'] for r in instances}
    finite = {r['name'] for r in instances if r.get('semantics') == 'finite'}
    runs = []
    for path in sorted(runs_dir.glob('*.json')):
        r = json.loads(path.read_text())
        name, arm, seed = parse_run_name(path.stem, family)
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
    out.mkdir(parents=True, exist_ok=True)
    cache = out / 'implications.json'
    load_cache(cache)
    jobs = []
    for arm in arms:
        for name in sorted({n for n, a in by if a == arm}):
            seeds = sorted(set(by[(name, arm)]) & set(by.get((name, baseline), {})))
            if seeds:
                jobs.append((arm, name, seeds))

    def measure(job):
        arm, name, seeds = job
        result = strength_readings(
            [f for s in seeds for f in by[(name, arm)][s]['formulas']],
            [f for s in seeds for f in by[(name, baseline)][s]['formulas']],
            name in finite)
        save_cache(cache, every=1800)
        return result

    with cf.ThreadPoolExecutor(int(argv[argv.index('--jobs') + 1])
                               if '--jobs' in argv else 1) as pool:
        strengths = list(pool.map(measure, jobs))
    save_cache(cache)
    for (arm, name, seeds), (ma, mb, share, undecided, non, imp) in zip(jobs, strengths):
        a = summarise([by[(name, arm)][s] for s in seeds])
        b = summarise([by[(name, baseline)][s] for s in seeds])
        rows.append({'instance': name, 'family': family[name], 'arm': arm,
                     'seeds': len(seeds), 'maximal': ma, 'baseline_maximal': mb,
                     'net_share': share, 'undecided_pairs': undecided, **a,
                     **{f'baseline_{k}': v for k, v in b.items()},
                     'net_share_non': non, 'net_share_imp': imp})
        compared[arm].append({'name': name, 'a': a, 'b': b, 'net_share': share,
                              'net_share_non': non, 'net_share_imp': imp})

    for fname, data in (('graded-runs.csv', [{k: v for k, v in r.items() if k != 'formulas'}
                                             for r in runs]),
                        ('graded-instances.csv', rows)):
        with open(out / fname, 'w', newline='') as f:
            w = csv.DictWriter(f, fieldnames=list(data[0]))
            w.writeheader()
            w.writerows(data)

    print(f'{len(runs)} runs, {len(errors)} without a result; '
          f'{len(UNDECIDED)} implication pairs undecided, read as not implied')
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
    if HL_MISMATCH[0]:
        print(f'\n{HL_MISMATCH[0]} Hodges-Lehmann intervals disagree with the exact p at 0.05 '
              f'(tied magnitudes)')


if __name__ == '__main__':
    main(sys.argv[1:])
