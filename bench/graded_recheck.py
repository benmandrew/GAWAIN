#!/usr/bin/env python3
"""Re-check the implication pairs that bench/graded_stats.py left undecided.

graded_stats.py gives Spot 30 s per batch and 2 s per retried pair, and
caches a pair it could not decide as None, read as not implied. This script
asks Spot about those pairs again, one pair per exec, with a larger limit.

    graded_recheck.py pairs DIR CACHE ARM [--baseline binary] > PAIRS.tsv
        Lists the pairs that a comparison of ARM with the baseline in DIR
        needs (as graded_stats.strength does) and CACHE holds as undecided.
        Columns: id, instance (the first that needs the pair), finite (0 or
        1), x, y.

    graded_recheck.py check PAIRS.tsv OUT.csv LIMIT [--jobs J] [--mem G]
            [--max-load L]
        Asks `ltlfilt` whether x implies y for each pair, J at a time (default
        16), each exec in its own systemd scope with MemoryMax=G GB (default
        6) and `timeout LIMIT`. Appends one row per pair to OUT.csv (id,
        limit, outcome, implied, seconds, detail), and skips ids it already
        holds, so a stopped run resumes. Outcome is `decided`, `timeout`,
        `memory` or `error`. Before each exec it waits while the 1-minute
        load average, less its own running execs, exceeds L (default 16),
        since the remote hosts are shared.

    graded_recheck.py merge CACHE PAIRS.tsv OUT.csv... NEWCACHE
        Writes CACHE with every pair decided in some OUT.csv set to its
        verdict, for graded_stats.py to read as OUTDIR/implications.json.

Spot 2.15.1 must be on PATH, as for graded_stats.py.
"""
import concurrent.futures as cf
import csv
import json
import os
import subprocess
import sys
import threading
import time
from collections import defaultdict
from pathlib import Path

# Some LTLf repairs are longer than csv's default field limit.
csv.field_size_limit(1 << 30)


def arg(argv, flag, default):
    return type(default)(argv[argv.index(flag) + 1]) if flag in argv else default


def pairs(argv):
    d, cache, arm = Path(argv[0]), json.loads(Path(argv[1]).read_text()), argv[2]
    baseline = arg(argv, '--baseline', 'binary')
    undecided = {(f, x, y) for f, x, y, v in cache if v is None}
    instances = list(csv.DictReader(open(d / 'instances.tsv'), delimiter='\t'))
    finite = {r['name'] for r in instances if r.get('semantics') == 'finite'}
    by = defaultdict(dict)
    for path in sorted((d / 'runs').glob('*.json')):
        name, a, seed = path.stem.rsplit('-', 2)
        if a in (arm, baseline):
            by[(name, a)][int(seed)] = [r['formula']
                                        for r in json.loads(path.read_text())['repairs']]
    todo = {}
    for name in sorted({n for n, a in by if a == arm}):
        seeds = set(by[(name, arm)]) & set(by.get((name, baseline), {}))
        fs = sorted({f for s in seeds for a in (arm, baseline) for f in by[(name, a)][s]})
        fin = name in finite
        for x in fs:
            for y in fs:
                if x != y and (fin, x, y) in undecided:
                    todo.setdefault((fin, x, y), name)
    w = csv.writer(sys.stdout, delimiter='\t', lineterminator='\n')
    w.writerow(['id', 'instance', 'finite', 'x', 'y'])
    for i, (fin, x, y) in enumerate(sorted(todo)):
        w.writerow([i, todo[(fin, x, y)], int(fin), x, y])


def check_one(fin, x, y, limit, mem):
    """One ltlfilt exec on x & !y; (outcome, implied, seconds, detail)."""
    ltlf = ['--from-ltlf=gawain_alive'] if fin else []
    cmd = ['systemd-run', '--user', '--scope', '-q', '-p', f'MemoryMax={mem}G',
           '-p', 'MemorySwapMax=0', 'nice', '-n', '10', 'timeout', str(limit),
           'ltlfilt', *ltlf, '-F', '-', '-v', '--satisfiable', '--format=%L']
    start = time.monotonic()
    r = subprocess.run(cmd, input=f'({x}) & !({y})\n', capture_output=True, text=True)
    secs = time.monotonic() - start
    if r.returncode == 124:
        return 'timeout', '', secs, ''
    if r.returncode in (137, -9):
        return 'memory', '', secs, ''
    # ltlfilt exits 1 when it prints nothing, that is when the pair is not implied.
    if r.returncode not in (0, 1) or r.stderr:
        return 'error', '', secs, f'{r.returncode}: {r.stderr.strip()[:200]}'
    return 'decided', int(r.stdout.split() == ['1']), secs, ''


def check(argv):
    src, out, limit = Path(argv[0]), Path(argv[1]), int(argv[2])
    jobs, mem, max_load = arg(argv, '--jobs', 16), arg(argv, '--mem', 6), arg(argv, '--max-load', 16.0)
    done = set()
    if out.exists():
        done = {r['id'] for r in csv.DictReader(open(out))}
    todo = [r for r in csv.DictReader(open(src), delimiter='\t') if r['id'] not in done]
    lock, running = threading.Lock(), [0]
    f = open(out, 'a', newline='')
    w = csv.writer(f)
    if not done:
        w.writerow(['id', 'limit', 'outcome', 'implied', 'seconds', 'detail'])

    def run(r):
        while True:
            with lock:
                others = os.getloadavg()[0] - running[0]
                if others <= max_load:
                    running[0] += 1
                    break
            time.sleep(60)
        try:
            got = check_one(r['finite'] == '1', r['x'], r['y'], limit, mem)
        finally:
            with lock:
                running[0] -= 1
        with lock:
            w.writerow([r['id'], limit, got[0], got[1], f'{got[2]:.2f}', got[3]])
            f.flush()

    print(f'{len(todo)} pairs at {limit} s, {jobs} at a time', file=sys.stderr)
    with cf.ThreadPoolExecutor(jobs) as pool:
        for i, _ in enumerate(pool.map(run, todo), 1):
            if i % 500 == 0:
                print(f'{i}/{len(todo)} at {time.strftime("%H:%M:%S")}, '
                      f'load {os.getloadavg()[0]:.1f}', file=sys.stderr)
    f.close()


def merge(argv):
    cache, src, outs, new = Path(argv[0]), Path(argv[1]), argv[2:-1], Path(argv[-1])
    key = {r['id']: (r['finite'] == '1', r['x'], r['y'])
           for r in csv.DictReader(open(src), delimiter='\t')}
    verdict = {}
    for o in outs:
        for r in csv.DictReader(open(o)):
            if r['outcome'] == 'decided':
                verdict[key[r['id']]] = r['implied'] == '1'
    items = json.loads(cache.read_text())
    for item in items:
        if item[3] is None and tuple(item[:3]) in verdict:
            item[3] = verdict[tuple(item[:3])]
    new.parent.mkdir(parents=True, exist_ok=True)
    new.write_text(json.dumps(items))
    print(f'{len(verdict)} pairs decided', file=sys.stderr)


if __name__ == '__main__':
    {'pairs': pairs, 'check': check, 'merge': merge}[sys.argv[1]](sys.argv[2:])
