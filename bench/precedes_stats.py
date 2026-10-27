#!/usr/bin/env python3
"""Place GAWAIN's repairs on the precedes set against the intended weakening.

Reads DIR from bench/precedes_prep.py (instances.tsv, reference.tsv,
models/) and the runs in DIR/RUNS of each ARM. For each instance and arm
it pools the repairs over the seeds, reduces them to the maximal set (as
bench/graded_stats.py does), and places the intended weakening ψ against
each member, modulo the encoding invariant I of GAWAIN's gate (one event
per position): `stronger` when the repair implies ψ, `weaker` when ψ
implies it. Per run it records whether some repair implies ψ, so is at
least as strong as the intended fix. A repair holds on M, so it cannot
imply a ψ that fails there. It also checks ψ on the translated model with
nuXmv, so a translation that disagrees with the recorded verdict shows,
and scores ψ and the best repair against φ under the rejection strength
(the `score` binary's third column).

Writes OUT (one row per instance and arm) and prints a summary per arm.
Spot 2.15.1 and nuXmv must be on PATH.

usage: precedes_stats.py DIR OUT [--runs runs-eval]
                         [--arms forced-default,forced-mutation] [--score PATH]
"""
import argparse
import csv
import json
import re
import statistics
import subprocess
from pathlib import Path

from baselines_cegiw import invariant, nuxmv_holds
from graded_stats import implications, maximal


def smv(f):
    """f in nuXmv's LTL syntax: Spot rewrites W, R and M with U, which
    nuXmv has (it writes R as V and has no W)."""
    out = subprocess.run(['ltlfilt', '--unabbreviate=WRM', '-p', '-f', f], capture_output=True,
                         text=True, check=True).stdout.strip()
    return re.sub(r'\b(true|false)\b', lambda m: m.group(1).upper(), out)


def rejection(score, phi, f):
    return float(subprocess.run([score], input=f'{phi}\t{f}\n', capture_output=True,
                                text=True, check=True).stdout.split()[2])


def main():
    p = argparse.ArgumentParser()
    p.add_argument('dir', type=Path)
    p.add_argument('out')
    p.add_argument('--runs', default='runs-eval')
    p.add_argument('--arms', default='forced-default,forced-mutation')
    p.add_argument('--score', default='target/release/score')
    a = p.parse_args()
    ref = {r['name']: r for r in csv.DictReader(open(a.dir / 'reference.tsv'), delimiter='\t')}
    rows = []
    for inst in csv.DictReader(open(a.dir / 'instances.tsv'), delimiter='\t'):
        name, phi = inst['name'], inst['phi']
        psi, verdict = ref[name]['weakening'], ref[name]['verdict']
        model = a.dir / inst['model']
        inv = invariant(model.read_text(), inst['atoms'].split(','))
        holds = nuxmv_holds(model, smv(psi))
        psi_rej = rejection(a.score, phi, psi)
        for arm in a.arms.split(','):
            runs = [json.loads(f.read_text().strip().splitlines()[-1])
                    for f in sorted((a.dir / a.runs).glob(f'{name}-{arm}-*.json'))]
            pooled = sorted({r['formula'] for run in runs for r in run['repairs']})
            pairs = [(x, y) for x in pooled for y in pooled if x != y]
            best = maximal(pooled, dict(zip(pairs, implications(pairs))))
            to_psi = dict(zip(pooled, implications([(f'({inv}) & ({f})', psi) for f in pooled])))
            from_psi = dict(zip(pooled, implications([(f'({inv}) & ({psi})', f) for f in pooled])))
            rel = [{(True, True): 'equivalent', (True, False): 'stronger',
                    (False, True): 'weaker', (False, False): 'incomparable'}[(to_psi[m], from_psi[m])]
                   for m in best]
            found = [run for run in runs if run['repairs']]
            reached = [run for run in found if any(to_psi[r['formula']] for r in run['repairs'])]
            best_rej = [max(rejection(a.score, phi, r['formula']) for r in run['repairs'])
                        for run in found]
            rows.append({
                'name': name, 'arm': arm, 'verdict': verdict, 'psi_holds_here': holds,
                'seeds': len(runs), 'found': len(found), 'reached_psi': len(reached),
                'maximal': len(best),
                **{k: rel.count(k) for k in ('stronger', 'equivalent', 'weaker', 'incomparable')},
                'psi_rejection': f'{psi_rej:.6f}',
                'best_rejection': f'{statistics.median(best_rej):.6f}' if best_rej else '',
                'maximal_set': ' ;; '.join(best),
            })
            print(f"{name} {arm}: ψ {verdict} (here {holds}), found {len(found)}/{len(runs)}, "
                  f"implies ψ on {len(reached)}, maximal {len(best)} {rel}, rejection ψ "
                  f"{psi_rej:.4f} best {rows[-1]['best_rejection']}", flush=True)
    with open(a.out, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    for arm in a.arms.split(','):
        rs = [r for r in rows if r['arm'] == arm]
        hold = [r for r in rs if r['verdict'] == 'holds']
        print(f"{arm}: found {sum(r['found'] for r in rs)}/{sum(r['seeds'] for r in rs)} runs; "
              f"on the {len(hold)} instances where ψ holds, a repair implies ψ on "
              f"{sum(r['reached_psi'] for r in hold)}/{sum(r['seeds'] for r in hold)} runs; "
              f"maximal repairs: {sum(r['stronger'] for r in rs)} stronger, "
              f"{sum(r['equivalent'] for r in rs)} equivalent, {sum(r['weaker'] for r in rs)} weaker, "
              f"{sum(r['incomparable'] for r in rs)} incomparable")


if __name__ == '__main__':
    main()
