#!/usr/bin/env python3
"""Rescore the ⊤ baseline's repairs with the rejection strength.

The rejection strength of φ' is log2 count(¬φ') / log2 count(¬φ), the log
share of φ's violating lassos that φ' still rejects (`Counter::rejections`).
⊤ scores 0 and φ 1. This script scores every distinct repair of the binary
runs of `bench/baselines_top.py` against φ with the `score` binary, which
prints the semantic similarity, the syntactic similarity and the rejection
strength to 15 places, and ⊤ alongside.

Inputs:

- RUNS: the directory of search_bench JSON lines named
  <instance>-binary-<seed>.json (`bench/data/baselines/top-runs`).
- CSV: bench/results/graded-runs.csv, for the family of each instance.

Writes OUT, one row per (instance, repair) and one per instance for ⊤
(`repair` = `true`), and prints: the repairs that tie with ⊤ under each
score, the repairs that reject no lasso of length 20, the instances where
⊤'s semantic similarity rounds to 1.000000, and the median over instances
of Kendall's tau-b and Spearman's rho between the two scores of the
repairs. `--jobs J` (default 6) scores J instances at once, one `score`
process each.

usage: baselines_rejection.py RUNS CSV OUT [--score PATH] [--jobs J]
"""
import argparse
import concurrent.futures as cf
import csv
import json
import re
import statistics
import subprocess
from collections import defaultdict
from pathlib import Path


def load(runs_dir):
    """φ and the sorted distinct repairs of each instance."""
    phi, repairs = {}, defaultdict(set)
    for path in sorted(Path(runs_dir).glob('*-binary-*.json')):
        name = re.sub(r'-binary-\d+\.json$', '', path.name)
        run = json.loads(path.read_text().strip().splitlines()[-1])
        if phi.setdefault(name, run['phi']) != run['phi']:
            raise SystemExit(f'{path}: φ differs from the other seeds')
        repairs[name].update(r['formula'] for r in run['repairs'])
    return phi, {n: sorted(repairs[n]) for n in phi}


def score(binary, phi, candidates):
    """The (semantic, syntactic, rejection) scores of each candidate, and
    the stderr of the `score` binary."""
    text = ''.join(f'{phi}\t{c}\n' for c in candidates)
    r = subprocess.run([binary, '--timeout', '120'], input=text, capture_output=True,
                       text=True, check=True)
    return [tuple(map(float, line.split('\t'))) for line in r.stdout.splitlines()], r.stderr


def ranks(xs):
    """Average ranks, ties sharing the mean of their positions."""
    order = sorted(range(len(xs)), key=lambda i: xs[i])
    out = [0.0] * len(xs)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and xs[order[j + 1]] == xs[order[i]]:
            j += 1
        for k in range(i, j + 1):
            out[order[k]] = (i + j) / 2
        i = j + 1
    return out


def spearman(xs, ys):
    rx, ry = ranks(xs), ranks(ys)
    mx, my = statistics.mean(rx), statistics.mean(ry)
    num = sum((a - mx) * (b - my) for a, b in zip(rx, ry))
    den = (sum((a - mx) ** 2 for a in rx) * sum((b - my) ** 2 for b in ry)) ** 0.5
    return num / den if den else None


def kendall(xs, ys):
    """Kendall's tau-b, None when either side is constant."""
    concordant = discordant = tie_x = tie_y = 0
    for i in range(len(xs)):
        for j in range(i + 1, len(xs)):
            dx, dy = xs[i] - xs[j], ys[i] - ys[j]
            if dx == 0 and dy == 0:
                continue
            if dx == 0:
                tie_x += 1
            elif dy == 0:
                tie_y += 1
            elif (dx > 0) == (dy > 0):
                concordant += 1
            else:
                discordant += 1
    den = ((concordant + discordant + tie_x) * (concordant + discordant + tie_y)) ** 0.5
    return (concordant - discordant) / den if den else None


def main():
    p = argparse.ArgumentParser()
    p.add_argument('runs')
    p.add_argument('csv')
    p.add_argument('out')
    p.add_argument('--score', default='target/release/score')
    p.add_argument('--jobs', type=int, default=6)
    a = p.parse_args()

    phi, repairs = load(a.runs)
    family = {r['instance']: r['family'] for r in csv.DictReader(open(a.csv))}
    names = sorted(phi)
    with cf.ThreadPoolExecutor(a.jobs) as pool:
        futures = {n: pool.submit(score, a.score, phi[n], ['true'] + repairs[n]) for n in names}
        scored = {n: futures[n].result() for n in names}

    rows, taus, rhos = [], [], []
    tied_semantic = tied_rejection = empty = 0
    for n in names:
        found, stderr = scored[n]
        if stderr.strip():
            print(f'{n}: {stderr.strip()}')
        (top_sem, _, top_rej), reps = found[0], found[1:]
        for f, (sem, syn, rej) in zip(['true'] + repairs[n], found):
            rows.append({'instance': n, 'family': family[n], 'phi': phi[n], 'repair': f,
                         'semantic': f'{sem:.15f}', 'syntactic': f'{syn:.6f}',
                         'rejection': f'{rej:.15f}'})
        tied_semantic += sum(s <= top_sem + 1e-6 for s, _, _ in reps)
        tied_rejection += sum(r <= top_rej for _, _, r in reps)
        empty += sum(r == 0.0 for _, _, r in reps)
        if round(top_sem, 6) == 1.0:
            print(f'{n}: ⊤ semantic {top_sem:.15f}; repairs by rejection: '
                  + '; '.join(f'{r:.6f} (semantic {s:.12f})'
                              for s, _, r in sorted(reps, key=lambda x: -x[2])))
        if len(reps) >= 2:
            sems, rejs = [s for s, _, _ in reps], [r for _, _, r in reps]
            for xs, f in ((taus, kendall), (rhos, spearman)):
                v = f(sems, rejs)
                if v is not None:
                    xs.append(v)
    with open(a.out, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    total = sum(len(repairs[n]) for n in names)
    print(f'{total} repairs on {len(names)} instances: {tied_semantic} tie with ⊤ in semantic '
          f'(to six places), {tied_rejection} in rejection, {empty} reject no lasso')
    for title, xs in (('Kendall tau-b', taus), ('Spearman rho', rhos)):
        q = statistics.quantiles(xs, n=4)
        print(f'{title} over {len(xs)} instances: median {statistics.median(xs):.3f}, '
              f'quartiles {q[0]:.3f} {q[2]:.3f}, {sum(x < 0 for x in xs)} negative')


if __name__ == '__main__':
    main()
