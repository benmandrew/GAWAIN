#!/usr/bin/env python3
"""BEEM instances: expand with m4, parse DVE, and print flat Promela.

The Promela keeps DVE's step semantics. The whole system becomes one
process whose `do` loop has one `d_step` per DVE transition and one per
pair of synchronising transitions, so one SPIN step is one DVE step and
the never claim sees exactly the DVE states. Each process's control state
is a global `<proc>_pc`, and each local variable a global `<proc>_<var>`.
Promela cannot initialise an array element by element, so the system
starts with one extra `d_step` that sets the initial values, and every
claim is checked as `X phi` (see bench/notes/beem.md).

usage: dve.py BEEM_DATA MODEL PARAMS [AP ...]  prints the Promela
"""
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

M4 = Path(__file__).resolve().parent / 'beem.m4'

PROMELA_KEYWORDS = set('''active assert atomic bit bool break byte chan d_step
    do else empty enabled fi full goto hidden if init int len mtype nempty
    never nfull od of pc_value printf priority proctype provided run short
    skip timeout typedef unless unsigned xr xs np_ eval true false'''.split())

# BEEM's own DVE files declare every bool as byte, so this does too.
TYPES = {'byte': 'byte', 'int': 'short', 'bool': 'byte'}
RANGE = {'byte': (0, 255), 'int': (-32768, 32767)}


def expand(text, params):
    """Run the BEEM m4 macros over `text` with the instance's parameters."""
    defs = [f'-D{k}={v}' for k, v in params.items()]
    r = subprocess.run(['m4', *defs, str(M4), '-'], input=text, text=True,
                       capture_output=True, check=True)
    return r.stdout


def parse_params(s):
    """'N=3, ERROR=1' -> {'N': '3', 'ERROR': '1'}"""
    out = {}
    for part in (s or '').split(','):
        if part.strip():
            k, v = part.split('=')
            out[k.strip()] = v.strip()
    return out


# ---------------------------------------------------------------- lexer

TOKEN = re.compile(r'''\s+|//[^\n]*|/\*.*?\*/
    |(?P<num>\d+)
    |(?P<id>[A-Za-z_][A-Za-z_0-9]*)
    |(?P<op><<|>>|<=|>=|==|!=|&&|\|\||->|[-+*/%<>=!~&|^(){}\[\],;.?:])''',
                   re.S | re.X)


def lex(text):
    toks, pos = [], 0
    while pos < len(text):
        m = TOKEN.match(text, pos)
        if not m:
            raise SyntaxError(f'bad character at {text[pos:pos + 30]!r}')
        pos = m.end()
        if m.lastgroup:
            toks.append(m.group(m.lastgroup))
    return toks


class Parser:
    def __init__(self, toks):
        self.t, self.i = toks, 0

    def peek(self, k=0):
        return self.t[self.i + k] if self.i + k < len(self.t) else None

    def next(self):
        tok = self.peek()
        self.i += 1
        return tok

    def eat(self, tok):
        if self.peek() != tok:
            raise SyntaxError(f'expected {tok!r}, got {self.peek()!r} at token {self.i}: '
                              + ' '.join(self.t[max(0, self.i - 8):self.i + 8]))
        return self.next()

    def accept(self, tok):
        if self.peek() == tok:
            self.i += 1
            return True
        return False

    # Expressions are parsed to trees: ('num', n), ('var', name, index),
    # ('dot', proc, name), ('un', op, e), ('bin', op, a, b).
    BINARY = [['imply'], ['or', '||'], ['and', '&&'], ['|'], ['^'], ['&'],
              ['==', '!='], ['<', '<=', '>', '>='], ['<<', '>>'], ['+', '-'],
              ['*', '/', '%']]
    CANON = {'or': '||', 'and': '&&'}

    def expr(self, level=0):
        if level == len(self.BINARY):
            return self.unary()
        lhs = self.expr(level + 1)
        while self.peek() in self.BINARY[level]:
            op = self.next()
            rhs = self.expr(level + 1)
            lhs = ('bin', self.CANON.get(op, op), lhs, rhs)
        return lhs

    def unary(self):
        tok = self.peek()
        if tok in ('not', '!'):
            self.next()
            return ('un', '!', self.unary())
        if tok in ('-', '~'):
            self.next()
            return ('un', tok, self.unary())
        if tok == '(':
            self.next()
            e = self.expr()
            self.eat(')')
            return e
        if tok.isdigit():
            self.next()
            return ('num', int(tok))
        if tok in ('true', 'false'):
            self.next()
            return ('num', int(tok == 'true'))
        name = self.next()
        if not re.fullmatch(r'[A-Za-z_]\w*', name):
            raise SyntaxError(f'unexpected {name!r}')
        if self.accept('.'):
            return ('dot', name, self.next())
        if self.accept('['):
            idx = self.expr()
            self.eat(']')
            return ('var', name, idx)
        return ('var', name, None)


def parse_ident_list(p):
    names = [p.next()]
    while p.accept(','):
        names.append(p.next())
    p.eat(';')
    return names


def parse_decl(p, typ):
    """One `type a = 1, b[3] = {..};` declaration -> list of variables."""
    out = []
    while True:
        name = p.next()
        size = None
        if p.accept('['):
            size = int(p.next())
            p.eat(']')
        init = None
        if p.accept('='):
            if p.accept('{'):
                init = [p.expr()]
                while p.accept(','):
                    init.append(p.expr())
                p.eat('}')
            else:
                init = p.expr()
        out.append({'type': typ, 'name': name, 'size': size, 'init': init})
        if p.accept(';'):
            return out
        p.eat(',')


def parse_model(text):
    p = Parser(lex(text))
    model = {'globals': [], 'channels': [], 'procs': []}

    def decl_into(lst):
        if p.peek() == 'const':
            p.next()
        typ = p.next()
        lst.extend(parse_decl(p, 'byte' if typ == 'bool' else typ))

    while p.peek() is not None:
        tok = p.peek()
        if tok in TYPES or tok == 'const':
            decl_into(model['globals'])
        elif tok == 'channel':
            p.next()
            if p.peek() == '{':
                raise SyntaxError('buffered channels are not supported')
            model['channels'].extend(parse_ident_list(p))
        elif tok == 'process':
            p.next()
            proc = {'name': p.next(), 'vars': [], 'trans': []}
            p.eat('{')
            while p.peek() in TYPES or p.peek() == 'const':
                decl_into(proc['vars'])
            p.eat('state')
            proc['states'] = parse_ident_list(p)
            p.eat('init')
            proc['init'] = p.next()
            p.eat(';')
            while p.peek() in ('accept', 'commit', 'assert'):
                kind = p.next()
                if kind == 'commit':
                    raise SyntaxError('committed states are not supported')
                while not p.accept(';'):  # assertions do not bear on LTL
                    p.next()
            if p.accept('trans'):
                while True:
                    proc['trans'].append(parse_trans(p))
                    if p.accept(';'):
                        break
                    p.eat(',')
            p.eat('}')
            model['procs'].append(proc)
        elif tok == 'system':
            p.next()
            if p.next() != 'async':
                raise SyntaxError('only `system async` is supported')
            p.eat(';')
        else:
            raise SyntaxError(f'unexpected {tok!r}')
    return model


def parse_trans(p):
    t = {'src': p.next()}
    p.eat('->')
    t['dst'] = p.next()
    t['guard'] = None
    t['sync'] = None
    t['effect'] = []
    p.eat('{')
    while not p.accept('}'):
        kw = p.next()
        if kw == 'guard':
            t['guard'] = p.expr()
            p.eat(';')
        elif kw == 'sync':
            chan = p.next()
            d = p.next()
            val = None if p.peek() == ';' else p.expr()
            t['sync'] = (chan, d, val)
            p.eat(';')
        elif kw == 'effect':
            while True:
                lhs = p.unary()
                p.eat('=')
                t['effect'].append((lhs, p.expr()))
                if p.accept(';'):
                    break
                p.eat(',')
        else:
            raise SyntaxError(f'unexpected {kw!r} in transition')
    return t


# ---------------------------------------------------------------- Promela

class Names:
    """Resolves DVE names to Promela names."""

    def __init__(self, model):
        self.globals = {v['name'] for v in model['globals']}
        self.procs = {p['name']: p for p in model['procs']}
        self.local = {p['name']: {v['name'] for v in p['vars']} for p in model['procs']}
        self.state = {p['name']: {s: i for i, s in enumerate(p['states'])}
                      for p in model['procs']}
        self.types = {self.safe(v['name']): v['type'] for v in model['globals']}
        self.types.update({f'{p["name"]}_{v["name"]}': v['type']
                           for p in model['procs'] for v in p['vars']})
        self.sizes = {self.safe(v['name']): v['size'] for v in model['globals']}
        self.sizes.update({f'{p["name"]}_{v["name"]}': v['size']
                           for p in model['procs'] for v in p['vars']})

    @staticmethod
    def safe(name):
        return name + '_' if name in PROMELA_KEYWORDS else name

    def var(self, proc, name):
        if proc and name in self.local[proc]:
            return f'{proc}_{name}'
        if name in self.globals:
            return self.safe(name)
        raise NameError(f'unknown variable {name} in {proc}')

    def pc(self, proc):
        return f'{proc}_pc'

    def resolve(self, e, proc):
        """`e` with every name resolved: ('pv', promela name, index) for a
        variable and ('bin', '==', pc, k) for a control state."""
        kind = e[0]
        if kind == 'var':
            name = self.var(proc, e[1])
            idx = self.resolve(e[2], proc) if e[2] is not None else (
                ('num', 0) if self.sizes[name] else None)  # train-gate's bare `e`
            return ('pv', name, idx)
        if kind == 'dot':
            other, name = e[1], e[2]
            if name in self.state[other]:
                return ('bin', '==', ('pv', self.pc(other), None),
                        ('num', self.state[other][name]))
            return ('pv', self.var(other, name), None)
        if kind == 'un':
            return ('un', e[1], self.resolve(e[2], proc))
        if kind == 'bin':
            return ('bin', e[1], self.resolve(e[2], proc), self.resolve(e[3], proc))
        return e

    def in_range(self, effects):
        """Guards that keep a step inside DVE's ranges: every assigned value
        fits its variable's type and every assigned index fits its array.
        A step that breaks either is not taken, which matches BEEM's state
        counts (bench/notes/beem.md); Promela would wrap the value or stop
        with an index error instead."""
        conds, writes = [], []
        for (_, name, idx), rhs in effects:
            val = substitute(rhs, writes)
            lo, hi = RANGE[self.types[name]]
            if not (val[0] == 'num' and lo <= val[1] <= hi):
                v = emit(val)
                conds.append(f'{lo} <= {v} && {v} <= {hi}')
            if idx is not None:
                idx = substitute(idx, writes)
                if idx[0] != 'num':
                    conds.append(f'0 <= {emit(idx)} && {emit(idx)} < {self.sizes[name]}')
            writes.append((name, idx, val))
        return conds

    def emit(self, e, proc):
        kind = e[0]
        if kind == 'num':
            return str(e[1])
        if kind == 'var':
            name = self.var(proc, e[1])
            if e[2] is None:  # an array named bare is its element 0
                return f'{name}[0]' if self.sizes[name] else name
            return f'{name}[{self.emit(e[2], proc)}]'
        if kind == 'dot':
            other, name = e[1], e[2]
            if name in self.state[other]:
                return f'({other}_pc == {self.state[other][name]})'
            return self.var(other, name)
        if kind == 'un':
            return f'({e[1]}{self.emit(e[2], proc)})'
        _, op, a, b = e
        a, b = self.emit(a, proc), self.emit(b, proc)
        if op == 'imply':
            return f'(!{a} || {b})'
        if op == '->':
            raise SyntaxError('-> in an expression')
        return f'({a} {op} {b})'


def promela(model):
    n = Names(model)
    out, init = [], []

    def declare(v, name):
        typ = TYPES[v['type']]
        if v['size'] is None:
            out.append(f'{typ} {name};')
            if v['init'] is not None:
                init.append(f'{name} = {n.emit(v["init"], None)}')
        else:
            out.append(f'{typ} {name}[{v["size"]}];')
            vals = v['init'] if isinstance(v['init'], list) else (
                [v['init']] * v['size'] if v['init'] is not None else [])
            for k, val in enumerate(vals[:v['size']]):  # anderson gives N+1 values
                init.append(f'{name}[{k}] = {n.emit(val, None)}')

    for v in model['globals']:
        declare(v, n.safe(v['name']))
    for p in model['procs']:
        out.append(f'byte {p["name"]}_pc;')
        init.append(f'{p["name"]}_pc = {n.state[p["name"]][p["init"]]}')
        for v in p['vars']:
            declare(v, f'{p["name"]}_{v["name"]}')

    def part(proc, t):
        """The guard and the effects of one transition of `proc`."""
        g = [f'{n.pc(proc)} == {n.state[proc][t["src"]]}']
        if t['guard'] is not None:
            g.append(n.emit(t['guard'], proc))
        eff = [(n.resolve(lhs, proc), n.resolve(rhs, proc)) for lhs, rhs in t['effect']]
        return g, eff, f'{n.pc(proc)} = {n.state[proc][t["dst"]]}'

    steps = []
    sends, recvs = {}, {}
    for p in model['procs']:
        for t in p['trans']:
            if t['sync'] is None:
                g, eff, pc = part(p['name'], t)
                steps.append((g, eff, [pc]))
            else:
                chan, d, _ = t['sync']
                (sends if d == '!' else recvs).setdefault(chan, []).append((p['name'], t))
    for chan in model['channels']:
        for sp, st in sends.get(chan, []):
            for rp, rt in recvs.get(chan, []):
                if sp == rp:
                    continue
                gs, es, pcs = part(sp, st)
                gr, er, pcr = part(rp, rt)
                sval, rval = st['sync'][2], rt['sync'][2]
                if (sval is None) != (rval is None):
                    raise SyntaxError(f'value mismatch on channel {chan}')
                # DVE: pass the value, then the sender's effect, then the receiver's.
                xfer = [] if rval is None else [(n.resolve(rval, rp), n.resolve(sval, sp))]
                steps.append((gs + gr, xfer + es + er, [pcs, pcr]))
    out.append('')
    out.append('active proctype dve() {')
    out.append('  d_step { ' + '; '.join(init) + ' };')
    out.append('  do')
    guards = []
    for g, eff, pcs in steps:
        guards.append('(' + ') && ('.join(g + n.in_range(eff)) + ')')
        body = [f'{emit(lhs)} = {emit(rhs)}' for lhs, rhs in eff] + pcs
        out.append(f'  :: d_step {{ {guards[-1]} -> {"; ".join(body)} }}')
    out.append('  od')
    out.append('}')
    # No step is enabled. A deadlock is absorbing, so `<> dve_dead` says the
    # run is finite in DVE terms (SPIN stutters on its last state).
    out.append('#define dve_dead (!(' + ' || '.join(guards or ['0']) + '))')
    return '\n'.join(out) + '\n', n


def emit(e):
    """A resolved expression (see Names.resolve) as Promela."""
    kind = e[0]
    if kind == 'num':
        return str(e[1])
    if kind == 'pv':
        return e[1] if e[2] is None else f'{e[1]}[{emit(e[2])}]'
    if kind == 'un':
        return f'({e[1]}{emit(e[2])})'
    if kind == 'cond':
        return f'({emit(e[1])} -> {emit(e[2])} : {emit(e[3])})'
    _, op, a, b = e
    if op == 'imply':
        return f'(!{emit(a)} || {emit(b)})'
    return f'({emit(a)} {op} {emit(b)})'


def substitute(e, writes):
    """`e` read after `writes`, as an expression over the state before them."""
    kind = e[0]
    if kind == 'pv':
        idx = None if e[2] is None else substitute(e[2], writes)
        out = ('pv', e[1], idx)
        for name, widx, rhs in writes:
            if name == e[1]:
                out = rhs if idx is None else ('cond', ('bin', '==', idx, widx), rhs, out)
        return out
    if kind == 'un':
        return ('un', e[1], substitute(e[2], writes))
    if kind == 'bin':
        return ('bin', e[1], substitute(e[2], writes), substitute(e[3], writes))
    return e


def proposition(text, names):
    """A DVE proposition expression, as a Promela expression."""
    p = Parser(lex(text))
    e = p.expr()
    if p.peek() is not None:
        raise SyntaxError(f'trailing {p.peek()!r} in proposition {text!r}')
    return names.emit(e, None)


# ---------------------------------------------------------------- BEEM index

def models(data):
    """Each model's XML description, parsed."""
    for x in sorted(Path(data).glob('*/*.xml')):
        yield x.parent.name, ET.parse(x).getroot()


def instance(data, model, params):
    """(Promela text, Names, {ap: promela expr}) for one parameter set."""
    d = Path(data) / model
    xml = ET.parse(d / f'{model}.xml').getroot()
    dve = expand((d / f'{model}.mdve').read_text(), params)
    pml, names = promela(parse_model(dve))
    aps = {}
    for ap in xml.findall('ap'):
        text = ap.find('ap-expression').text
        if ap.get('parametrized') == 'yes':
            text = expand(text, params)
        aps[ap.find('ap-name').text.strip()] = proposition(text.strip(), names)
    return pml, names, aps


if __name__ == '__main__':
    pml, _, aps = instance(sys.argv[1], sys.argv[2], parse_params(sys.argv[3]))
    sys.stdout.write(pml)
    for a, e in aps.items():
        print(f'#define p_{a} {e}')
