#!/usr/bin/env python3
"""Summarise the initial-state triviality runs of bench/graded_run.py.

Reads DIR/instances.tsv and DIR/RUNS/*.json from two arms on the same
seeds: FLAG (`--initial flag`, the search as before, with each repair
marked when I ∧ Init implies it) and FILTER (`--initial filter`, those
repairs dropped in the loop and the gate). See "Triviality at the initial
state" in docs/dev/search.md. Writes three CSVs into OUT, with PREFIX:

- PREFIX-runs.csv: one row per run: found, repairs, the repairs I ∧ Init
  implies (FLAG only) and how many of those carry a vacuity flag, wall
  time, back-end calls and the cost of finding Init.
- PREFIX-instances.csv: one row per instance over the seeds both arms ran:
  the found rates of FLAG, of FLAG after dropping the marked repairs post
  hoc, and of FILTER; the share of FLAG's repairs marked; the sizes of
  FLAG's post-hoc maximal set and of FILTER's, and FILTER's net
  subsumption share against it (as in bench/graded_stats.py).
- PREFIX-removed.csv: every distinct marked repair, with Init, its vacuity
  count, its best semantic similarity and ⊤'s (from TOP, when given).

It prints the per-family totals and the paired tests. Spot 2.15.1 must be
on PATH for the strength comparison.

usage: initial_stats.py DIR OUT PREFIX [--runs runs-init]
       [--flag initflag] [--filter initfilter] [--top baselines-top.csv]
"""
import argparse
import csv
import json
import random
import statistics
from collections import defaultdict
from pathlib import Path

from graded_stats import SEED, bootstrap_median, fmt, strength, wilcoxon


def load(d, runs, arm, rows):
    out = {}
    for row in rows:
        for path in (d / runs).glob(f"{row['name']}-{arm}-*.json"):
            seed = int(path.stem.rsplit('-', 1)[1])
            if path.stem != f"{row['name']}-{arm}-{seed}":
                continue
            out[(row['name'], seed)] = json.loads(path.read_text())
    return out


def run_row(row, arm, seed, j):
    reps = j.get('repairs', [])
    trivial = [r for r in reps if r.get('initial_trivial')]
    return {'instance': row['name'], 'family': row['family'], 'arm': arm, 'seed': seed,
            'class': j['instance'], 'found': int(bool(reps)), 'repairs': len(reps),
            'trivial': len(trivial), 'trivial_vacuous': sum(r['vacuous'] > 0 for r in trivial),
            'kept': len(reps) - len(trivial),
            'best_semantic': max((r['semantic'] for r in reps), default=''),
            'wall': j['wall'], 'search_wall': j['search_wall'],
            'backend_calls': j['backend_calls'],
            'initial_calls': j.get('initial_calls', ''), 'initial_time': j.get('initial_time', ''),
            'initial': j.get('initial', ''), 'initial_error': j.get('initial_error', '')}


def main():
    p = argparse.ArgumentParser()
    p.add_argument('dir')
    p.add_argument('out')
    p.add_argument('prefix')
    p.add_argument('--runs', default='runs-init')
    p.add_argument('--flag', default='initflag')
    p.add_argument('--filter', default='initfilter')
    p.add_argument('--top')
    a = p.parse_args()
    d, out = Path(a.dir), Path(a.out)
    rows = list(csv.DictReader(open(d / 'instances.tsv'), delimiter='\t'))
    flag, filt = load(d, a.runs, a.flag, rows), load(d, a.runs, a.filter, rows)
    top = {}
    if a.top:
        # baselines-top.csv names the instance `instance`, baselines-cegiw.csv `name`.
        top = {r.get('instance', r.get('name')): float(r['top_semantic'])
               for r in csv.DictReader(open(a.top))}

    run_rows = []
    for row in rows:
        for arm, runs in ((a.flag, flag), (a.filter, filt)):
            for (name, seed), j in sorted(runs.items()):
                if name == row['name']:
                    run_rows.append(run_row(row, arm, seed, j))
    with open(out / f'{a.prefix}-runs.csv', 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(run_rows[0]))
        w.writeheader()
        w.writerows(run_rows)

    inst_rows, removed = [], {}
    for row in rows:
        name = row['name']
        seeds = sorted(s for (n, s) in flag if n == name and (n, s) in filt)
        if not seeds:
            continue
        fl = [flag[(name, s)] for s in seeds]
        fi = [filt[(name, s)] for s in seeds]
        if not any(j['instance'].startswith('proper') or j.get('repairs') or j.get('holding')
                   for j in fl):
            continue
        reps = [r for j in fl for r in j['repairs']]
        kept = [r['formula'] for r in reps if not r.get('initial_trivial')]
        filt_reps = [r['formula'] for j in fi for r in j['repairs']]
        for j in fl:
            for r in j['repairs']:
                if r.get('initial_trivial'):
                    e = removed.setdefault((name, r['formula']), {
                        'instance': name, 'family': row['family'], 'initial': j.get('initial', ''),
                        'formula': r['formula'], 'vacuous': r['vacuous'], 'seeds': 0,
                        'semantic': r['semantic'], 'top_semantic': top.get(name, '')})
                    e['seeds'] += 1
        size_post, size_filter, share = '', '', ''
        if kept and filt_reps:
            size_filter, size_post, share = strength(filt_reps, kept)
        med = lambda js, k: statistics.median(j[k] for j in js)
        inst_rows.append({
            'instance': name, 'family': row['family'], 'seeds': len(seeds),
            'class': fl[0]['instance'], 'initial': fl[0].get('initial', ''),
            'initial_error': fl[0].get('initial_error', ''),
            'found_flag': sum(bool(j['repairs']) for j in fl) / len(seeds),
            'found_posthoc': sum(any(not r.get('initial_trivial') for r in j['repairs'])
                                 for j in fl) / len(seeds),
            'found_filter': sum(bool(j['repairs']) for j in fi) / len(seeds),
            'repairs_flag': len(reps), 'trivial_flag': len(reps) - len(kept),
            'repairs_filter': len(filt_reps),
            'maximal_posthoc': size_post, 'maximal_filter': size_filter,
            'net_share_filter': '' if share in ('', None) else round(share, 4),
            'wall_flag': med(fl, 'wall'), 'wall_filter': med(fi, 'wall'),
            'calls_flag': med(fl, 'backend_calls'), 'calls_filter': med(fi, 'backend_calls'),
            'initial_time': med(fl, 'initial_time') if 'initial_time' in fl[0] else '',
            'initial_calls': med(fl, 'initial_calls') if 'initial_calls' in fl[0] else '',
        })
    with open(out / f'{a.prefix}-instances.csv', 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(inst_rows[0]))
        w.writeheader()
        w.writerows(inst_rows)
    with open(out / f'{a.prefix}-removed.csv', 'w', newline='') as f:
        rem = sorted(removed.values(), key=lambda e: (e['family'], e['instance'], -e['seeds']))
        w = csv.DictWriter(f, fieldnames=['instance', 'family', 'initial', 'formula', 'vacuous',
                                          'seeds', 'semantic', 'top_semantic'])
        w.writeheader()
        w.writerows(rem)

    rng = random.Random(SEED)
    fams = sorted({r['family'] for r in inst_rows})
    print('| Family | Instances | Runs | Repairs | Marked | Marked, vacuous | '
          'Instances with a mark | Runs losing all | Of those, filter finds one | '
          'Instances losing all |')
    print('|---|---|---|---|---|---|---|---|---|---|')
    filter_found = {(r['instance'], r['seed']): r['found'] for r in run_rows
                    if r['arm'] == a.filter}
    for fam in fams + ['all']:
        ir = [r for r in inst_rows if fam in ('all', r['family'])]
        rr = [r for r in run_rows if r['arm'] == a.flag and fam in ('all', r['family'])]
        lost = [r for r in rr if r['found'] and not r['kept']]
        print(f"| {fam} | {len(ir)} | {len(rr)} | {sum(r['repairs'] for r in rr)} | "
              f"{sum(r['trivial'] for r in rr)} | {sum(r['trivial_vacuous'] for r in rr)} | "
              f"{sum(r['trivial_flag'] > 0 for r in ir)} | {len(lost)} | "
              f"{sum(filter_found.get((r['instance'], r['seed']), 0) for r in lost)} | "
              f"{sum(r['found_flag'] > 0 and r['found_posthoc'] == 0 for r in ir)} |")
    print()
    print('| Family | Measure | Flag | Filter | Δ median | p | higher/lower/equal |')
    print('|---|---|---|---|---|---|---|')
    for fam in fams + ['all']:
        ir = [r for r in inst_rows if fam in ('all', r['family'])]
        for label, x, y in (('found rate, post hoc', 'found_posthoc', 'found_filter'),
                            ('found rate, any', 'found_flag', 'found_filter'),
                            ('wall (s)', 'wall_flag', 'wall_filter'),
                            ('back-end calls', 'calls_flag', 'calls_filter')):
            ds = [r[y] - r[x] for r in ir]
            hi = sum(v > 0 for v in ds)
            lo = sum(v < 0 for v in ds)
            ci = bootstrap_median(ds, rng)
            print(f"| {fam} | {label} | {fmt(statistics.median(r[x] for r in ir))} | "
                  f"{fmt(statistics.median(r[y] for r in ir))} | {fmt(statistics.median(ds))} "
                  f"[{fmt(ci[0])}, {fmt(ci[1])}] | {wilcoxon(ds):.4f} | "
                  f"{hi}/{lo}/{len(ds) - hi - lo} |")
        shares = [r['net_share_filter'] for r in ir if r['net_share_filter'] != '']
        if shares:
            print(f"| {fam} | net share, filter vs post hoc | | | "
                  f"{fmt(statistics.median(shares))} | {wilcoxon(shares):.4f} | "
                  f"{sum(s > 0 for s in shares)}/{sum(s < 0 for s in shares)}/"
                  f"{sum(s == 0 for s in shares)} |")
    it = [float(r['initial_time']) for r in run_rows
          if r['arm'] == a.flag and r['initial_time'] != '']
    ic = [int(r['initial_calls']) for r in run_rows
          if r['arm'] == a.flag and r['initial_calls'] != '']
    errs = [r for r in inst_rows if r['initial_error']]
    if it:
        print(f"\nInit: median {statistics.median(it):.3f} s, max {max(it):.3f} s, "
              f"median {statistics.median(ic)} back-end calls, max {max(ic)}; "
              f"{len(errs)} instances without Init: "
              + ', '.join(f"{r['instance']} ({r['initial_error'][:60]})" for r in errs))


if __name__ == '__main__':
    main()
