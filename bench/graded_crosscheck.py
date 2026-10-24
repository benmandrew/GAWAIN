#!/usr/bin/env python3
"""Re-check the RERS repairs of the status ablation with SPIN.

The search decides RERS candidates on the explicit back end (the Kripke
structure of src/rers.rs and Spot's automaton). This script checks every
distinct X-free repair that any arm reported on a RERS instance against the
shipped Promela model instead: the model's `ltl` blocks are stripped, Spot's
`ltl2tgba --spin` builds the never claim of the negated repair, and SPIN
searches for an acceptance cycle (`pan -a`). An X-free formula is
stutter-invariant, so the hidden steps of the Promela model do not change its
verdict (see docs/dev/rers-kripke.md). A repair with X is skipped.

Appends one row per distinct (instance, repair) to OUT, skipping the rows
already there: the instance, the
repair, the arms that reported it, and SPIN's verdict (holds, violated or
unknown with a reason). A `violated` row is a disagreement between the two
back ends.

usage: graded_crosscheck.py DIR OUT [--runs runs] [--jobs 8]
"""
import argparse
import concurrent.futures as cf
import csv
import json
import re
import shutil
import subprocess
import tempfile
from collections import defaultdict
from pathlib import Path

LTL_RE = re.compile(r'^\s*ltl\s+\w+\s*\{.*\}\s*$')
TIMEOUT = 600
DEPTH = 10_000_000
MEMMAX_MB = 4096


def limited(cmd, seconds):
    return ['systemd-run', '--user', '--scope', '-q', '-p', f'MemoryMax={MEMMAX_MB}M',
            '-p', 'MemorySwapMax=0', 'nice', '-n', '10', 'timeout', str(seconds), *cmd]


def check(body, formula, tmp):
    """SPIN's verdict on `formula` over the model `body`."""
    work = Path(tempfile.mkdtemp(dir=tmp))
    try:
        r = subprocess.run(['ltl2tgba', '--spin', f'!({formula})'], capture_output=True,
                           text=True, timeout=TIMEOUT)
        if r.returncode != 0:
            return 'unknown', f'ltl2tgba exit {r.returncode}'
        (work / 'model.pml').write_text(f'{body}\n\n{r.stdout}\n')
        flags = ['-DCOLLAPSE', '-DVECTORSZ=8192', f'-DMEMLIM={MEMMAX_MB * 7 // 8}']
        for cmd in (['spin', '-a', 'model.pml'],
                    ['gcc', '-O2', *flags, '-w', '-o', 'pan', 'pan.c']):
            r = subprocess.run(limited(cmd, TIMEOUT), cwd=work, capture_output=True,
                               text=True, timeout=TIMEOUT + 60)
            if r.returncode != 0:
                return 'unknown', f'{cmd[0]} exit {r.returncode}'
        r = subprocess.run(limited(['./pan', '-a', f'-m{DEPTH}'], TIMEOUT), cwd=work,
                           capture_output=True, text=True, timeout=TIMEOUT + 60)
        out = r.stdout + r.stderr
        m = re.search(r'errors:\s*(\d+)', out)
        if m and int(m.group(1)) > 0:
            return 'violated', ''
        for mark in ('max search depth too small', 'Search not completed',
                     'reached -DMEMLIM bound', 'out of memory'):
            if mark in out:
                return 'unknown', mark
        if m:
            return 'holds', ''
        return 'unknown', f'pan exit {r.returncode}'
    except subprocess.TimeoutExpired:
        return 'unknown', 'timeout'
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('dir')
    p.add_argument('out')
    p.add_argument('--runs', default='runs')
    p.add_argument('--jobs', type=int, default=8)
    a = p.parse_args()
    d = Path(a.dir)
    model = {r['name']: d / r['model']
             for r in csv.DictReader(open(d / 'instances.tsv'), delimiter='\t')
             if r['family'] == 'rers'}
    arms = defaultdict(set)
    for path in (d / a.runs).glob('rers*.json'):
        name, arm, _ = path.stem.rsplit('-', 2)
        for repair in json.loads(path.read_text())['repairs']:
            arms[(name, repair['formula'])].add(arm)
    bodies = {}
    for name, pml in model.items():
        bodies[name] = '\n'.join(line for line in pml.read_text().splitlines()
                                 if not LTL_RE.match(line))
    keys = sorted(arms)
    out = Path(a.out)
    done = {}
    if out.exists():
        done = {(r['instance'], r['repair']): (r['spin'], r['reason'])
                for r in csv.DictReader(open(out))}
    todo = [k for k in keys if k not in done and not re.search(r'\bX\b', k[1])]
    tmp = tempfile.mkdtemp(prefix='crosscheck-')
    # Rows are written as they finish, so an interrupted run resumes.
    with open(out, 'a', newline='') as f, cf.ThreadPoolExecutor(a.jobs) as pool:
        w = csv.writer(f)
        if not done:
            w.writerow(['instance', 'repair', 'arms', 'spin', 'reason'])
        for k in keys:
            if k not in done and re.search(r'\bX\b', k[1]):
                done[k] = ('skipped', 'uses X')
                w.writerow([k[0], k[1], ' '.join(sorted(arms[k])), *done[k]])
        jobs = {pool.submit(check, bodies[k[0]], k[1], tmp): k for k in todo}
        for job in cf.as_completed(jobs):
            k = jobs[job]
            done[k] = job.result()
            w.writerow([k[0], k[1], ' '.join(sorted(arms[k])), *done[k]])
            f.flush()
    shutil.rmtree(tmp, ignore_errors=True)
    counts = defaultdict(int)
    for k in keys:
        counts[done[k][0]] += 1
    print(f'{len(keys)} distinct repairs: {dict(counts)}')


if __name__ == '__main__':
    main()
