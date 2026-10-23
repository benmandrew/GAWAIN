#!/usr/bin/env python3
"""Summarise oracle_bench CSVs: cache hit rates and model-checking time distributions.

usage: oracle_stats.py CSV...

Groups are each bench (one model), then each back end pooled (benches named rers* ran
SPIN, lmcs* ran nuXmv). For each group it prints:
  * queries and the share answered by each tier; the cache hit rate is the share answered
    by tiers 0-2 (memo, lasso, implication);
  * for back-end calls (time of the back end alone) and for all queries (wall time of the
    whole query): n, median, mean, the central 95% range (p2.5-p97.5) and a bootstrap 95%
    confidence interval for the mean (10,000 resamples, fixed seed), by verdict;
  * the unknown verdicts by reason.
Percentiles interpolate linearly between order statistics. Times are in seconds.
"""
import csv, random, re, sys
from collections import Counter, defaultdict
from multiprocessing import Pool

TIERS = ['memo', 'lasso', 'implication', 'backend']
RESAMPLES, SEED = 10_000, 20261023


def percentile(xs, p):
    """xs sorted."""
    h = (len(xs) - 1) * p
    lo = int(h)
    hi = min(lo + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (h - lo)


def summary(xs):
    xs = sorted(xs)
    n = len(xs)
    rng = random.Random(SEED)
    means = sorted(sum(rng.choices(xs, k=n)) / n for _ in range(RESAMPLES))
    return (n, percentile(xs, 0.5), sum(xs) / n, percentile(xs, 0.025), percentile(xs, 0.975),
            percentile(means, 0.025), percentile(means, 0.975))


def reason_class(reason):
    if 'X counts' in reason: return 'X formula (SPIN refuses)'
    if 'timed out' in reason: return 'timeout'
    if 'killed by signal' in reason: return 'killed (memory or CPU limit)'
    if 'depth' in reason.lower(): return 'search depth reached'
    return re.sub(r'\d+', '#', reason)[:70]


def fmt(x):
    return f'{x:.3g}' if x < 100 else f'{x:.0f}'


def main(paths):
    rows = []
    for path in paths:
        with open(path, newline='') as fh:
            rows += list(csv.DictReader(fh))
    groups = defaultdict(list)
    for r in rows:
        groups[r['bench']].append(r)
    benches = sorted(groups)
    for r in rows:
        groups['SPIN pooled' if r['bench'].startswith('rers') else 'nuXmv pooled'].append(r)
    order = benches + [g for g in ('SPIN pooled', 'nuXmv pooled') if g in groups]

    print('## Cache hit rate\n')
    print('| Group | Queries | Memo | Lasso | Implication | Back end | Cache hit rate | Unknown |')
    print('|---|---|---|---|---|---|---|---|')
    for g in order:
        rs = groups[g]
        n = len(rs)
        c = Counter(r['tier'] for r in rs)
        unknown = sum(r['verdict'] == 'unknown' for r in rs)
        share = lambda k: f'{100 * c[k] / n:.1f}%'
        hit = 100 * (n - c['backend']) / n
        print(f'| {g} | {n} | ' + ' | '.join(share(t) for t in TIERS) + f' | {hit:.1f}% | {unknown} |')

    jobs, keys = [], []
    for g in order:
        for scope, col in (('back end', 'backend_s'), ('all queries', 'query_s')):
            rs = [r for r in groups[g] if scope == 'all queries' or r['tier'] == 'backend']
            for verdict in ('holds', 'fails', 'holds+fails'):
                xs = [float(r[col]) for r in rs
                      if r['verdict'] != 'unknown' and (verdict == 'holds+fails' or r['verdict'] == verdict)]
                if xs:
                    keys.append((g, scope, verdict)); jobs.append(xs)
    with Pool() as pool:
        results = pool.map(summary, jobs)

    print('\n## Time per query (seconds)\n')
    print('| Group | Scope | Verdict | n | Median | Mean | p2.5–p97.5 | Mean 95% CI |')
    print('|---|---|---|---|---|---|---|---|')
    for (g, scope, verdict), (n, med, mean, lo_p, hi_p, lo, hi) in zip(keys, results):
        print(f'| {g} | {scope} | {verdict} | {n} | {fmt(med)} | {fmt(mean)} | '
              f'{fmt(lo_p)}–{fmt(hi_p)} | {fmt(lo)}–{fmt(hi)} |')

    print('\n## Unknown verdicts\n')
    print('| Group | Reason | Count | Back-end time median (s) |')
    print('|---|---|---|---|')
    for g in order:
        by = defaultdict(list)
        for r in groups[g]:
            if r['verdict'] == 'unknown':
                by[reason_class(r['reason'])].append(float(r['backend_s'] or 0))
        for reason, ts in sorted(by.items()):
            print(f'| {g} | {reason} | {len(ts)} | {fmt(percentile(sorted(ts), 0.5))} |')


if __name__ == '__main__':
    main(sys.argv[1:])
