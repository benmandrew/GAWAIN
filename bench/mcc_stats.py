#!/usr/bin/env python3
"""Summarise mcc_check CSVs against the MCC 2026 consensus verdicts.

usage: mcc_stats.py CANDIDATES CSV...

CANDIDATES is the list from bench/mcc_select.py, whose verdict column is the
oracle's (every row FALSE). Each CSV row of mcc_check holds the TAPAAL back
end's verdict on phi and on its negation. The script prints:
  * agreement: phi FALSE (with a replayed lasso) / TRUE (a disagreement) /
    UNKNOWN, over every candidate, and missing rows;
  * the degeneracy filter: of the phi-FALSE rows, how many have neg TRUE
    (M |= !phi, dropped), neg FALSE (kept) and neg UNKNOWN;
  * time per check of phi (seconds): n, median, mean, central 95% range
    (p2.5-p97.5) and a bootstrap 95% CI of the mean (10,000 resamples,
    fixed seed), for phi FALSE and for neg by verdict;
  * the unknown verdicts by reason.
With --subset FILE it also writes the kept rows (phi FALSE, neg FALSE)
as CSV to FILE.
"""
import csv, random, re, sys
from collections import Counter, defaultdict

RESAMPLES, SEED = 10_000, 20261023


def percentile(xs, p):
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


def fmt(x):
    return f'{x:.3g}' if x < 100 else f'{x:.0f}'


def reason_class(reason):
    reason = reason.split(' | ')[0]
    if 'timed out' in reason: return 'timeout'
    if 'killed by signal' in reason: return 'killed (memory or CPU limit)'
    return re.sub(r'\d+', '#', reason)[:80]


def main(argv):
    subset = None
    if argv[:1] == ['--subset']:
        subset, argv = argv[1], argv[2:]
    cand_path, paths = argv[0], argv[1:]
    with open(cand_path, newline='') as fh:
        cands = {f"{r['instance']}-{r['formula']}": r for r in csv.DictReader(fh)}
    rows = {}
    for path in paths:
        with open(path, newline='') as fh:
            for r in csv.DictReader(fh):
                if r['formula'] in cands:
                    rows[r['formula']] = r
    n = len(cands)
    c = Counter(r['verdict'] for r in rows.values())
    print('## Agreement with the oracle (all candidates FALSE)\n')
    print('| Candidates | FALSE (agree) | TRUE (disagree) | UNKNOWN | No row |')
    print('|---|---|---|---|---|')
    print(f"| {n} | {c['FALSE']} ({100 * c['FALSE'] / n:.1f}%) | {c['TRUE']} | {c['UNKNOWN']} | {n - len(rows)} |")
    for r in rows.values():
        if r['verdict'] == 'TRUE':
            print(f"\nDisagreement: {r['formula']} (oracle tools: {cands[r['formula']]['tools']})")

    false = [r for r in rows.values() if r['verdict'] == 'FALSE']
    d = Counter(r['neg_verdict'] for r in false)
    print('\n## Degeneracy filter on the agreeing rows\n')
    print('| phi FALSE | neg TRUE (degenerate, dropped) | neg FALSE (kept) | neg UNKNOWN |')
    print('|---|---|---|---|')
    print(f"| {len(false)} | {d['TRUE']} | {d['FALSE']} | {d['UNKNOWN']} |")

    print('\n## Time per check (seconds)\n')
    print('| Check | Verdict | n | Median | Mean | p2.5–p97.5 | Mean 95% CI |')
    print('|---|---|---|---|---|---|---|')
    groups = [('phi', 'FALSE', [float(r['seconds']) for r in false])]
    for v in ('TRUE', 'FALSE'):
        groups.append(('neg', v, [float(r['neg_seconds']) for r in false if r['neg_verdict'] == v]))
    for check, verdict, xs in groups:
        if xs:
            k, med, mean, lo_p, hi_p, lo, hi = summary(xs)
            print(f'| {check} | {verdict} | {k} | {fmt(med)} | {fmt(mean)} | {fmt(lo_p)}–{fmt(hi_p)} | {fmt(lo)}–{fmt(hi)} |')
    for limit in (1, 10):
        share = sum(float(r['seconds']) <= limit for r in false) / max(len(false), 1)
        print(f'\nphi FALSE within {limit} s: {100 * share:.1f}%')

    print('\n## Unknown verdicts\n')
    print('| Check | Reason | Count |')
    print('|---|---|---|')
    by = Counter()
    for r in rows.values():
        if r['verdict'] == 'UNKNOWN':
            by[('phi', reason_class(r['reason']))] += 1
        elif r['neg_verdict'] == 'UNKNOWN':
            by[('neg', reason_class(r['reason']))] += 1
    for (check, reason), k in sorted(by.items()):
        print(f'| {check} | {reason} | {k} |')

    if subset:
        kept = sorted((r for r in false if r['neg_verdict'] == 'FALSE'), key=lambda r: r['formula'])
        with open(subset, 'w', newline='') as fh:
            w = csv.writer(fh)
            w.writerow(['instance', 'states', 'formula', 'atoms', 'seconds', 'neg_seconds', 'prefix', 'cycle', 'oracle_tools'])
            for r in kept:
                cand = cands[r['formula']]
                w.writerow([r['instance'], cand['states'], r['formula'], r['atoms'], r['seconds'],
                            r['neg_seconds'], r['prefix'], r['cycle'], cand['tools']])


if __name__ == '__main__':
    main(sys.argv[1:])
