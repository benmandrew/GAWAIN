#!/usr/bin/env python3
"""Write one oracle_bench job per property, for RERS (SPIN) and LMCS-2006 (nuXmv).

usage: oracle_prep.py DATA_DIR OUT_DIR

DATA_DIR is bench/data. Each job is a directory OUT_DIR/<bench>/<property>/ holding
`atoms` and `properties` (and, for LMCS, `model.smv`), and a line of OUT_DIR/jobs.tsv:
bench, property, back end, model path, job directory. Paths in jobs.tsv are as given,
so run from the directory the arguments are relative to.

RERS: the properties are the model's `ltl pN { ... }` blocks, the atoms its #defines.

LMCS: the properties and models are those bench/lmcs_check.py checks. The oracle's
formulas have only future operators over boolean atoms, so each property is abstracted:
every comparison or bare variable becomes a DEFINE `aragon_aK`, and every maximal
past-time subformula becomes a monitor variable `aragon_mK`, set by INIT and TRANS
constraints added to the main module. A property with a future operator under a past
one cannot be monitored that way; it is listed in OUT_DIR/skipped.tsv instead.
"""
import re, sys
from pathlib import Path

RERS = [('rers2016', 'rers/2016/ParallelRERS2016_version2/ParallelRERS2016_version2', range(101, 107)),
        ('rers2017', 'rers/2017/ParallelRERS2017_version2/ParallelRERS2017', range(101, 106))]
LMCS_DIRS = {'bc57-sensors': 'reactor', 'dme': 'dme'}

TOKEN = re.compile(r'\s*(<->|->|!=|<=|>=|[()!&|=<>]|[A-Za-z_][\w.$#\[\]]*(?:-(?!>)[\w.$#\[\]]+)*|\d+)')
FUTURE1, PAST1 = {'G', 'F', 'X'}, {'Y', 'Z', 'H', 'O'}
FUTURE2, PAST2 = {'U': 'U', 'V': 'R'}, {'S', 'T'}
CMP = {'=', '!=', '<', '<=', '>', '>='}


def tokens(text):
    out, i = [], 0
    text = text.strip()
    while i < len(text):
        m = TOKEN.match(text, i)
        if not m:
            raise ValueError(f'cannot lex at {text[i:i + 20]!r}')
        out.append(m.group(1)); i = m.end()
    return out


class Parser:
    """NuSMV LTL with past operators, into tuples:
    ('atom', smv_text) | ('not', a) | (op, a) | (binop, a, b) | ('const', 'TRUE'/'FALSE')."""

    def __init__(self, text):
        self.t, self.i = tokens(text), 0

    def peek(self):
        return self.t[self.i] if self.i < len(self.t) else None

    def take(self, want=None):
        tok = self.peek()
        if want is not None and tok != want:
            raise ValueError(f'expected {want}, got {tok}')
        self.i += 1
        return tok

    def parse(self):
        e = self.implies()
        if self.peek() is not None:
            raise ValueError(f'trailing {self.peek()}')
        return e

    def implies(self):
        lhs = self.binary(['|'], self.conj)
        if self.peek() in ('->', '<->'):
            op = self.take()
            return (op, lhs, self.implies())
        return lhs

    def binary(self, ops, sub):
        e = sub()
        while self.peek() in ops:
            op = self.take(); e = (op, e, sub())
        return e

    def conj(self):
        return self.binary(['&'], self.temporal)

    def temporal(self):
        e = self.unary()
        if self.peek() in FUTURE2 or self.peek() in PAST2:
            op = self.take()
            return (op, e, self.temporal())
        return e

    def unary(self):
        tok = self.peek()
        if tok == '!':
            self.take(); return ('not', self.unary())
        if tok in FUTURE1 | PAST1 and self.t[self.i + 1:self.i + 2] == ['(']:
            self.take(); return (tok, self.unary())
        if tok == '(':
            self.take(); e = self.implies(); self.take(')'); return e
        if tok in ('TRUE', 'FALSE'):
            self.take(); return ('const', tok)
        name = self.take()
        if self.peek() in CMP:
            op = self.take(); value = self.take()
            return ('atom', f'{name} {op} {value}')
        return ('atom', name)


def is_past(e):
    return e[0] in PAST1 or e[0] in PAST2


def has(e, ops):
    return e[0] in ops or any(has(c, ops) for c in e[1:] if isinstance(c, tuple))


class Abstraction:
    def __init__(self):
        self.atoms, self.defines, self.monitors = [], {}, []

    def atom(self, smv):
        if smv not in self.defines:
            self.defines[smv] = f'aragon_a{len(self.defines)}'
            self.atoms.append(self.defines[smv])
        return self.defines[smv]

    def smv(self, e):
        """A propositional SMV expression for a past-or-propositional e, adding monitors."""
        k = e[0]
        if k == 'atom': return self.atom(e[1])
        if k == 'const': return e[1]
        if k == 'not': return f'!({self.smv(e[1])})'
        if k in ('&', '|', '->', '<->'): return f'({self.smv(e[1])}) {k} ({self.smv(e[2])})'
        if k in FUTURE1 or k in FUTURE2:
            raise ValueError(f'future operator {k} under a past operator')
        args = [self.smv(c) for c in e[1:]]
        m = f'aragon_m{len(self.monitors)}'
        a, b = args[0], args[-1]
        na, nb = f'next({a})', f'next({b})'
        init, trans = {
            'Y': ('FALSE', a),
            'Z': ('TRUE', a),
            'H': (a, f'{m} & {na}'),
            'O': (a, f'{m} | {na}'),
            'S': (b, f'{nb} | ({na} & {m})'),
            'T': (b, f'{nb} & ({na} | {m})'),
        }[k]
        self.monitors.append((m, init, trans))
        self.atoms.append(m)
        return m

    def spot(self, e):
        """The future part in Spot syntax, past subformulas replaced by monitors."""
        k = e[0]
        if is_past(e): return self.smv(e)
        if k == 'atom': return self.atom(e[1])
        if k == 'const': return e[1].lower()
        if k == 'not': return f'!({self.spot(e[1])})'
        if k in FUTURE1: return f'{k}({self.spot(e[1])})'
        op = FUTURE2.get(k, k)
        return f'({self.spot(e[1])}) {op} ({self.spot(e[2])})'

    def smv_text(self):
        lines = ['VAR'] + [f'  {m} : boolean;' for m, _, _ in self.monitors] if self.monitors else []
        lines += ['DEFINE'] + [f'  {n} := ({smv});' for smv, n in self.defines.items()]
        for m, init, trans in self.monitors:
            lines += [f'INIT {m} <-> ({init})', f'TRANS next({m}) <-> ({trans})']
        return '\n'.join(lines)


def add_to_main(model, extra):
    """Appends `extra` to the main module's sections."""
    m = re.search(r'(?m)^MODULE\s+main\b', model)
    if not m:
        raise ValueError('no main module')
    nxt = re.search(r'(?m)^MODULE\b', model[m.end():])
    at = m.end() + nxt.start() if nxt else len(model)
    return model[:at].rstrip() + '\n' + extra + '\n\n' + model[at:]


def job(out, jobs, bench, prop, backend, model, atoms, formula):
    d = out / bench / prop
    d.mkdir(parents=True, exist_ok=True)
    (d / 'atoms').write_text('\n'.join(atoms) + '\n')
    (d / 'properties').write_text(f'{prop}\t{formula}\n')
    jobs.append('\t'.join([bench, prop, backend, str(model), str(d)]))


def main(data, out):
    data, out = Path(data), Path(out)
    jobs, skipped = [], []
    for year, sub, problems in RERS:
        for n in problems:
            pml = data / sub / f'problem{n}' / f'problem{n}.pml'
            text = pml.read_text()
            atoms = [l.split()[1] for l in text.splitlines() if l.startswith('#define ')]
            for line in text.splitlines():
                m = re.match(r'ltl (p\d+)\s*\{(.*)\}', line)
                if m:
                    job(out, jobs, f'{year}-{n}', m.group(1), 'spin', pml, atoms,
                        m.group(2).strip())
    lmcs = data / 'lmcs2006'
    for line in (lmcs / 'README').read_text().splitlines():
        f = [x.strip() for x in line.split(',')]
        if not (len(f) >= 4 and f[3] in ('TRUE', 'FALSE', 'UNKNOWN') and f[1].isdigit()):
            continue
        model, prop = f[0], f[2]
        base = lmcs / 'fixed' / ('dme' if re.fullmatch(r'dme\d', model) else LMCS_DIRS.get(model, model))
        ltl = ' '.join((base / f'{prop}.ltl').read_text().split())
        a = Abstraction()
        try:
            formula = a.spot(Parser(ltl).parse())
        except ValueError as e:
            skipped.append(f'lmcs-{model}\t{prop}\t{e}')
            continue
        d = out / f'lmcs-{model}' / prop
        d.mkdir(parents=True, exist_ok=True)
        smv = d / 'model.smv'
        smv.write_text(add_to_main((base / f'{model}.smv').read_text(errors='replace'), a.smv_text()))
        job(out, jobs, f'lmcs-{model}', prop, 'nuxmv', smv, a.atoms, formula)
    (out / 'jobs.tsv').write_text('\n'.join(jobs) + '\n')
    (out / 'skipped.tsv').write_text('\n'.join(skipped) + '\n')
    print(f'{len(jobs)} jobs, {len(skipped)} skipped', file=sys.stderr)


if __name__ == '__main__':
    main(*sys.argv[1:3])
