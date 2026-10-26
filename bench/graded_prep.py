#!/usr/bin/env python3
"""Build the instance set for the binary-against-graded ablation.

Writes OUT/models/ with every model the runs need, and OUT/instances.tsv
with one instance per line: name, family, back end, model (relative to
OUT), atoms, phi and MCC property id (empty where unused). The sample is
fixed so that a rerun builds the same set:

- LMCS-2006: abp4 `p2false` with the DEFINEs of tests/search.rs. Every
  other surviving LMCS failure uses past-time operators (O, S, T), which
  the formula core does not have.
- RERS 2016 parallel problems 101 to 106, every property that survives the
  filter (bench/results/rers-parallel-filter.csv); explicit back end.
- RERS 2018 parallel problem 101, every property that survives the SPIN
  filter or, where SPIN timed out on !phi, the explicit check in
  bench/results/rers-2018-explicit.csv; explicit back end. Problem 102 did
  not finish exploring in 50 minutes and 85 GB, and 103 exceeds the 128-bit
  state key.
- BEEM: the 20 pairs of bench/beem/search_models.py; SPIN, stepwise.
- MCC: 20 properties of bench/results/mcc-subset.csv, drawn with seed 1,
  one per instance, among those with at least 2 atoms and with phi and
  !phi each checked in under 1 s; TAPAAL. The instances are fetched with
  bench/mcc_fetch.sh.

usage: graded_prep.py OUT
"""
import csv
import random
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE / 'data'
RESULTS = HERE / 'results'
RERS = DATA / 'rers/2016/ParallelRERS2016_version2/ParallelRERS2016_version2'
RERS2018 = DATA / 'rers/2018/ParallelLtlRers2018WithPml/ParallelLtlRers2018WithPml'
ABP4_DEFINES = """
DEFINE
  s_get := sender.state = get;
  s_wait := sender.state = wait_for_ack;
  s_abp := sender.abp;
  r_ack0 := r2s_out.tag = ack0;
  r_ack1 := r2s_out.tag = ack1;
"""
ABP4_PHI = 'G (s_get -> X (!s_get U (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1)))))'
MCC_SAMPLE = 20


def rers_properties(pml):
    """{number: formula} of the `ltl pN { ... }` blocks of a RERS file."""
    out = {}
    for line in pml.read_text().splitlines():
        m = re.match(r'ltl p(\d+)\s*\{(.*)\}\s*$', line.strip())
        if m:
            out[int(m.group(1))] = m.group(2).strip()
    return out


def main(out):
    out = Path(out)
    models = out / 'models'
    models.mkdir(parents=True, exist_ok=True)
    rows = []

    abp4 = models / 'abp4-defines.smv'
    abp4.write_text((DATA / 'lmcs2006/fixed/abp4/abp4.smv').read_text() + ABP4_DEFINES)
    rows.append(('abp4-p2false', 'lmcs', 'nuxmv', 'models/abp4-defines.smv',
                 's_get,s_wait,s_abp,r_ack0,r_ack1', ABP4_PHI, ''))

    survivors = [r for r in csv.DictReader(open(RESULTS / 'rers-parallel-filter.csv'))
                 if r['year'] == '2016' and int(r['problem']) <= 106 and r['survives'] == 'yes']
    for problem in sorted({int(r['problem']) for r in survivors}):
        src = RERS / f'problem{problem}/problem{problem}.pml'
        shutil.copy(src, models / src.name)
        props = rers_properties(src)
        for r in survivors:
            if int(r['problem']) == problem:
                p = int(r['prop'])
                rows.append((f'rers2016-{problem}-p{p}', 'rers', 'explicit',
                             f'models/{src.name}', '', props[p], ''))

    survivors = [r for name in ('rers-parallel-filter.csv', 'rers-2018-explicit.csv')
                 for r in csv.DictReader(open(RESULTS / name))
                 if r['year'] == '2018' and r['problem'] == '101' and r['survives'] == 'yes']
    src = RERS2018 / 'problem101/problem101.pml'
    shutil.copy(src, models / 'rers2018-problem101.pml')
    props = rers_properties(src)
    for p in sorted(int(r['prop']) for r in survivors):
        rows.append((f'rers2018-101-p{p}', 'rers', 'explicit',
                     'models/rers2018-problem101.pml', '', props[p], ''))

    beem = models / 'beem'
    subprocess.run([sys.executable, str(HERE / 'beem/search_models.py'),
                    str(DATA / 'beem/data'), str(RESULTS / 'beem-verdicts.csv'), str(beem)],
                   check=True)
    for line in (beem / 'pairs.tsv').read_text().splitlines():
        name, model, atoms, phi = line.split('\t')
        rows.append((f'beem-{name}', 'beem', 'spin',
                     str(Path(model).relative_to(out)), atoms, phi, ''))

    mcc = [r for r in csv.DictReader(open(RESULTS / 'mcc-subset.csv'))
           if int(r['atoms']) >= 2 and float(r['seconds']) < 1 and float(r['neg_seconds']) < 1]
    by_instance = {}
    for r in mcc:
        by_instance.setdefault(r['instance'], []).append(r)
    rng = random.Random(1)
    chosen = [rng.choice(by_instance[i]) for i in rng.sample(sorted(by_instance), MCC_SAMPLE)]
    instances = sorted({r['instance'] for r in chosen})
    subprocess.run(['sh', str(HERE / 'mcc_fetch.sh'), *instances], check=True)
    for r in sorted(chosen, key=lambda r: r['formula']):
        inst = r['instance']
        dest = models / 'mcc' / inst
        dest.mkdir(parents=True, exist_ok=True)
        exam = r['formula'].rsplit('-', 2)[-2]
        for f in ('model.pnml', f'{exam}.xml'):
            shutil.copy(DATA / 'mcc/inputs' / inst / f, dest / f)
        rows.append((f"mcc-{r['formula']}", 'mcc', 'tapaal', f'models/mcc/{inst}', '', '',
                     r['formula']))

    with open(out / 'instances.tsv', 'w') as f:
        f.write('name\tfamily\tbackend\tmodel\tatoms\tphi\tproperty\n')
        for row in rows:
            f.write('\t'.join(row) + '\n')
    print(f'{len(rows)} instances in {out / "instances.tsv"}')


if __name__ == '__main__':
    main(*sys.argv[1:])
