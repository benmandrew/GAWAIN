#!/usr/bin/env python3
"""Run CEGIW on the cases of bench/sets/cegiw-cases.tsv, and prepare the
same cases for GAWAIN.

CEGIW (`~/projects/cegiw`, ABZ 2026, arXiv 2604.24540) weakens one interval
of an MTL formula against a nuXmv model. Each case names the model, the
formula with `{I}` in place of the interval to weaken, the initial interval
and the de Bruijn index that CEGIW takes to find it. A model is
`cegiw:<file>` (CEGIW's models/ directory) or `graded:<file>`
(bench/data/graded/models/).

The script writes into OUT:

- models/: a copy of each model.
- instances.tsv: the cases in the format of `bench/graded_run.py`, with φ
  the LTL translation of the MTL formula (CEGIW's own `mtl_to_ltl`, SPIN
  syntax), so GAWAIN and CEGIW start from the same property.
- cegiw/<name>-<rep>.log: CEGIW's output for each repetition.
- cegiw.csv: one row per case and repetition, with the outcome
  (`weakened`, `none` when CEGIW proves that no interval works, `holds`
  when φ already holds within CEGIW's bound, or `error`), the final
  interval, the weakened formula in MTL and in LTL, CEGIW's own total time
  and the wall time of the process.

It runs inside CEGIW's development shell, since it imports CEGIW's parser:

    env -C <copy of cegiw> nix develop -c python3 <this> CEGIW CASES OUT [--reps 3]

Each CEGIW run sits in a systemd scope capped at 8 GB, with 3600 s, one at
a time, so wall times are not disturbed by each other. Existing log files
are reused, so the script resumes.
"""
import argparse
import csv
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
TIMEOUT = 3600


def resolve(model, cegiw):
    kind, _, name = model.partition(':')
    root = {'cegiw': cegiw / 'models', 'graded': HERE / 'data' / 'graded' / 'models'}[kind]
    return root / name


def outcome(log):
    """The outcome, final interval and CEGIW's total time of one log."""
    total = re.search(r'Total time: ([0-9.]+) seconds', log)
    if 'No suitable weakening' in log:
        result = ('none', '')
    else:
        steps = re.findall(r'(\[[0-9]+,[0-9∞]+\]) → (\[[0-9]+,[0-9∞]+\]|Final)', log)
        if not steps or steps[-1][1] != 'Final':
            result = ('error', '')
        else:
            final = steps[-1][0]
            result = ('holds' if len(steps) == 1 else 'weakened', final)
    return result + (float(total.group(1)) if total else None,)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('cegiw', type=Path)
    p.add_argument('cases', type=Path)
    p.add_argument('out', type=Path)
    p.add_argument('--reps', type=int, default=3)
    a = p.parse_args()
    sys.path.insert(0, str(a.cegiw))
    from src import custom_args, mtl2ltlspec  # pylint: disable=import-outside-toplevel
    from src.logic import parser  # pylint: disable=import-outside-toplevel

    def ltl(mtl):
        return mtl2ltlspec.main(custom_args.ModelChecker.SPIN, parser.parse_mtl(mtl))

    cases = list(csv.DictReader(open(a.cases), delimiter='\t'))
    (a.out / 'models').mkdir(parents=True, exist_ok=True)
    (a.out / 'cegiw').mkdir(exist_ok=True)
    with open(a.out / 'instances.tsv', 'w') as f:
        f.write('name\tfamily\tbackend\tmodel\tatoms\tphi\tproperty\n')
        for c in cases:
            src = resolve(c['model'], a.cegiw)
            shutil.copy(src, a.out / 'models' / src.name)
            phi = ltl(c['mtl'].replace('{I}', c['interval']))
            f.write(f"{c['name']}\tcegiw\tnuxmv\tmodels/{src.name}\t{c['atoms']}\t{phi}\t\n")

    rows = []
    for rep in range(1, a.reps + 1):
        for c in cases:
            log_path = a.out / 'cegiw' / f"{c['name']}-{rep}.log"
            wall = None
            if not log_path.exists():
                mtl = c['mtl'].replace('{I}', c['interval'])
                cmd = ['systemd-run', '--user', '--scope', '-q', '-p', 'MemoryMax=8G',
                       '-p', 'MemorySwapMax=0', 'nice', '-n', '10', 'timeout', str(TIMEOUT),
                       sys.executable, '-m', 'src.iterative_weaken', '--model-checker', 'NUXMV',
                       '--model', str((a.out / 'models' / resolve(c['model'], a.cegiw).name).resolve()),
                       '--de-bruijn', c['de_bruijn'], '--mtl', mtl]
                start = time.time()
                r = subprocess.run(cmd, cwd=a.cegiw, capture_output=True, text=True)
                wall = time.time() - start
                log_path.write_text(f'# wall {wall:.3f}\n{r.stdout}{r.stderr}')
            log = log_path.read_text()
            wall = float(re.match(r'# wall ([0-9.]+)', log).group(1))
            kind, final, total = outcome(log)
            weakened = c['mtl'].replace('{I}', final) if final else ''
            rows.append({'name': c['name'], 'rep': rep, 'outcome': kind, 'interval': c['interval'],
                         'final': final, 'mtl': weakened, 'ltl': ltl(weakened) if weakened else '',
                         'cegiw_time': total if total is not None else '', 'wall': f'{wall:.3f}'})
            print(c['name'], rep, kind, final, total, f'{wall:.2f}', flush=True)
    with open(a.out / 'cegiw.csv', 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)


if __name__ == '__main__':
    main()
