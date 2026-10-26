#!/usr/bin/env python3
"""Compare CEGIW's interval weakenings with GAWAIN's repairs.

Reads DIR from `bench/cegiw_run.py` (instances.tsv, cegiw.csv, models/)
and GAWAIN's runs of the same instances in DIR/runs (`bench/graded_run.py
DIR 1 --arms binary`). A degenerate case (M ⊨ ¬φ), which the binary arm
does not search, takes its runs from the `forced` arm, which does. For
each case it

- checks CEGIW's weakened formula on the model with a full nuXmv LTL check
  (`check_ltlspec`, BDD-based), since CEGIW stops when bounded model
  checking finds no counterexample up to its bound;
- asks Spot whether it is valid, and valid modulo the encoding invariant I
  of GAWAIN's gate (`G !(a & b)` over the atoms of one enumerated variable,
  from the model's DEFINEs);
- scores it and ⊤ against φ with the `score` binary, as GAWAIN scores
  repairs;
- pools GAWAIN's repairs over the seeds, reduces them to the maximal set
  (plain implication, as `bench/graded_stats.py` does), and places CEGIW's
  formula against each member: stronger, weaker, equivalent or
  incomparable, both plainly and modulo I.

Writes OUT (one row per case) and prints a table. Spot 2.15.1 and nuXmv
must be on PATH.

usage: baselines_cegiw.py DIR OUT [--score PATH]
"""
import argparse
import csv
import json
import re
import statistics
import subprocess
import tempfile
from collections import defaultdict
from pathlib import Path

from graded_stats import implications, maximal


def invariant(model_text, atoms):
    """GAWAIN's encoding invariant over atoms, or 'true' with no group."""
    groups = defaultdict(list)
    seen = set()
    for name, var, const in re.findall(r'(\w+)\s*:=\s*([\w.]+)\s*=\s*(\w+)\s*;', model_text):
        if name in atoms and (var, const) not in seen:
            seen.add((var, const))
            groups[var].append(name)
    pairs = [f'!({x} & {y})' for g in groups.values() if len(g) > 1
             for i, x in enumerate(g) for y in g[i + 1:]]
    return f"G({' & '.join(pairs)})" if pairs else 'true'


def nuxmv_holds(model, spin_ltl):
    """Whether the model satisfies the LTL formula, by a full nuXmv check."""
    text = spin_ltl.replace('[]', 'G').replace('<>', 'F').replace('&&', '&').replace('||', '|')
    with tempfile.TemporaryDirectory() as tmp:
        m = Path(tmp) / 'model.smv'
        m.write_text(Path(model).read_text() + f'\nLTLSPEC {text};\n')
        cmds = Path(tmp) / 'cmds'
        cmds.write_text('set on_failure_script_quits 1\ngo\ncheck_ltlspec\nquit\n')
        r = subprocess.run(['nuXmv', '-source', str(cmds), str(m)], capture_output=True,
                           text=True, timeout=600)
    verdict = re.search(r'-- specification .* is (true|false)', r.stdout)
    if not verdict:
        raise RuntimeError(f'nuXmv gave no verdict: {r.stdout[-2000:]}{r.stderr}')
    return verdict.group(1) == 'true'


def relation(c, m, imp):
    cm, mc = imp[(c, m)], imp[(m, c)]
    return {(True, True): 'equivalent', (True, False): 'cegiw stronger',
            (False, True): 'gawain stronger', (False, False): 'incomparable'}[(cm, mc)]


def main():
    p = argparse.ArgumentParser()
    p.add_argument('dir', type=Path)
    p.add_argument('out')
    p.add_argument('--score', default='target/release/score')
    a = p.parse_args()

    instances = {r['name']: r for r in csv.DictReader(open(a.dir / 'instances.tsv'),
                                                       delimiter='\t')}
    cegiw = defaultdict(list)
    for r in csv.DictReader(open(a.dir / 'cegiw.csv')):
        cegiw[r['name']].append(r)
    rows = []
    for name, inst in instances.items():
        model = a.dir / inst['model']
        inv = invariant(model.read_text(), inst['atoms'].split(','))
        phi = inst['phi']
        def runs_of(arm):
            return [json.loads(path.read_text().strip().splitlines()[-1])
                    for path in sorted((a.dir / 'runs').glob(f'{name}-{arm}-*.json'))]
        runs = runs_of('binary')
        klass = runs[0]['instance'] if runs else ''
        arm = 'binary'
        if klass == 'degenerate':
            runs, arm = runs_of('forced'), 'forced'
        reps = cegiw[name]
        outcomes = {r['outcome'] for r in reps}
        if len(outcomes) != 1 or len({r['ltl'] for r in reps}) != 1:
            raise SystemExit(f'{name}: CEGIW repetitions disagree: {reps}')
        c = reps[0]['ltl'] or None
        pooled = sorted({rep['formula'] for run in runs for rep in run['repairs']})
        formulas = pooled + ([c] if c else [])
        pairs = [(x, y) for x in formulas for y in formulas if x != y]
        imp = dict(zip(pairs, implications(pairs)))
        imp_i = dict(zip(pairs, implications([(f'({inv}) & ({x})', y) for x, y in pairs])))
        best = maximal(pooled, imp)
        found = [run for run in runs if run['repairs']]
        row = {
            'name': name, 'phi': phi, 'invariant': inv, 'class': klass, 'gawain_arm': arm,
            'cegiw_outcome': reps[0]['outcome'], 'cegiw_interval': reps[0]['interval'],
            'cegiw_final': reps[0]['final'], 'cegiw_mtl': reps[0]['mtl'], 'cegiw_ltl': c or '',
            'cegiw_wall': f"{statistics.median(float(r['wall']) for r in reps):.3f}",
            'cegiw_reps': len(reps),
            'gawain_seeds': len(runs), 'gawain_found': len(found),
            'gawain_wall': f"{statistics.median(r['wall'] for r in runs):.3f}" if runs else '',
            'gawain_backend_calls': statistics.median(r['backend_calls'] for r in runs) if runs else '',
            'gawain_best_semantic': (f"{statistics.median(max(x['semantic'] for x in r['repairs']) for r in found):.6f}"
                                     if found else ''),
            'top_semantic': subprocess.run(
                [a.score], input=f'{phi}\ttrue\n', capture_output=True, text=True,
                check=True).stdout.split()[0],
            'gawain_maximal': len(best),
            'gawain_maximal_set': ' ;; '.join(best),
        }
        if c:
            holds = nuxmv_holds(model, c)
            valid, valid_i = implications([('true', c), (inv, c)])
            sem, syn, _ = subprocess.run(
                [a.score], input=f'{phi}\t{c}\n', capture_output=True, text=True,
                check=True).stdout.split()
            rel = [relation(c, m, imp) for m in best]
            rel_i = [relation(c, m, imp_i) for m in best]
            row.update({'cegiw_holds': holds, 'cegiw_valid': valid, 'cegiw_valid_mod_i': valid_i,
                        'cegiw_semantic': sem, 'cegiw_syntactic': syn,
                        'relation': '; '.join(rel), 'relation_mod_i': '; '.join(rel_i)})
        rows.append(row)
        print(f"{name}: CEGIW {row['cegiw_outcome']} {row['cegiw_final']} "
              f"({row['cegiw_wall']} s, holds {row.get('cegiw_holds')}, valid "
              f"{row.get('cegiw_valid')}/{row.get('cegiw_valid_mod_i')}, semantic "
              f"{row.get('cegiw_semantic')}); GAWAIN {len(found)}/{len(runs)} found, "
              f"{row['gawain_wall']} s, best {row['gawain_best_semantic']}, maximal {len(best)}; "
              f"{row.get('relation_mod_i')}", flush=True)
    fields = list(dict.fromkeys(k for r in rows for k in r))
    with open(a.out, 'w', newline='') as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)


if __name__ == '__main__':
    main()
