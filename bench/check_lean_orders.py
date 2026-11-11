#!/usr/bin/env python3
"""Check the orders the Lean proofs decide against Spot.

usage: check_lean_orders.py [MTL_CSV]

Runs `lake env lean --run Tables.lean` in proofs/, so it needs the proofs
shell (`nix develop '.#proofs'`), and target/release/rule_table, built
first with `nix develop -c cargo build --release --bin rule_table`.

Each `mtl` row is a rule and case of MTL_CSV (default
bench/results/operator-orders-mtl.csv, written by operator_orders.py), and
its Lean verdict is compared with the Spot column. Each `unary` row is a
pair of words over X, F and G of length at most three, and its Lean verdict
(`word_exact`) is compared with a fresh Spot check. Each `rules` row lists
every outcome of `outs` (proofs/Gawain/Rules.lean) for one formula,
direction and rule set, and is compared, in order, with the list
`monotone_outcomes` gives for the same formula in the Rust code.

As a control, each table is also compared with Spot read one row out of
step (MTL), with the pair swapped (unary), or with the other rule set
(rules). Those must disagree somewhere, or the check is not looking at the
verdicts.
"""
import csv, os, subprocess, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from operator_orders import implications  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RULE_TABLE = os.path.join(ROOT, 'target', 'release', 'rule_table')
BINARY = {'and': '&', 'or': '|', 'imp': '->', 'iff': '<->', 'U': 'U', 'W': 'W', 'R': 'R'}


def parse_sexp(text):
    """An S-expression of rule_table or Tables.lean as nested tuples."""
    tokens = text.replace('(', ' ( ').replace(')', ' ) ').split()
    pos = 0

    def item():
        nonlocal pos
        tok = tokens[pos]
        pos += 1
        if tok != '(':
            return tok
        out = []
        while tokens[pos] != ')':
            out.append(item())
        pos += 1
        return tuple(out)
    return item()


def render(t):
    """A tuple from parse_sexp in GAWAIN's input syntax, fully bracketed."""
    if isinstance(t, str):
        return {'tt': 'true', 'ff': 'false'}.get(t, t)
    if t[0] == 'not': return f'!({render(t[1])})'
    if t[0] in ('X', 'F', 'G'): return f'{t[0]}({render(t[1])})'
    return f'({render(t[1])}) {BINARY[t[0]]} ({render(t[2])})'


def rule_table(sexps):
    """rule_table's rows for each formula, keyed by (sexp, dir, set)."""
    if not os.path.exists(RULE_TABLE):
        sys.exit(f'{RULE_TABLE} is missing: run '
                 "nix develop -c cargo build --release --bin rule_table")
    text = ''.join(render(parse_sexp(x)) + '\n' for x in sexps)
    out = subprocess.run([RULE_TABLE], input=text, capture_output=True, text=True, check=True)
    rows = {}
    for line in out.stdout.splitlines():
        f, d, rs, outs = line.split('\t')
        rows[(f, d, rs)] = outs
    return rows


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

    lean_rules = {(r[1], r[2], r[3]): r[4] if len(r) > 4 else '' for r in rows if r[0] == 'rules'}
    rust = rule_table(sorted({k[0] for k in lean_rules}))
    rbad = [k for k, v in lean_rules.items() if rust.get(k) != v]
    other = {'peredur': 'ordered', 'ordered': 'peredur'}
    crossed = sum(rust.get((f, d, other[rs])) != v for (f, d, rs), v in lean_rules.items())
    n_out = sum(len(v.split(' ; ')) for v in lean_rules.values() if v)
    print(f'Rules: {len(lean_rules)} rows, {n_out} outcomes, {len(rbad)} differ from rule_table; '
          f'control: {crossed} differ from the other rule set')
    for k in rbad[:5]:
        print(f'  {k}:\n    Lean {lean_rules[k][:300]}\n    Rust {rust.get(k, "missing")[:300]}')

    if bad or ubad or rbad or not shifted or not swapped or not crossed:
        sys.exit(1)


if __name__ == '__main__':
    main()
