#!/usr/bin/env python3
"""Check phi and !phi on every LMCS-2006 pair with nuXmv; write the filter CSV.
usage: lmcs_check.py DATA_DIR OUT_CSV [TIMEOUT_S] [JOBS]"""
import csv, re, subprocess, sys, tempfile, os, time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

NUXMV = os.environ.get('NUXMV', '/usr/local/bin/nuXmv')
data = Path(sys.argv[1]).resolve(); out = sys.argv[2]
TO = int(sys.argv[3]) if len(sys.argv) > 3 else 300
JOBS = int(sys.argv[4]) if len(sys.argv) > 4 else 6
DIRS = {'bc57-sensors': 'reactor', 'dme': 'dme'}

pairs = []
for line in (data / 'README').read_text().splitlines():
    f = [x.strip() for x in line.split(',')]
    if len(f) >= 4 and f[3] in ('TRUE', 'FALSE', 'UNKNOWN') and f[1].isdigit():
        pairs.append((f[0], f[2], f[3]))

def locate(model, prop):
    m = re.fullmatch(r'dme\d', model)
    d = 'dme' if m else DIRS.get(model, model)
    base = data / 'fixed' / d
    return base / f'{model}.smv', base / f'{prop}.ltl'

def check(model, smv, formula):
    with tempfile.TemporaryDirectory() as td:
        cmd = Path(td) / 'c.cmd'
        cmd.write_text(f'read_model -i {smv}\nflatten_hierarchy\nencode_variables\nbuild_model\ncheck_ltlspec -p "{formula}"\nquit\n')
        t = time.time()
        try:
            r = subprocess.run(['systemd-run','--user','--scope','-q','-p','MemoryMax=4G','-p','MemorySwapMax=0','nice','-n','10','timeout',str(TO),NUXMV, '-source', str(cmd)], cwd=smv.parent, capture_output=True,
                               text=True, timeout=TO + 20, stdin=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            return 'TIMEOUT', time.time() - t
        o = r.stdout + r.stderr
        m = re.search(r'is (true|false)', o)
        return (m.group(1).upper() if m else 'ERROR:' + ' '.join(o.split('\n')[-6:])[:150]), time.time() - t

def job(p):
    model, prop, readme = p
    smv, ltl = locate(model, prop)
    phi = ' '.join(ltl.read_text().split())
    v, t1 = check(model, smv, phi)
    n, t2 = check(model, smv, f'!({phi})')
    return model, prop, readme, v, n, round(t1 + t2, 1)

pairs = [p for p in pairs if not re.search(os.environ.get('SKIP', '^$'), p[0])]
t0 = time.time()
with ThreadPoolExecutor(JOBS) as ex:
    rows = list(ex.map(job, pairs))
with open(out, 'w', newline='') as fh:
    w = csv.writer(fh)
    w.writerow(['model', 'property', 'readme', 'phi', 'not_phi', 'verdict_matches', 'degenerate_filter', 'seconds'])
    for model, prop, readme, v, n, t in rows:
        match = 'n/a' if readme == 'UNKNOWN' else str(v == readme).lower()
        if v == 'FALSE' and n == 'FALSE': flt = 'survives'
        elif v == 'FALSE' and n == 'TRUE': flt = 'dropped'
        else: flt = 'n/a'
        w.writerow([model, prop, readme, v, n, match, flt, t])
print('total seconds', round(time.time() - t0, 1))
