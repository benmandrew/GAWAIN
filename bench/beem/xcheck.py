#!/usr/bin/env python3
"""Cross-check: run the BEEM properties on BEEM's own Promela translation.

BEEM ships a Promela file for most instances, one proctype per DVE process
with a label per DVE state. Here each proposition is translated with
remote references (`P@state`, `P:var`) and checked with SPIN, without the
flat translation of dve.py, so the two routes share only the m4 expansion
of the propositions. Prints one line per check: instance, claim, verdict.

usage: xcheck.py BEEM_DATA WORKDIR MODEL.ID ...
"""
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dve  # noqa: E402
import prepare  # noqa: E402

RENAMED = {'len': 'ln'}  # names BEEM's translator changed


class RemoteNames(dve.Names):
    def emit(self, e, proc):
        if e[0] == 'dot':
            other, name = e[1], e[2]
            if name in self.state[other]:
                return f'({other}@{name})'
            return f'{other}:{RENAMED.get(name, name)}'
        if e[0] == 'var' and e[2] is None:
            return RENAMED.get(e[1], e[1])
        if e[0] == 'var':
            return f'{RENAMED.get(e[1], e[1])}[{self.emit(e[2], proc)}]'
        return super().emit(e, proc)


def main(data, work, insts):
    for inst in insts:
        model, iid = inst.rsplit('.', 1)
        d = Path(data) / model
        xml = ET.parse(d / f'{model}.xml').getroot()
        params = dve.parse_params(next(i for i in xml.findall('instance')
                                       if i.get('id') == iid).find('parameters').text)
        names = RemoteNames(dve.parse_model(dve.expand((d / f'{model}.mdve').read_text(), params)))
        aps = {}
        for ap in xml.findall('ap'):
            text = ap.find('ap-expression').text
            if ap.get('parametrized') == 'yes':
                text = dve.expand(text, params)
            p = dve.Parser(dve.lex(text.strip()))
            aps[ap.find('ap-name').text.strip()] = names.emit(p.expr(), None)
        lines = [(d / 'generated_files' / f'{model}.{iid}.pm').read_text()]
        lines += [f'#define p_{a} ({e})' for a, e in aps.items()]
        claims = []
        for p in xml.findall('property'):
            if p.get('prop-type') != 'LTL':
                continue
            f = ' '.join(p.find('formula').text.split())
            for tag, g in (('phi', f), ('neg', f'!({f})')):
                claims.append(f'{tag}_{p.get("id")}')
                lines.append(f'ltl {claims[-1]} {{ {prepare.spin_ltl(g, aps)[1]} }}')
        w = Path(work) / inst
        w.mkdir(parents=True, exist_ok=True)
        (w / 'model.pml').write_text('\n'.join(lines) + '\n')
        r = subprocess.run(['spin', '-a', 'model.pml'], cwd=w, capture_output=True, text=True)
        if r.returncode:  # train-gate's file uses an array as a scalar
            print(inst, 'all', 'BUILD-FAILED', r.stdout.splitlines()[:1], flush=True)
            continue
        subprocess.run(['gcc', '-O2', '-DNOREDUCE', '-DCOLLAPSE', '-DMEMLIM=6000', '-o', 'pan',
                        'pan.c'], cwd=w, check=True, capture_output=True)
        for c in claims:
            try:
                o = subprocess.run(['./pan', '-a', '-m50000000', '-w26', '-n', '-N', c], cwd=w,
                                   capture_output=True, text=True, timeout=600).stdout
                v = 'UNKNOWN' if 'depth too small' in o else (
                    'TRUE' if 'errors: 0' in o else 'FALSE' if re.search(
                        r'acceptance cycle|assertion violated', o) else 'UNKNOWN')
            except subprocess.TimeoutExpired:
                v = 'UNKNOWN'
            print(inst, c, v, flush=True)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], sys.argv[3:])
