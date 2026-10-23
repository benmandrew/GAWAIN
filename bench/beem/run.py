#!/usr/bin/env python3
"""Run the SPIN jobs written by prepare.py; append one JSON line per check.

Each variant is compiled twice: `pan` checks the ltl claims with `-a`, and
`pan_safety` (-DNOCLAIM -DSAFETY) explores the state space once to count
states and deadlocks. Every check runs under systemd-run with a memory cap
and a timeout. A rerun skips checks already in OUT.

usage: [CLAIMS=regex] run.py JOBS_DIR OUT.jsonl [TIMEOUT_S] [JOBS] [MEM_GB] [KEY_REGEX]
"""
import json
import os
import re
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

SPIN = os.environ.get('SPIN', 'spin')
jobs_dir = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2])
TO = int(sys.argv[3]) if len(sys.argv) > 3 else 1800
JOBS = int(sys.argv[4]) if len(sys.argv) > 4 else 8
MEM = int(sys.argv[5]) if len(sys.argv) > 5 else 10
FILTER = re.compile(sys.argv[6] if len(sys.argv) > 6 else '')
# Which claims to run; the fin* claims only matter where a variant deadlocks.
CLAIMS = re.compile(os.environ.get('CLAIMS', '^(?!fin)'))
# pan allocates its depth stack up front: 5M steps cost about 270 MB.
DEPTH = '-m' + os.environ.get('DEPTH', '5000000')
HASH = '-w26'
lock = threading.Lock()


def limited(cmd, timeout, cwd):
    """Runs cmd under the memory cap and the timeout; (output, seconds)."""
    wrap = ['systemd-run', '--user', '--scope', '-q', '-p', f'MemoryMax={MEM}G',
            '-p', 'MemorySwapMax=0', 'nice', '-n', '10', 'timeout', str(timeout)]
    t = time.time()
    r = subprocess.run(wrap + cmd, cwd=cwd, capture_output=True, text=True,
                       stdin=subprocess.DEVNULL)
    return r.returncode, r.stdout + r.stderr, time.time() - t


def build(d, safety=True):
    memlim = f'-DMEMLIM={MEM * 1024 - 512}'
    # -o2 keeps write-only variables in the state vector, so state counts
    # match DiVinE's; dropping them cannot change a verdict.
    code, o, t = limited([SPIN, '-a', '-o2', 'model.pml'], 600, d)
    if code:
        return f'spin -a failed: {o[-300:]}', t
    exes = [('pan', []), ('pan_safety', ['-DNOCLAIM', '-DSAFETY'])][:2 if safety else 1]
    for exe, extra in exes:
        code, o2, t2 = limited(['gcc', '-O2', '-DNOREDUCE', '-DCOLLAPSE', '-DVECTORSZ=4096',
                                memlim, *extra, '-o', exe, 'pan.c'], 1200, d)
        t += t2
        if code:
            return f'gcc failed: {o2[-300:]}', t
    return None, t


def verdict(code, o):
    """Reads pan's output: TRUE, FALSE, or UNKNOWN with a reason."""
    if code == 124:
        return 'UNKNOWN', 'timeout'
    if 'indexing' in o or 'value out of range' in o:
        return 'UNKNOWN', 'model error: ' + ' '.join(l for l in o.splitlines() if 'pan:' in l)[:200]
    if 'out of memory' in o or 'MEMLIM' in o or code == 137 or code == -9:
        return 'UNKNOWN', 'memory'
    if 'max search depth too small' in o:
        return 'UNKNOWN', 'depth'
    m = re.search(r'errors: (\d+)', o)
    if not m:
        return 'UNKNOWN', f'no result (exit {code}): {o[-200:]}'
    if m.group(1) == '0':
        if 'Search not completed' in o:
            return 'UNKNOWN', 'incomplete'
        return 'TRUE', ''
    if re.search(r'acceptance cycle|assertion violated|claim violated', o):
        return 'FALSE', ''
    return 'UNKNOWN', 'unrecognised error: ' + o[-200:]


def record(row):
    with lock, open(out, 'a') as fh:
        fh.write(json.dumps(row) + '\n')


builds = {}


def built(key):
    """Builds `key` (a variant, or variant/fin claim) once, on first use;
    the build's error or None."""
    with lock:
        entry = builds.setdefault(key, [threading.Lock(), False, None])
    with entry[0]:
        if not entry[1]:
            entry[2], t = build(jobs_dir / key, safety='/' not in key)
            entry[1] = True
            record({'key': key, 'check': 'build', 'seconds': round(t, 2), 'error': entry[2]})
    return entry[2]


def check(key, c):
    d = jobs_dir / key
    if c.startswith('fin'):  # its own build, with a never claim
        d = d / c
        if built(f'{key}/{c}'):
            record({'key': key, 'check': c, 'verdict': 'UNKNOWN', 'reason': 'build', 'seconds': 0})
        else:
            code, o, t = limited(['./pan', '-a', DEPTH, HASH, '-n'], TO, d)
            v, why = verdict(code, o)
            record({'key': key, 'check': c, 'verdict': v, 'reason': why, 'seconds': round(t, 2)})
    elif built(key):
        record({'key': key, 'check': c, 'verdict': 'UNKNOWN', 'reason': 'build', 'seconds': 0})
    elif c == 'states':
        code, o, t = limited(['./pan_safety', DEPTH, HASH, '-c0', '-n'], TO, d)
        m = re.search(r'(\d+) states, stored', o)
        e = re.search(r'errors: (\d+)', o)
        v, why = verdict(code, o)
        if m and e and code == 0 and (v != 'UNKNOWN' or why.startswith('unrecognised')):
            # -c0 goes on past every deadlock, so errors > 0 is no verdict here
            v, why = 'DONE', ''
        if re.search(r'MEMLIM|out of memory|depth too small|Search not completed', o):
            v, why = 'UNKNOWN', 'incomplete'
        record({'key': key, 'check': c, 'verdict': v, 'reason': why,
                'states': int(m.group(1)) if m else None,
                'deadlocks': int(e.group(1)) if e else None, 'seconds': round(t, 2)})
    else:
        code, o, t = limited(['./pan', '-a', DEPTH, HASH, '-n', '-N', c], TO, d)
        v, why = verdict(code, o)
        record({'key': key, 'check': c, 'verdict': v, 'reason': why, 'seconds': round(t, 2)})


def main():
    jobs = json.load(open(jobs_dir / 'jobs.json'))['variants']
    done = set()
    if out.exists():
        for line in open(out):
            r = json.loads(line)
            if r['check'] != 'build' and r.get('verdict') != 'UNKNOWN':
                done.add((r['key'], r['check']))
    keys = [k for k in jobs if FILTER.search(k)]
    tasks = [(k, c) for k in keys for c in ['states'] + jobs[k]['claims'] if CLAIMS.search(c)
             if (k, c) not in done]
    with ThreadPoolExecutor(JOBS) as ex:
        for f in [ex.submit(check, k, c) for k, c in tasks]:
            f.result()


if __name__ == '__main__':
    main()
