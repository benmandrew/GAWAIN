#!/usr/bin/env python3
"""Write the Promela models for the weakening search on the 20 BEEM pairs.

The 20 pairs are those of bench/results/beem-verdicts.csv where phi fails
on the erroneous variant and holds on its correct twin (bench/notes/beem.md).
For each, OUT/<instance>-<property>/model.pml holds dve.py's flat Promela
with one shadow bit per proposition `a`: `a_a` is set to the proposition's
value at the end of every d_step, including the initialising one, and
`#define p_a (a_a == 1)` is the atom. The SPIN back end then reads each atom
as a variable compared with a constant, and its stepwise mode
(`Spin::stepwise`) makes every d_step one position. OUT/pairs.tsv lists
name, model, atoms and phi, one pair per line, in the formula syntax of the
Rust crate.

usage: search_models.py BEEM_DATA VERDICTS_CSV OUT
"""
import csv
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dve  # noqa: E402

OPS = {'G', 'F', 'U', 'R', 'X', 'W', 'true', 'false'}


def pairs(verdicts):
    """The (row of the erroneous variant) for each of the 20 pairs."""
    rows = list(csv.DictReader(open(verdicts)))
    by = {(r['instance'], r['property'], r['variant']): r for r in rows}
    for (inst, prop, variant), r in by.items():
        twin = by.get((inst, prop, 'correct'))
        if (variant == 'error' and r['verdict'] == 'FALSE'
                and r['neg_verdict'] == 'FALSE' and twin and twin['verdict'] == 'TRUE'):
            yield r


def formula(text, aps):
    """A BEEM formula with each proposition `a` renamed to `p_a`."""
    def rename(m):
        name = m.group(0)
        if name in OPS:
            return name
        if name not in aps and set(name) <= OPS:  # GF p
            return ' '.join(name)
        if name not in aps:
            raise NameError(f'unknown proposition {name} in {text}')
        return f'p_{name}'
    return re.sub(r'[A-Za-z_][A-Za-z_0-9]*', rename, text)


def shadowed(pml, aps):
    """`pml` with a shadow bit per proposition, updated after every d_step."""
    updates = ''.join(f'; a_{a} = ({e} -> 1 : 0)' for a, e in aps.items())
    out = []
    for line in pml.splitlines():
        if line.startswith('active proctype'):
            out.extend(f'bit a_{a};' for a in aps)
        if 'd_step {' in line:
            head, tail = line.rsplit(' }', 1)
            line = f'{head}{updates} }}{tail}'
        out.append(line)
    out.extend(f'#define p_{a} (a_{a} == 1)' for a in aps)
    return '\n'.join(out) + '\n'


def main(data, verdicts, out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    lines = []
    for r in pairs(verdicts):
        model = r['instance'].split('.')[0]
        pml, _, aps = dve.instance(data, model, dve.parse_params(r['params']))
        phi = formula(r['formula'], aps)
        used = sorted(set(re.findall(r'p_([A-Za-z_0-9]+)', phi)))
        name = f"{r['instance']}-{r['property']}"
        d = out / name
        d.mkdir(exist_ok=True)
        (d / 'model.pml').write_text(shadowed(pml, {a: aps[a] for a in used}))
        atoms = ','.join(f'p_{a}' for a in used)
        lines.append(f'{name}\t{d / "model.pml"}\t{atoms}\t{phi}\n')
    (out / 'pairs.tsv').write_text(''.join(lines))
    print(f'{len(lines)} pairs in {out / "pairs.tsv"}')


if __name__ == '__main__':
    main(*sys.argv[1:])
