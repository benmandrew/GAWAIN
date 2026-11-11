#!/usr/bin/env python3
"""Check the orders the Lean proofs decide against Spot.

usage: check_lean_orders.py [MTL_CSV]

Runs `lake env lean --run Tables.lean` in proofs/, so it needs the proofs
shell (`nix develop '.#proofs'`). Each `mtl` row is a rule and case of
MTL_CSV (default bench/results/operator-orders-mtl.csv, written by
operator_orders.py), and its Lean verdict is compared with the Spot column.
Each `unary` row is a pair of words over X, F and G of length at most three,
and its Lean verdict (`word_exact`) is compared with a fresh Spot check.

As a control, each table is also compared with Spot read one row out of
step (MTL) or with the pair swapped (unary). Those must disagree somewhere,
or the check is not looking at the verdicts.
"""
import csv, os, subprocess, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from operator_orders import implications  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main():
    mtl_csv = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        ROOT, 'bench', 'results', 'operator-orders-mtl.csv')
    out = subprocess.run(['lake', 'env', 'lean', '--run', 'Tables.lean'],
                         cwd=os.path.join(ROOT, 'proofs'), capture_output=True,
                         text=True, check=True).stdout
    rows = [line.split('\t') for line in out.splitlines()]
    lean_mtl = {(r[1], r[2]): r[3] == '1' for r in rows if r[0] == 'mtl'}
    unary = [(r[1], r[2], r[3] == '1') for r in rows if r[0] == 'unary']

    with open(mtl_csv, newline='') as fh:
        spot_rows = list(csv.DictReader(fh))
    missing = [(s['rule'], s['case']) for s in spot_rows
               if (s['rule'], s['case']) not in lean_mtl]
    if missing or len(spot_rows) != len(lean_mtl):
        sys.exit(f'MTL grids differ: {len(spot_rows)} Spot rows, {len(lean_mtl)} Lean rows, '
                 f'first missing {missing[:3]}')
    keys = [(s['rule'], s['case']) for s in spot_rows]
    spot = [s['spot'] == '1' for s in spot_rows]
    lean = [lean_mtl[k] for k in keys]
    bad = [(k, l, s) for k, l, s in zip(keys, lean, spot) if l != s]
    shifted = sum(l != s for l, s in zip(lean, spot[1:] + spot[:1]))
    print(f'MTL: {len(keys)} cases, {sum(spot)} implications, {len(bad)} disagree; '
          f'control: {shifted} disagree one row out of step')
    for k, l, s in bad[:10]:
        print(f'  {k}: Lean {l}, Spot {s}')

    def render(w):
        return ''.join(f'{c}(' for c in w[:-1]) + 'p' + ')' * (len(w) - 1)
    verdicts = implications([(render(u), render(v)) for u, v, _ in unary])
    index = {(u, v): s for (u, v, _), s in zip(unary, verdicts)}
    ubad = [(u, v, l, s) for (u, v, l), s in zip(unary, verdicts) if l != s]
    swapped = sum(l != index[(v, u)] for u, v, l in unary)
    print(f'Unary: {len(unary)} pairs, {sum(verdicts)} implications, {len(ubad)} disagree; '
          f'control: {swapped} disagree with the pair swapped')
    for u, v, l, s in ubad[:10]:
        print(f'  {u} => {v}: Lean {l}, Spot {s}')

    if bad or ubad or not shifted or not swapped:
        sys.exit(1)


if __name__ == '__main__':
    main()
