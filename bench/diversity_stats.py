#!/usr/bin/env python3
"""Diversity of each arm's repairs against the baseline's, as PEREDUR's
paper measures it: the net containment share over pooled maximal sets.

Reads DIR/instances.tsv and DIR/RUNS/*.json as bench/graded_stats.py does,
writes OUT/diversity-instances.csv (one row per instance and arm) and
prints, per arm, over all instances and per family, the three tests of
PEREDUR's protocol. A is the arm and B the baseline.

Distance. `target/release/diversity` (src/bin/diversity.rs) samples 2^16
traces over the instance's atoms, lassos with a stem of at most 8 steps and
a loop of at most 8 (finite words of 1 to 16 steps where the `semantics`
column is `finite`), and prints |a xor b| / |a or b| over the satisfying
sets of every pair of the instance's repairs.

Containment (Kynkaenniemi et al.'s improved recall). Each repair of a set
has a ball reaching to its K-th nearest neighbour in that set (K = 3; the
repair itself is at distance 0 and does not count). The containment of B in
A is the share of B's repairs inside the union of A's balls.

Pooling and the control, as PEREDUR's pooled_div2.py. The seeds both arms
have run are split at random into halves S and S' (floor and ceiling when
their number is odd), SPLITS times. Each side's repairs over one half are
pooled and reduced to their maximal set by graded_stats' implication
machinery and its cache, OUT/implications.json. Cross containment pairs A
on S with B on S' and A on S' with B on S, so no cross pair shares a seed;
the controls are A on S against A on S', and likewise B, both ways. All
four sets are subsampled to a common size m, the smallest of the four
capped at CAP (200), SUBSAMPLES (2) times per split, and every figure is
the mean over splits and subsamples. A split enters only when m > K, so
both sides' maximal sets hold more than K repairs; an instance enters when
some split does.

Tests, over the instances that enter: the net containment share
B_in_A - A_in_B (positive when A's repairs spread wider), and each
direction against the control of the side whose region it is measured in,
B_in_A - ctrl_A and A_in_B - ctrl_B. Each gives the median, the mean, the
Hodges-Lehmann shift with its exact 95% interval, the exact signed-rank p,
that p Holm-adjusted over the three tests of the block, the rank-biserial
r and the signs. Only the net share is in PEREDUR's table; the directions
are its direction_ctrl.py reading.

usage: diversity_stats.py DIR OUT --runs RUNS --baseline ARM [--k 3]
       [--splits 5] [--subsamples 2] [--cap 200] [--filter REGEX] [--jobs J]
       [--bin target/release/diversity] [--seed 1]
       diversity_stats.py --self-check
"""
import argparse
import concurrent.futures as cf
import csv
import json
import math
import random
import re
import statistics
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import graded_stats as G  # noqa: E402

TESTS = [('net B_in_A - A_in_B', 'net'), ('B_in_A - ctrl_A', 'b_vs_ctrl'),
         ('A_in_B - ctrl_B', 'a_vs_ctrl')]


def radii(dist, xs, k):
    """Each x's distance to its k-th nearest member of xs, x itself first."""
    return [sorted(dist[x][y] for y in xs)[k] for x in xs]


def containment(dist, a, b, k):
    """The share of b inside the union of a's balls."""
    r = radii(dist, a, k)
    return sum(any(dist[x][y] <= rx for x, rx in zip(a, r)) for y in b) / len(b)


def contain(dist, a, b, rng, m, k):
    """containment after subsampling a and b to m each, a first, as
    pooled_div2.contain draws them."""
    a = sorted(rng.sample(a, m))
    b = sorted(rng.sample(b, m))
    return containment(dist, a, b, k)


def distances(binary, formulas, atoms, finite, seed):
    """The diversity binary's distance matrix over formulas, by index."""
    cmd = [binary, '--seed', str(seed), '--semantics', 'finite' if finite else 'infinite']
    if atoms:
        cmd += ['--atoms', atoms]
    r = subprocess.run(cmd, input=''.join(f + '\n' for f in formulas), capture_output=True,
                       text=True, check=True)
    return json.loads(r.stdout)['distance']


def diversity(pool_a, pool_b, imp, dist, seeds, rng, args):
    """The pooled figures of one instance: pool_x maps each repair (an index
    into dist) to the seeds that found it. None when no split enters."""
    acc, ms = defaultdict(list), []
    half = len(seeds) // 2
    for _ in range(args.splits):
        s = set(rng.sample(seeds, half))
        sc = set(seeds) - s
        f = {}
        for side, pool in (('A', pool_a), ('B', pool_b)):
            for h, hs in (('S', s), ('Sc', sc)):
                f[side + h] = G.maximal(sorted(x for x, ss in pool.items() if ss & hs), imp)
        m = min(min(len(v) for v in f.values()), args.cap)
        if m <= args.k:
            continue
        ms.append(m)
        k = args.k
        for _ in range(args.subsamples):
            acc['B_in_A'] += [contain(dist, f['AS'], f['BSc'], rng, m, k),
                              contain(dist, f['ASc'], f['BS'], rng, m, k)]
            acc['A_in_B'] += [contain(dist, f['BSc'], f['AS'], rng, m, k),
                              contain(dist, f['BS'], f['ASc'], rng, m, k)]
            acc['ctrl_A'] += [contain(dist, f['AS'], f['ASc'], rng, m, k),
                              contain(dist, f['ASc'], f['AS'], rng, m, k)]
            acc['ctrl_B'] += [contain(dist, f['BS'], f['BSc'], rng, m, k),
                              contain(dist, f['BSc'], f['BS'], rng, m, k)]
    if not ms:
        return None
    return {'splits': len(ms), 'm': statistics.median(ms),
            **{key: statistics.mean(v) for key, v in acc.items()}}


def measure(job, args, atoms, finite):
    arm, name, seeds, by = job
    t0 = time.time()
    pools = []
    for side in (arm, args.baseline):
        pool = defaultdict(set)
        for s in seeds:
            for f in by[(name, side)][s]['formulas']:
                pool[f].add(s)
        pools.append(pool)
    formulas = sorted(set(pools[0]) | set(pools[1]))
    row = {'instance': name, 'arm': arm, 'seeds': len(seeds)}
    if not formulas:
        return row
    index = {f: i for i, f in enumerate(formulas)}
    pairs = [(x, y) for x in formulas for y in formulas if x != y]
    imp_f = dict(zip(pairs, G.implications(pairs, finite)))
    imp = {(index[x], index[y]): v for (x, y), v in imp_f.items()}
    G.save_cache(args.out / 'implications.json', every=1800)
    whole = [G.maximal(sorted(index[f] for f in p), imp) for p in pools]
    row.update(maximal=len(whole[0]), baseline_maximal=len(whole[1]))
    dist = distances(args.bin, formulas, atoms, finite, args.seed)
    pool_a, pool_b = ({index[f]: ss for f, ss in p.items()} for p in pools)
    rng = random.Random(f'{name}|{arm}|{args.baseline}')
    got = diversity(pool_a, pool_b, imp, dist, seeds, rng, args)
    if got is not None:
        row.update(got)
        row.update(net=got['B_in_A'] - got['A_in_B'], b_vs_ctrl=got['B_in_A'] - got['ctrl_A'],
                   a_vs_ctrl=got['A_in_B'] - got['ctrl_B'])
    row['secs'] = round(time.time() - t0, 2)
    return row


def report(title, rows):
    entered = [r for r in rows if 'net' in r]
    print(f'\n## {title}: {len(entered)} of {len(rows)} instances enter')
    if not entered:
        return
    means = {k: statistics.mean(r[k] for r in entered)
             for k in ('B_in_A', 'A_in_B', 'ctrl_A', 'ctrl_B')}
    print('mean containment: ' + ', '.join(f'{k} {v:.3f}' for k, v in means.items()))
    print(f'{"test":20} {"n":>3} {"median":>7} {"mean":>7} {"HL shift [exact 95% CI]":>26} '
          f'{"p exact":>8} {"p Holm":>7} {"r_rb":>6} {"+/-/0":>9}')
    got = []
    for label, key in TESTS:
        d = [r[key] for r in entered]
        got.append((label, d, G.hodges_lehmann(d), G.wilcoxon_exact(d)))
    for (label, d, (est, lo, hi), p), held in zip(got, G.holm([g[3] for g in got])):
        signs = f'{sum(x > 0 for x in d)}/{sum(x < 0 for x in d)}/{sum(x == 0 for x in d)}'
        ci = f'{G.fmt(est)} [{G.fmt(lo)}, {G.fmt(hi)}]'
        print(f'{label:20} {len(d):3d} {G.fmt(statistics.median(d)):>7} '
              f'{G.fmt(statistics.mean(d)):>7} {ci:>26} {p:8.4f} {held:7.4f} '
              f'{G.fmt(G.rank_biserial(d), 2):>6} {signs:>9}')


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    p.add_argument('dir', type=Path)
    p.add_argument('out', type=Path)
    p.add_argument('--runs', default='runs')
    p.add_argument('--baseline', required=True)
    p.add_argument('--k', type=int, default=3)
    p.add_argument('--splits', type=int, default=5)
    p.add_argument('--subsamples', type=int, default=2)
    p.add_argument('--cap', type=int, default=200)
    p.add_argument('--filter', default='', help='only instances whose name this regex matches')
    p.add_argument('--jobs', type=int, default=1)
    p.add_argument('--bin', default=str(HERE.parent / 'target/release/diversity'))
    p.add_argument('--seed', type=int, default=1, help='the trace sample seed')
    args = p.parse_args(argv)
    instances = list(csv.DictReader(open(args.dir / 'instances.tsv'), delimiter='\t'))
    family = {r['name']: r['family'] for r in instances}
    atoms = {r['name']: r.get('atoms', '') for r in instances}
    finite = {r['name'] for r in instances if r.get('semantics') == 'finite'}
    keep = re.compile(args.filter)
    by = defaultdict(dict)
    for path in sorted((args.dir / args.runs).glob('*.json')):
        name, arm, seed = G.parse_run_name(path.stem, family)
        if keep.search(name):
            r = json.loads(path.read_text())
            by[(name, arm)][int(seed)] = {'formulas': [x['formula'] for x in r['repairs']]}
    arms = sorted({a for _, a in by} - {args.baseline})
    jobs = []
    for arm in arms:
        for name in sorted({n for n, a in by if a == arm}):
            seeds = sorted(set(by[(name, arm)]) & set(by.get((name, args.baseline), {})))
            if len(seeds) >= 2:
                jobs.append((arm, name, seeds, by))
    args.out.mkdir(parents=True, exist_ok=True)
    G.load_cache(args.out / 'implications.json')
    with cf.ThreadPoolExecutor(args.jobs) as pool:
        rows = list(pool.map(lambda j: measure(j, args, atoms[j[1]], j[1] in finite), jobs))
    G.save_cache(args.out / 'implications.json')
    for r in rows:
        r['family'] = family[r['instance']]
    fields = ['instance', 'family', 'arm', 'seeds', 'maximal', 'baseline_maximal', 'splits', 'm',
              'B_in_A', 'A_in_B', 'ctrl_A', 'ctrl_B', 'net', 'b_vs_ctrl', 'a_vs_ctrl', 'secs']
    with open(args.out / 'diversity-instances.csv', 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)
    print(f'{len(rows)} instance comparisons, K = {args.k}, {args.splits} splits, '
          f'{args.subsamples} subsamples, cap {args.cap}; '
          f'{len(G.UNDECIDED)} implication pairs undecided, read as not implied')
    for arm in arms:
        mine = [r for r in rows if r['arm'] == arm]
        report(f'{arm} (A) against {args.baseline} (B): all', mine)
        for fam in sorted({r['family'] for r in mine}):
            report(f'{arm} (A) against {args.baseline} (B): {fam}',
                   [r for r in mine if r['family'] == fam])
    if G.HL_MISMATCH[0]:
        print(f'\n{G.HL_MISMATCH[0]} Hodges-Lehmann intervals disagree with the exact p at 0.05')


def self_check():
    """Hand-computed cases for containment and the paired statistics."""
    # Six points on a line at 0, 1, 2, 3, 4 and 9, distance |x - y| / 10:
    # A is the first three, B the last three.
    pts = [0, 1, 2, 3, 4, 9]
    dist = [[abs(x - y) / 10 for y in pts] for x in pts]
    a, b = [0, 1, 2], [3, 4, 5]
    # K = 1: A's radii are 0.1 each, so its region holds B's 3 (0.1 from 2)
    # but not 4 or 9. B's radii are 0.1, 0.1 and 0.5 (9 to 4): its region
    # holds A's 2 (0.1 from 3) but not 1 or 0.
    assert radii(dist, a, 1) == [0.1, 0.1, 0.1]
    assert containment(dist, a, b, 1) == 1 / 3
    assert containment(dist, b, a, 1) == 1 / 3
    # K = 2: A's radii are 0.2, 0.1 and 0.2, so 2's ball takes in 3 and 4.
    # B's are 0.6, 0.5 and 0.6, and 3's ball takes in all of A.
    assert radii(dist, a, 2) == [0.2, 0.1, 0.2]
    assert containment(dist, a, b, 2) == 2 / 3
    assert containment(dist, b, a, 2) == 1.0
    # Hodges-Lehmann on 1..10: the 55 Walsh averages have median 5.5.
    # P(W <= 8) = 25/1024 <= 0.025 < P(W <= 9) = 33/1024, so the interval
    # is the 9th smallest and 9th largest average, [3.0, 8.0], as in
    # Hollander and Wolfe's table; the two-sided exact p is 2/1024.
    est, lo, hi = G.hodges_lehmann(list(range(1, 11)))
    assert (est, lo, hi) == (5.5, 3.0, 8.0), (est, lo, hi)
    assert abs(G.wilcoxon_exact(list(range(1, 11))) - 2 / 1024) < 1e-12
    # W- = 9 on (1..8, -9, 10): p = 66/1024 > 0.05, so the interval must
    # hold zero. PEREDUR's helper gives [0.5, 7.0] here.
    d = [1, 2, 3, 4, 5, 6, 7, 8, -9, 10]
    est, lo, hi = G.hodges_lehmann(d)
    assert lo <= 0 <= hi and abs(G.wilcoxon_exact(d) - 66 / 1024) < 1e-12, (lo, hi)
    assert G.hodges_lehmann([1, 2, 3])[1:] == (-math.inf, math.inf)
    # Mixed signs: d = (-1, 2, 3, 4, 5); W+ = 14, two-sided p = 4/32.
    assert G.wilcoxon_exact([-1, 2, 3, 4, 5]) == 4 / 32
    assert G.rank_biserial([-1, 2, 3, 4, 5]) == (14 - 1) / 15
    assert G.holm([0.01, 0.04, 0.03]) == [0.03, 0.06, 0.06]
    # x implies y is proved and the converse undecided. The current reading
    # takes x as strictly stronger, so net = 1; PEREDUR's `non` drops the
    # pair, net 0; its `imp` subsumes both x and y, net 1 - 1/2.
    imp = {('x', 'y'): True, ('y', 'x'): False, ('z', 'y'): False, ('y', 'z'): False}
    und = {('x', 'y'), ('y', 'x')}
    assert G.net_share(['x', 'z'], ['y'], imp) == 1.0
    assert G.net_share_readings(['x', 'z'], ['y'], imp, und) == (0.0, 0.5)
    print('self-check passed')


if __name__ == '__main__':
    if sys.argv[1:] == ['--self-check']:
        self_check()
    else:
        main(sys.argv[1:])
