#!/usr/bin/env python3
"""Check the m4 macros and the DVE parser against BEEM's own DVE files.

For every instance of every model, expands the model's .mdve with the
instance's parameters, parses it and the shipped generated_files/<m>.<i>.dve,
and compares the two parsed models (processes compared by name, so their
order does not matter).

usage: check_dve.py BEEM_DATA [MODEL ...]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import dve  # noqa: E402


def canon(model):
    return {**model, 'procs': sorted(model['procs'], key=lambda p: p['name']),
            'channels': sorted(model['channels'])}


def main(data, only):
    ok = bad = 0
    for name, xml in dve.models(data):
        if only and name not in only:
            continue
        mdve = (Path(data) / name / f'{name}.mdve').read_text()
        for inst in xml.findall('instance'):
            iid = inst.get('id')
            params = dve.parse_params(inst.find('parameters').text)
            shipped = Path(data) / name / 'generated_files' / f'{name}.{iid}.dve'
            try:
                mine = canon(dve.parse_model(dve.expand(mdve, params)))
                theirs = canon(dve.parse_model(shipped.read_text()))
            except Exception as e:  # report and go on
                print(f'{name}.{iid}: {type(e).__name__}: {e}')
                bad += 1
                continue
            if mine == theirs:
                ok += 1
            else:
                print(f'{name}.{iid}: differs')
                bad += 1
    print(f'{ok} match, {bad} differ or fail')


if __name__ == '__main__':
    main(sys.argv[1], set(sys.argv[2:]))
