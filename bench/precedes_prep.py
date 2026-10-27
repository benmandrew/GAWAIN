#!/usr/bin/env python3
"""Build the instance set of the precedes reference run.

The RERS 2016/2017 "precedes" cases (bench/sets/rers-precedes.csv,
bench/notes/rers-precedes.md) each carry an intended weakening, the Dwyer
pattern that the shipped formula mistranslates, with a decided verdict on
the model. GAWAIN searches from the shipped formula, and
bench/precedes_stats.py places its repairs against the intended one.

GAWAIN reaches these C programs only through the SMV translation of
bench/rers_seq_smv.py and its nuXmv back end. A run is affordable only on
problems whose translation has no `signed word[32]` variable: 2016/1 and
2017/1 search in under a second, while a 4x8 search ran past 900 s on
every problem with words and past 700 s on 2016/4 and 2017/4. The default
--problems are those two.

Each model gets `FAIRNESS !dead`, so the check covers only the runs that
never take an invalid input, as RERS reads its properties. The
translation's start position, where no event atom holds, stays in. All
cases are degenerate (M satisfies the negation of the shipped formula), so
the runs use the `forced-` arms of bench/graded_run.py.

Writes OUT/models/, OUT/instances.tsv in the format of bench/graded_prep.py,
and OUT/reference.tsv (name, intended weakening, its verdict).

usage: precedes_prep.py OUT [--problems 2016:1 2017:1]
"""
import argparse
import csv
import re
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main():
    p = argparse.ArgumentParser()
    p.add_argument('out', type=Path)
    p.add_argument('--problems', nargs='*', default=['2016:1', '2017:1'])
    a = p.parse_args()
    (a.out / 'models').mkdir(parents=True, exist_ok=True)
    cases = list(csv.DictReader(open(HERE / 'sets/rers-precedes.csv')))
    instances, reference = [], []
    for yp in a.problems:
        year, prob = yp.split(':')
        smv = subprocess.run(['python3', str(HERE / 'rers_seq_smv.py'), 'model',
                              '--years', year, '--problems', prob],
                             capture_output=True, text=True, check=True).stdout
        if 'word[' in smv:
            raise SystemExit(f'{yp}: the translation has word variables')
        model = f'models/rers-seq-{year}-{prob}.smv'
        (a.out / model).write_text(smv + 'FAIRNESS\n  !dead;\n')
        atoms = ','.join(re.findall(r'^  ([io][A-Z]) :=', smv, re.M))
        for c in cases:
            if (c['year'], c['problem']) != (year, prob):
                continue
            name = f'rers-seq-{year}-{prob}-p{c["prop"]}'
            instances.append([name, 'rers-seq', 'nuxmv', model, atoms, c['phi'], ''])
            reference.append([name, c['weakening'], c['weakening_verdict']])
    with open(a.out / 'instances.tsv', 'w', newline='') as f:
        w = csv.writer(f, delimiter='\t', lineterminator='\n')
        w.writerow(['name', 'family', 'backend', 'model', 'atoms', 'phi', 'property'])
        w.writerows(instances)
    with open(a.out / 'reference.tsv', 'w', newline='') as f:
        w = csv.writer(f, delimiter='\t', lineterminator='\n')
        w.writerow(['name', 'weakening', 'verdict'])
        w.writerows(reference)
    print(f'{len(instances)} instances')


if __name__ == '__main__':
    main()
