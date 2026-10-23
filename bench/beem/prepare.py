#!/usr/bin/env python3
"""Write the SPIN jobs for the BEEM ERROR pairs and their correct variants.

For every instance with ERROR=1 or ERROR=2 and every LTL property of its
model, the erroneous variant is the instance itself and the correct
variant is the same parameters with ERROR=0. Each variant becomes one
directory under OUT with `model.pml`, which holds the flat Promela of
dve.py and one `ltl` claim per check: `phi_<id>` and `neg_<id>` for each
property, and `ap_<name>` = G !ap for each proposition, which fails iff
the proposition is reachable. Every claim is `X (...)` to skip the
initialising step. `finphi_<id>` and `finneg_<id>` are phi and !phi with
every run that ends in a deadlock let through; SPIN's ltl parser cannot
take the deadlock predicate inline, so each is a never claim from Spot's
ltl2tgba in its own `<claim>/model.pml`. OUT/jobs.json lists the
variants and claims.

usage: prepare.py BEEM_DATA OUT
"""
import json
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dve  # noqa: E402

LTL_OPS = {'G', 'F', 'U', 'R', 'X', 'W', 'M', 'true', 'false'}


def spin_ltl(formula, aps):
    """A BEEM formula in SPIN syntax, with each AP `a` renamed to `p_a`."""
    def rename(m):
        name = m.group(0)
        if name in LTL_OPS:
            return name
        if name not in aps and set(name) <= LTL_OPS:  # GF p
            return ' '.join(name)
        if name not in aps:
            raise NameError(f'unknown AP {name} in {formula}')
        return f'p_{name}'
    renamed = re.sub(r'[A-Za-z_][A-Za-z_0-9]*', rename, formula)
    r = subprocess.run(['ltlfilt', '--spin', '-f', renamed], text=True,
                       capture_output=True, check=True)
    return renamed, r.stdout.strip()


def beem_answers(data, model):
    """{(instance id, property id): 'yes' | 'no' | 'unknown'} and state counts."""
    answers, states = {}, {}
    f = Path(data) / model / 'generated_files' / 'results.xml'
    if f.exists():
        for i in ET.parse(f).getroot().findall('instance'):
            complete = i.find('reach-time').get('complete') == 'yes'
            states[i.get('id')] = int(i.find('states').text) if complete else None
            for lp in i.findall('ltl-prob'):
                answers[(i.get('id'), lp.get('prop-id'))] = lp.get('answer')
    return answers, states


def main(data, out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    variants = {}  # key -> job
    pairs = []
    for model, xml in dve.models(data):
        props = [p for p in xml.findall('property') if p.get('prop-type') == 'LTL']
        insts = {i.get('id'): dve.parse_params(i.find('parameters').text)
                 for i in xml.findall('instance')}
        answers, states = beem_answers(data, model)
        for iid, params in insts.items():
            if params.get('ERROR', '0') == '0' or not props:
                continue
            good = dict(params, ERROR='0')
            twin = next((j for j, q in insts.items()
                         if dict(q, ERROR=q.get('ERROR', '0')) == good), None)
            for variant, ps, beem_id in (('error', params, iid), ('correct', good, twin)):
                key = f'{model}-' + '-'.join(f'{k}{v}' for k, v in sorted(ps.items()))
                if key not in variants:
                    variants[key] = make_variant(data, model, ps, props, out / key)
                    variants[key]['beem_id'] = beem_id
                    variants[key]['beem_states'] = states.get(beem_id)
                for p in props:
                    pairs.append({
                        'instance': f'{model}.{iid}', 'params': ','.join(
                            f'{k}={v}' for k, v in params.items()),
                        'variant': variant, 'key': key, 'property': p.get('id'),
                        'formula': ' '.join(p.find('formula').text.split()),
                        'beem': answers.get((beem_id, p.get('id')), '') if beem_id else '',
                        'beem_instance': f'{model}.{beem_id}' if beem_id else ''})
    json.dump({'variants': variants, 'pairs': pairs}, open(out / 'jobs.json', 'w'), indent=1)
    print(len(variants), 'variants,', len(pairs), 'pairs', file=sys.stderr)


def make_variant(data, model, params, props, d):
    pml, _, aps = dve.instance(data, model, params)
    d.mkdir(parents=True, exist_ok=True)
    claims, fin = {}, {}
    lines = [pml] + [f'#define p_{a} ({e})' for a, e in aps.items()]
    for p in props:
        f = ' '.join(p.find('formula').text.split())
        _, phi = spin_ltl(f, aps)
        _, neg = spin_ltl(f'!({f})', aps)
        claims[f'phi_{p.get("id")}'] = phi
        claims[f'neg_{p.get("id")}'] = neg
        # The same two with every run that ends in a deadlock let through:
        # the verdicts of a checker that ignores finite runs.
        for tag, g in (('finphi', phi), ('finneg', neg)):
            fin[f'{tag}_{p.get("id")}'] = f'X (<> dve_dead || ({g}))'
    for a in aps:
        claims[f'ap_{a}'] = f'[] !p_{a}'
    for name, f in claims.items():
        lines.append(f'ltl {name} {{ X ({f}) }}')
    (d / 'model.pml').write_text('\n'.join(lines) + '\n')
    for name, f in fin.items():
        r = subprocess.run(['ltl2tgba', '--spin', '-f', f'!({spot(f)})'], text=True,
                           capture_output=True, check=True)
        (d / name).mkdir(exist_ok=True)
        (d / name / 'model.pml').write_text('\n'.join(lines[:len(aps) + 1]) + '\n' + r.stdout)
    return {'model': model, 'params': params, 'claims': list(claims) + list(fin)}


def spot(f):
    """A SPIN-syntax formula in Spot's syntax."""
    return f.replace('[]', 'G').replace('<>', 'F').replace(' V ', ' R ')


if __name__ == '__main__':
    main(*sys.argv[1:3])
