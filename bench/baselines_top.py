#!/usr/bin/env python3
"""Compare GAWAIN's repairs with the trivial weakening ⊤.

⊤ holds on every model and is implied by every φ, so it is the floor of
any weakening. This script reads the binary-arm runs of the status
ablation and reports, per instance, how far above ⊤ the search lands.

Inputs:

- RUNS: a directory of search_bench JSON lines named
  <instance>-binary-<seed>.json, the binary arm of `bench/graded_run.py`
  at 30 x 40 (copied from runs/ on av2 and av3; see
  `bench/notes/baselines.md`).
- CSV: bench/results/graded-runs.csv, which selects the runs and gives the
  family; each JSON's wall time must match its row.
- CSV_4X8 (optional): bench/results/graded-runs-4x8.csv, for the best
  score at the small budget, which needs no JSON.

For each instance, ⊤ is scored against φ by the `score` binary (the same
lasso count as the search, with 120 s per exec), and every distinct repair
is asked whether it is valid, that is equivalent to ⊤, with one
`ltlfilt --satisfiable` batch (Spot 2.15.1 on PATH). The closed share of a
run is (best - top) / (1 - top), where best is the run's best semantic
similarity and top is ⊤'s: 0 at the floor, 1 at φ itself. Every distinct
repair is also scored by its rejection strength, under which ⊤ scores 0
and φ 1, so a run's best rejection strength is its closed share without
the normalisation; `closed_rejection` is its median over seeds.

Writes OUT (one row per instance) and prints the summary that
`bench/notes/baselines.md` quotes.

`--arm` reads another arm, such as the evaluation's `default` or
`mutation` (docs/dev/evaluation.md), with RUNS and CSV to match.

usage: baselines_top.py RUNS CSV OUT [--csv-4x8 PATH] [--score PATH] [--arm binary]
"""
import argparse
import csv
import json
import random
import statistics
import subprocess
from collections import defaultdict
from pathlib import Path

from graded_stats import bootstrap_median, implications

SEED = 20261024
# Scores are printed to six places, so a tie with ⊤ is within this.
TIE = 1e-6


def load(runs_dir, csv_path, arm):
    """The rows of arm in csv_path by instance, each with its JSON run."""
    by_instance = defaultdict(list)
    for row in csv.DictReader(open(csv_path)):
        if row['arm'] != arm:
            continue
        path = Path(runs_dir) / f"{row['instance']}-{arm}-{row['seed']}.json"
        run = json.loads(path.read_text().strip().splitlines()[-1])
        if abs(run['wall'] - float(row['wall'])) > 0.01:
            raise SystemExit(f'{path}: wall {run["wall"]} != {row["wall"]} in {csv_path}')
        by_instance[row['instance']].append((row, run))
    return by_instance


def scores(binary, pairs):
    """The (semantic, syntactic, rejection) score of each (φ, φ') pair."""
    text = ''.join(f'{phi}\t{candidate}\n' for phi, candidate in pairs)
    r = subprocess.run([binary, '--timeout', '120'], input=text, capture_output=True,
                       text=True, check=True)
    return [tuple(map(float, line.split('\t'))) for line in r.stdout.splitlines()]


def best_of(rows):
    """The best semantic similarity of each row that found a repair."""
    return [float(r['best_semantic']) for r in rows if r['found'] == '1' and r['best_semantic']]


def median(xs):
    return statistics.median(xs) if xs else None


def main():
    p = argparse.ArgumentParser()
    p.add_argument('runs')
    p.add_argument('csv')
    p.add_argument('out')
    p.add_argument('--csv-4x8')
    p.add_argument('--score', default='target/release/score')
    p.add_argument('--arm', default='binary')
    a = p.parse_args()

    runs = load(a.runs, a.csv, a.arm)
    small = defaultdict(list)
    if a.csv_4x8:
        for row in csv.DictReader(open(a.csv_4x8)):
            if row['arm'] == 'binary':
                small[row['instance']].append(row)
    names = sorted(runs)
    phis = {n: runs[n][0][1]['phi'] for n in names}
    top = dict(zip(names, scores(a.score, [(phis[n], 'true') for n in names])))

    # Every distinct repair, asked once whether it is valid.
    repairs = {n: sorted({rep['formula'] for _, run in runs[n] for rep in run['repairs']})
               for n in names}
    flat = [(n, f) for n in names for f in repairs[n]]
    valid = dict(zip(flat, implications([('true', f) for _, f in flat])))
    # The rejection strength of every distinct repair, one `score` exec per instance.
    rejection = {}
    for n in names:
        if repairs[n]:
            scored = scores(a.score, [(phis[n], f) for f in repairs[n]])
            rejection[n] = {f: r for f, (_, _, r) in zip(repairs[n], scored)}

    out_rows = []
    for n in names:
        rows = [row for row, _ in runs[n]]
        t_sem, t_syn, _ = top[n]
        # The runs store their scores to six places, and `score` prints
        # fifteen, so ⊤ is rounded to compare like with like.
        t_sem = round(t_sem, 6)
        best = best_of(rows)
        closed = [(b - t_sem) / (1 - t_sem) for b in best] if t_sem < 1 else []
        semantic = {rep['formula']: rep['semantic'] for _, run in runs[n] for rep in run['repairs']}
        syntactic = [max(rep['similarity'] for rep in run['repairs'])
                     for _, run in runs[n] if run['repairs']]
        rejected = [max(rejection[n][rep['formula']] for rep in run['repairs'])
                    for _, run in runs[n] if run['repairs']]
        small_best = best_of(small.get(n, []))
        out_rows.append({
            'instance': n,
            'family': rows[0]['family'],
            'phi': phis[n],
            'seeds': len(rows),
            'found': len(best),
            'top_semantic': f'{t_sem:.6f}',
            'best_semantic': f'{median(best):.6f}' if best else '',
            'best_semantic_max': f'{max(best):.6f}' if best else '',
            'closed_share': f'{median(closed):.4f}' if closed else '',
            'closed_rejection': f'{median(rejected):.4f}' if rejected else '',
            'top_syntactic': f'{t_syn:.6f}',
            'best_syntactic': f'{median(syntactic):.6f}' if syntactic else '',
            'repairs': len(repairs[n]),
            'valid_repairs': sum(valid[(n, f)] for f in repairs[n]),
            'tied_repairs': sum(semantic[f] <= t_sem + TIE for f in repairs[n]),
            'runs_at_top': sum(b <= t_sem + TIE for b in best),
            'seeds_4x8': len(small.get(n, [])),
            'found_4x8': len(small_best),
            'best_semantic_4x8': f'{median(small_best):.6f}' if small_best else '',
            'closed_share_4x8': (f'{median([(b - t_sem) / (1 - t_sem) for b in small_best]):.4f}'
                                 if small_best and t_sem < 1 else ''),
        })
    with open(a.out, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(out_rows[0]))
        w.writeheader()
        w.writerows(out_rows)
    report(out_rows)


def report(rows):
    rng = random.Random(SEED)

    def line(title, rs):
        def col(key):
            return [float(r[key]) for r in rs if r[key] != '']
        parts = [f'{title} ({len(rs)} instances)']
        for key in ('top_semantic', 'best_semantic', 'closed_share', 'closed_rejection',
                    'closed_share_4x8'):
            xs = col(key)
            if xs:
                lo, hi = bootstrap_median(xs, rng)
                parts.append(f'{key} {statistics.median(xs):.4f} [{lo:.4f}, {hi:.4f}]')
        parts.append(f"repairs {sum(r['repairs'] for r in rs)}, valid "
                     f"{sum(r['valid_repairs'] for r in rs)}, tied with top "
                     f"{sum(r['tied_repairs'] for r in rs)}; runs at top "
                     f"{sum(r['runs_at_top'] for r in rs)} of {sum(r['found'] for r in rs)}; "
                     f"instances with a tied repair "
                     f"{sum(r['tied_repairs'] > 0 for r in rs)}")
        print('\n  '.join(parts))

    line('all', rows)
    for family in sorted({r['family'] for r in rows}):
        line(family, [r for r in rows if r['family'] == family])


if __name__ == '__main__':
    main()
