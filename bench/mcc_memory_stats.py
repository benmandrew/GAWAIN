#!/usr/bin/env python3
"""Summarise bench/mcc_memory.sh runs, for "Bounded monoid" in
docs/dev/search.md.

Each DIR is one arm (its name is the arm's label) holding one directory per
run. The CSV row of a run gives its exit status, the scope's peak memory,
the peak resident set (VmHWM) of the search process and of its largest
child, the largest transition monoid and prefix subset map of any count,
the counts that never reached loop length 20 (stopped by the bound, or by
the end of the run), and the JSON line's wall time, count time, count
failures, repairs and best repair similarity. The monoid columns come only
from a build with bench/notes/count-monoid-log.patch, which prints each
count's size to stderr; they are empty otherwise.

The script also prints, per tool, the largest VmHWM over all runs, and per
arm the counts whose estimated monoid (by the rule of src/count/mod.rs)
passes each of several budgets.

usage: bench/mcc_memory_stats.py OUT.csv DIR...
"""
import csv
import json
import re
import sys
from pathlib import Path

SEARCH = ('search_bench', 'after-dbg', 'before-dbg')
ENTRY_BYTES = 160


def tool(argv):
    name = Path(argv[0]).name
    if name in SEARCH:
        return 'search_bench'
    if name == 'verifypn':
        return 'verifypn -p' if '-p' in argv[1:4] else 'verifypn'
    return name


def counts(err):
    """(n, letters, relations, estimated bytes, seconds, finished) per
    count. A count that never logs loop length 20 stopped at the bound."""
    out, cur = [], None
    for line in err.splitlines():
        m = re.match(r'COUNT start n=(\d+) letters=(\d+)', line)
        if m:
            cur = [int(m[1]), int(m[2]), 0, 0, 0.0, False]
            out.append(cur)
            continue
        m = re.match(r'COUNT len=(\d+) relations=(\d+) rows=(\d+) t=([\d.]+)', line)
        if m and cur:
            n, letters = cur[0], cur[1]
            relations, rows = int(m[2]), int(m[3])
            words = -(-n // 64)
            cur[2] = relations
            # Rows are estimated at 3 planes; they are a small share.
            cur[3] = (relations * (4 * n + 8 * letters + ENTRY_BYTES)
                      + rows * (8 * 3 * words + 4 * letters + ENTRY_BYTES))
            cur[4] = float(m[4])
            cur[5] = m[1] == '20'
    return out


def main():
    out_csv, dirs = sys.argv[1], [Path(d) for d in sys.argv[2:]]
    rows, peaks, budgets = [], {}, {}
    for d in dirs:
        arm = d.name
        all_counts = []
        for run in sorted(p for p in d.iterdir() if p.is_dir()):
            read = lambda f: (run / f).read_text(errors='replace') if (run / f).exists() else ''
            status = read('exit').split()
            cg_peak = max((int(l.split()[2]) for l in read('cg.log').splitlines()
                           if len(l.split()) == 3), default=0)
            own = child = 0
            for line in read('procs.log').splitlines():
                f = line.split()
                if len(f) < 5 or Path(f[4]).name == 'timeout':
                    continue
                hwm, name = int(f[2]) << 10, tool(f[4:])
                if name == 'search_bench':
                    own = max(own, hwm)
                else:
                    child = max(child, hwm)
                if hwm > peaks.get(name, (0, ''))[0]:
                    peaks[name] = (hwm, f'{arm}/{run.name}')
            err = read('stderr')
            mine = counts(err)
            all_counts += [(run.name,) + tuple(c) for c in mine]
            subsets = max(map(int, re.findall(r'subsets=(\d+)', err)), default='')
            j = {}
            line = read('stdout').strip().splitlines()
            if line and line[-1].startswith('{'):
                j = json.loads(line[-1])
            reps = j.get('repairs')
            rows.append({
                'arm': arm,
                'run': run.name,
                'exit': status[1] if len(status) > 1 else '',
                'scope_peak_mb': cg_peak >> 20,
                'search_hwm_mb': own >> 20,
                'child_hwm_mb': child >> 20,
                'monoid_relations': max((c[2] for c in mine), default=''),
                'monoid_mib': max((c[3] for c in mine), default=0) >> 20 if mine else '',
                'subsets': subsets,
                'monoid_cuts': sum(not c[5] for c in mine) if mine else '',
                'wall': j.get('wall', ''),
                'count_time': j.get('count_time', ''),
                'count_wait': j.get('count_wait', ''),
                'count_failures': j.get('count_failures', ''),
                'repairs': len(reps) if reps is not None else '',
                'best_semantic': max((r['semantic'] for r in reps), default='') if reps else '',
            })
        budgets[arm] = all_counts
    with open(out_csv, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    print('Peak VmHWM per tool:')
    for name, (hwm, where) in sorted(peaks.items()):
        print(f'  {name}: {hwm >> 20} MB ({where})')
    for arm, cs in budgets.items():
        if not cs:
            continue
        print(f'{arm}: {len(cs)} counts in {len({c[0] for c in cs})} runs, '
              f'{sum(c[5] for c in cs):.0f} s in the lasso count')
        for mib in (256, 512, 1024, 2048):
            over = [c for c in cs if c[4] > mib << 20]
            print(f'  over {mib} MiB: {len(over)} counts in {len({c[0] for c in over})} runs')


if __name__ == '__main__':
    main()
