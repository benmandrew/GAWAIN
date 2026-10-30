#!/usr/bin/env python3
"""Run the binary-against-graded ablation.

Every instance of DIR/instances.tsv (from bench/graded_prep.py) runs under
every arm and seed, JOBS at a time, one search_bench process per run. A run
writes DIR/runs/<name>-<arm>-<seed>.json, or <...>.err with its stderr when
it prints no JSON line (a timeout or a crash). A run whose JSON exists is
skipped, so the script resumes. Runs are ordered seed-major with the arms
interleaved, so a partial sweep stays paired.

The budget is fixed for every instance: by default 30 generations of 40
(`--budget GxP` sets another), 120 s and 4 GB per back-end call, and
3600 s and 8 GB per run (`--run-timeout` and `--memory-gb` set others,
for a larger budget or a rerun of the runs a cap killed). `--out` names the directory under DIR that
takes the results (default `runs`). Each run sits in its
own systemd scope when `systemd-run --user` works, and under `ulimit -v`
otherwise. The script stops before the first run unless the `ltlfilt` on
PATH is Spot 2.15.1: an older Spot lacks `--satisfiable`, so the output
gate fails on every run and the runs report no repairs.

The arms are `binary`, `graded` (option B), `feasible` (B with every
proved candidate ranked first), `reserve4` and `reserve2` (feasible with a
quarter or a half of the population kept for unproved candidates) and
`forced` (binary, searching
degenerate instances too, for the CEGIW case studies), and `initflag`,
`initfilter` and their `forced-` twins for the initial-state check
(bench/initial_stats.py), `default` and `forced-default` for the
evaluation (docs/dev/evaluation.md), and `mutation` and `forced-mutation`
for its random-mutation baseline; see ARMS. The `random` and
`states` arms of the ablation ran from commit `ddd9648` (tag `archive/graded-arms`),
whose code was removed; see "Graded status" in `docs/dev/search.md`.

usage: graded_run.py DIR JOBS --seeds 1-10 --arms binary,graded
                     [--budget 30x40] [--out runs] [--memory-gb 8] [--run-timeout 3600]
                     [--bin PATH]
                     [--filter REGEX]
"""
import argparse
import concurrent.futures as cf
import csv
import re
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPOT = '2.15.1'
# The search_bench flags of each arm. The arms measured before the
# initial-state filter became the default keep `--initial ignore`, so they
# reproduce their recorded results.
ARMS = {
    'binary': ['--status', 'binary', '--initial', 'ignore'],
    'graded': ['--status', 'graded', '--initial', 'ignore'],
    # The binary arm searching degenerate instances too, for the CEGIW
    # case studies (bench/baselines_cegiw.py).
    'forced': ['--status', 'binary', '--degenerate', 'search', '--initial', 'ignore'],
    'feasible': ['--status', 'graded-feasible', '--initial', 'ignore'],
    'reserve4': ['--status', 'graded-reserve-4', '--initial', 'ignore'],
    'reserve2': ['--status', 'graded-reserve-2', '--initial', 'ignore'],
    # Binary with the initial-state predicate: `initflag` only marks the
    # repairs that I ∧ Init implies, `initfilter` drops them in the loop
    # and the gate. The `forced-` arms search degenerate instances too.
    'initflag': ['--status', 'binary', '--initial', 'flag'],
    'initfilter': ['--status', 'binary', '--initial', 'filter'],
    'forced-initflag': ['--status', 'binary', '--degenerate', 'search', '--initial', 'flag'],
    'forced-initfilter': ['--status', 'binary', '--degenerate', 'search', '--initial', 'filter'],
    # The evaluation's configuration: search_bench's defaults, so binary
    # status with the initial-state filter (docs/dev/evaluation.md).
    'default': ['--status', 'binary'],
    'forced-default': ['--status', 'binary', '--degenerate', 'search'],
    # The random-mutation baseline: binary status and the default
    # initial-state filter, with no selection during the run
    # ("Random-mutation baseline" in docs/dev/search.md). `forced-mutation`
    # searches degenerate instances too, for the CEGIW case studies.
    'mutation': ['--status', 'binary', '--search', 'random'],
    'forced-mutation': ['--status', 'binary', '--degenerate', 'search', '--search', 'random'],
}


def seeds(text):
    lo, _, hi = text.partition('-')
    return list(range(int(lo), int(hi or lo) + 1))


def scoped(gb):
    """The prefix that caps a run's memory."""
    try:
        ok = subprocess.run(['systemd-run', '--user', '--scope', '-q', '-p', 'MemoryMax=1G',
                             'true'], capture_output=True, timeout=30).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        ok = False
    if ok:
        return ['systemd-run', '--user', '--scope', '-q', '-p', f'MemoryMax={gb}G',
                '-p', 'MemorySwapMax=0']
    return ['prlimit', f'--as={gb << 30}']


def run(job, d, out, budget, binary, prefix, limit):
    row, seed, arm = job
    base = f"{row['name']}-{arm}-{seed}"
    json_path, err_path = out / f'{base}.json', out / f'{base}.err'
    if json_path.exists():
        return 'skip'
    args = [binary, '--backend', row['backend'], '--model', str(d / row['model']),
            '--seed', str(seed), '--generations', budget[0], '--population', budget[1],
            '--timeout', '120', '--memory-mb', '4096'] + ARMS[arm]
    for flag, key in (('--atoms', 'atoms'), ('--ltl', 'phi'), ('--property', 'property')):
        if row[key]:
            args += [flag, row[key]]
    start = time.time()
    try:
        r = subprocess.run(prefix + ['nice', '-n', '10', 'timeout', str(limit)] + args,
                           capture_output=True, text=True, timeout=limit + 60)
        stdout, stderr = r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        stdout, stderr = '', 'killed by the runner\n'
    line = stdout.strip().splitlines()[-1] if stdout.strip() else ''
    if line.startswith('{'):
        json_path.write_text(line + '\n')
        err_path.unlink(missing_ok=True)
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
    p.add_argument('--budget', default='30x40')
    p.add_argument('--out', default='runs')
    p.add_argument('--memory-gb', type=int, default=8)
    p.add_argument('--run-timeout', type=int, default=3600)
    a = p.parse_args()
    version = subprocess.run(['ltlfilt', '--version'], capture_output=True, text=True).stdout
    if f'(spot) {SPOT}' not in version.splitlines()[0]:
        raise SystemExit(f'ltlfilt on PATH is {version.splitlines()[0]!r}, not Spot {SPOT}')
    d = Path(a.dir).resolve()
    out = d / a.out
    out.mkdir(exist_ok=True)
    budget = a.budget.split('x')
    rows = [r for r in csv.DictReader(open(d / 'instances.tsv'), delimiter='\t')
            if re.search(a.filter, r['name'])]
    jobs = [(r, s, arm) for s in seeds(a.seeds) for r in rows for arm in a.arms.split(',')]
    prefix = scoped(a.memory_gb)
    done = {}
    with cf.ThreadPoolExecutor(a.jobs) as pool:
        for status in pool.map(lambda j: run(j, d, out, budget, a.bin, prefix, a.run_timeout), jobs):
            done[status] = done.get(status, 0) + 1
    print(done)


if __name__ == '__main__':
    main()
