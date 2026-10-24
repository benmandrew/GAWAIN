#!/usr/bin/env python3
"""Run the binary-against-graded ablation.

Every instance of DIR/instances.tsv (from bench/graded_prep.py) runs under
every arm and seed, JOBS at a time, one search_bench process per run. A run
writes DIR/runs/<name>-<arm>-<seed>.json, or <...>.err with its stderr when
it prints no JSON line (a timeout or a crash). A run whose JSON exists is
skipped, so the script resumes. Runs are ordered seed-major with the arms
interleaved, so a partial sweep stays paired.

The budget is fixed for every instance: 30 generations of 40, 120 s and
4 GB per back-end call, and 3600 s and 8 GB per run. Each run sits in its
own systemd scope when `systemd-run --user` works, and under `ulimit -v`
otherwise.

usage: graded_run.py DIR JOBS --seeds 1-10 --arms binary,graded
                     [--bin PATH] [--filter REGEX]
"""
import argparse
import concurrent.futures as cf
import csv
import re
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUN_TIMEOUT = 3600


def seeds(text):
    lo, _, hi = text.partition('-')
    return list(range(int(lo), int(hi or lo) + 1))


def scoped():
    """The prefix that caps a run's memory."""
    try:
        ok = subprocess.run(['systemd-run', '--user', '--scope', '-q', '-p', 'MemoryMax=1G',
                             'true'], capture_output=True, timeout=30).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        ok = False
    if ok:
        return ['systemd-run', '--user', '--scope', '-q', '-p', 'MemoryMax=8G',
                '-p', 'MemorySwapMax=0']
    return ['prlimit', f'--as={8 << 30}']


def run(job, d, binary, prefix):
    row, seed, arm = job
    base = f"{row['name']}-{arm}-{seed}"
    json_path, err_path = d / 'runs' / f'{base}.json', d / 'runs' / f'{base}.err'
    if json_path.exists():
        return 'skip'
    args = [binary, '--backend', row['backend'], '--model', str(d / row['model']),
            '--seed', str(seed), '--generations', '30', '--population', '40',
            '--status', arm, '--timeout', '120', '--memory-mb', '4096']
    for flag, key in (('--atoms', 'atoms'), ('--ltl', 'phi'), ('--property', 'property')):
        if row[key]:
            args += [flag, row[key]]
    start = time.time()
    try:
        r = subprocess.run(prefix + ['nice', '-n', '10', 'timeout', str(RUN_TIMEOUT)] + args,
                           capture_output=True, text=True, timeout=RUN_TIMEOUT + 60)
        stdout, stderr = r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        stdout, stderr = '', 'killed by the runner\n'
    line = stdout.strip().splitlines()[-1] if stdout.strip() else ''
    if line.startswith('{'):
        json_path.write_text(line + '\n')
        return 'ok'
    err_path.write_text(f'{stderr}\nno result after {time.time() - start:.0f} s\n')
    return 'error'


def main():
    p = argparse.ArgumentParser()
    p.add_argument('dir')
    p.add_argument('jobs', type=int)
    p.add_argument('--seeds', default='1-10')
    p.add_argument('--arms', default='binary,graded')
    p.add_argument('--bin', default=str(HERE.parent / 'target/release/search_bench'))
    p.add_argument('--filter', default='.')
    a = p.parse_args()
    d = Path(a.dir).resolve()
    (d / 'runs').mkdir(exist_ok=True)
    rows = [r for r in csv.DictReader(open(d / 'instances.tsv'), delimiter='\t')
            if re.search(a.filter, r['name'])]
    jobs = [(r, s, arm) for s in seeds(a.seeds) for r in rows for arm in a.arms.split(',')]
    prefix = scoped()
    done = {}
    with cf.ThreadPoolExecutor(a.jobs) as pool:
        for status in pool.map(lambda j: run(j, d, a.bin, prefix), jobs):
            done[status] = done.get(status, 0) + 1
    print(done)


if __name__ == '__main__':
    main()
