#!/usr/bin/env python3
"""Replay every implication pair logged by GAWAIN_SPOT_LOG, one pair per ltlfilt exec.

usage: tier2_replay.py LTLFILT TIMEOUT_S WORKERS OUT_CSV LOG...

The logs come from oracle_bench built with bench/notes/tier2-spot-log.patch, a
diagnostic patch to src/oracle/tiered.rs as of commit 69992f2 (before the
rewriting-rule stage). With GAWAIN_SPOT_LOG=FILE set, each tier 2 or eviction exec
appends a line `BATCH<TAB>lookup|evict<TAB>pairs<TAB>seconds<TAB>store size<TAB>result`
and one line `PAIR<TAB>psi<TAB>phi` per pair. The method and results are in
"Tier 2 eviction" in docs/dev/oracle-measurements.md.

For each distinct (purpose, psi, phi) it times
  synt: ltlfilt -r2 --ap=0 --format=%L,%f   (proved iff the line simplifies to 0)
  full: ltlfilt -v --satisfiable --format=%L (proved iff unsatisfiable), TIMEOUT_S limit
Each exec runs with a 2 GB address-space limit.
"""
import csv, resource, subprocess, sys, time
from multiprocessing import Pool

LTLFILT, TIMEOUT = sys.argv[1], float(sys.argv[2])


def limit():
    resource.setrlimit(resource.RLIMIT_AS, (2 << 30, 2 << 30))


def run(args, line):
    t = time.time()
    try:
        p = subprocess.run([LTLFILT, '-F', '-', *args], input=line, capture_output=True,
                           text=True, timeout=TIMEOUT, preexec_fn=limit)
        out = p.stdout.strip() if p.returncode in (0, 1) else f'ERR{p.returncode}'
    except subprocess.TimeoutExpired:
        out = 'TIMEOUT'
    return out, time.time() - t


def job(item):
    purpose, psi, phi, count = item
    line = f'({psi}) & !({phi})\n'
    s_out, s_t = run(['-r2', '--ap=0', '--format=%L,%f'], line)
    f_out, f_t = run(['-v', '--satisfiable', '--format=%L'], line)
    synt = 'proved' if s_out == '1,0' else ('TIMEOUT' if s_out == 'TIMEOUT' else 'not')
    full = 'proved' if f_out == '1' else (f_out if f_out in ('TIMEOUT',) or f_out.startswith('ERR') else 'not')
    return [purpose, count, len(psi), len(phi), synt, f'{s_t:.4f}', full, f'{f_t:.4f}', psi, phi]


def main():
    workers, out_path, logs = int(sys.argv[3]), sys.argv[4], sys.argv[5:]
    counts = {}
    for log in logs:
        purpose = None
        for raw in open(log):
            parts = raw.rstrip('\n').split('\t')
            if parts[0] == 'BATCH':
                purpose = parts[1]
            elif parts[0] == 'PAIR':
                key = (purpose, parts[1], parts[2])
                counts[key] = counts.get(key, 0) + 1
    items = [(*k, c) for k, c in counts.items()]
    print(f'{len(items)} distinct pairs', flush=True)
    with open(out_path, 'w', newline='') as fh, Pool(workers) as pool:
        w = csv.writer(fh)
        w.writerow(['purpose', 'count', 'psi_len', 'phi_len', 'synt', 'synt_s', 'full', 'full_s', 'psi', 'phi'])
        for i, row in enumerate(pool.imap_unordered(job, items)):
            w.writerow(row)
            fh.flush()


if __name__ == '__main__':
    main()
