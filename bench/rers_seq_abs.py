#!/usr/bin/env python3
"""Decide RERS 2016/2017 sequential properties on a sound interval abstraction.

The arithmetic globals of a RERS sequential program (those assigned an
expression with `%`) take tens of thousands of values, which makes the
explicit state space too large to finish, but the program tests them only
against constants (`a70 <= 160`). This script keeps every other global
exact and replaces each arithmetic global by the *cell* it lies in: the
maximal interval of 32-bit integers on which every guard comparison of
that global with a constant has a fixed truth value. A step evaluates the
rule table over intervals, with C's 32-bit -fwrapv semantics for `+`, `-`,
`*`, `/` and `%`; a comparison whose truth the intervals do not fix sends
the search down both branches, and an assigned interval moves to every
cell it meets. Every run of the program is therefore a run of the abstract
graph, so a property that holds on the graph holds on the program. A
property that fails on the graph may fail only through an abstract run;
for the fix and its negation, the script asks Spot for a violating lasso
of the graph and reports `fails` only when the compiled program replays
it (bench/rers_seq.py's replay) and Spot accepts the replayed word in the
automaton of the negation, and `unknown` otherwise.

The rule table comes from the parser in bench/rers_seq_smv.py, which
refuses any file outside the event-condition-action shape. An array read
whose index interval leaves the array is counted; it then yields every
value, and a problem with a nonzero count is reported but its verdicts
are still sound for the compiled program only if such reads never run.

  rers_seq_abs.py check --years 2016 --problems 3 --out-dir DIR

writes DIR/abs-YEAR-PROB.tsv with every shipped property (to compare with
the official verdicts), and for each mistranslated one the fix and the
negations used for the degeneracy checks. Needs ltl2tgba and autfilt.
"""

import argparse
import array
import collections
import itertools
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import rers_seq
import rers_seq_smv as smv
import rers_seq_walk

MIN, MAX = -(1 << 31), (1 << 31) - 1
FULL = (MIN, MAX)
CMP = ("<", "<=", ">", ">=", "==", "!=")
FLIP = {"<": ">", "<=": ">=", ">": "<", ">=": "<="}
PYOP = {"<": "<", "<=": "<=", ">": ">", ">=": ">=", "==": "==", "!=": "!="}


class Unsupported(Exception):
    """The abstraction cannot handle this program."""


# --- Interval arithmetic on C int with -fwrapv ------------------------------

def wrap(lo, hi):
    """The smallest interval holding every value of [lo, hi] wrapped to int32."""
    if MIN <= lo and hi <= MAX:
        return (lo, hi)
    if hi - lo >= 1 << 32:
        return FULL
    a = (lo - MIN) % (1 << 32) + MIN
    b = (hi - MIN) % (1 << 32) + MIN
    return (a, b) if a <= b else FULL


def tdiv(x, y):
    """C division: truncates towards zero."""
    q = abs(x) // abs(y)
    return q if (x >= 0) == (y >= 0) else -q


def div(a, b):
    """C division. A zero divisor traps (SIGFPE on x86) and ends the run,
    so it adds no value, and neither does the trapping INT_MIN / -1."""
    qs = []
    for lo, hi in ((b[0], min(b[1], -1)), (max(b[0], 1), b[1])):
        if lo <= hi:
            qs += [tdiv(x, y) for x in a for y in (lo, hi)]
    if not qs:
        raise Unsupported("division by zero")
    return wrap(min(qs), max(qs))


def mod(a, b):
    """C remainder: takes the dividend's sign, and |a % b| < |b|, <= |a|.
    As with division, a zero divisor ends the run."""
    if b == (0, 0):
        raise Unsupported("remainder by zero")
    if b[0] == b[1]:
        m = abs(b[0])
        if a[0] >= 0 and a[0] // m == a[1] // m:
            return (a[0] % m, a[1] % m)
        if a[1] <= 0 and (-a[1]) // m == (-a[0]) // m:
            return (-((-a[0]) % m), -((-a[1]) % m))
    m = max(abs(b[0]), abs(b[1])) - 1
    lo = 0 if a[0] >= 0 else max(a[0], -m)
    hi = 0 if a[1] <= 0 else min(a[1], m)
    return (lo, hi)


def arith(op, a, b):
    if op == "+":
        return wrap(a[0] + b[0], a[1] + b[1])
    if op == "-":
        return wrap(a[0] - b[1], a[1] - b[0])
    if op == "*":
        ps = [x * y for x in a for y in b]
        return wrap(min(ps), max(ps))
    if op == "/":
        return div(a, b)
    return mod(a, b)


def compare(op, a, b):
    """Three-valued comparison: True, False or None (both possible)."""
    if op == "<":
        return True if a[1] < b[0] else False if a[0] >= b[1] else None
    if op == "<=":
        return True if a[1] <= b[0] else False if a[0] > b[1] else None
    if op == ">":
        return compare("<", b, a)
    if op == ">=":
        return compare("<=", b, a)
    if a[0] == a[1] == b[0] == b[1]:
        eq = True
    elif a[1] < b[0] or b[1] < a[0]:
        eq = False
    else:
        eq = None
    if op == "==":
        return eq
    return None if eq is None else not eq


# --- Evaluation --------------------------------------------------------------

class Machine:
    """One problem's rule table and its abstract semantics."""

    def __init__(self, year, prob):
        src = rers_seq.problem_source(year, prob).read_text()
        g, funcs, inputs, arrays = smv.Parser(smv.tokenize(src)).program()
        g.pop("cf")
        self.inputs, self.arrays = inputs, arrays
        self.rules = {v: smv.rules_for_input(funcs, v, arrays) for v in inputs}
        for rs in self.rules.values():
            for _, body in rs:
                smv.check_body(body)
        self.names = sorted(g)
        self.init = g
        self.abstract = set()
        for rs in self.rules.values():
            for _, body in rs:
                self._find_arith(body)
        self.slot = {v: k for k, v in enumerate(self.names)}
        # The arrays each pointer global can point to.
        self.targets = {v: {x} for v, x in g.items() if str(x).startswith("t_")}
        for rs in self.rules.values():
            for _, body in rs:
                self._find_targets(body)
        self.oob = 0
        self.ranges = self._ranges()
        self.oob = 0
        self.cells = {}
        for v, cells in self._all_cells().items():
            lo, hi = self.ranges[v]
            self.cells[v] = [(max(a, lo), min(b, hi)) for a, b in cells if a <= hi and lo <= b]
        self.absnames = sorted(self.abstract)
        self.absslots = [self.slot[v] for v in self.absnames]
        self.fast = self._compile_guards()

    def _ranges(self, rounds=200):
        """An interval for every numeric global that holds in every
        reachable state: it holds initially, and every rule that can fire
        from a state inside the intervals (judged by the guard's top-level
        comparisons with constants) leaves its results inside. Found by
        iterating to a fixed point; a bound still moving after `rounds`
        rounds is widened to the int32 limit."""
        env = {v: frozenset(self.targets[v]) if v in self.targets else (x, x)
               for v, x in self.init.items()}
        moved = {v: 0 for v in env}
        for _ in range(rounds + 2):
            changed = False
            for rs in self.rules.values():
                for guard, body in rs:
                    local = self._restrict(guard, env)
                    if local is None:
                        continue
                    for after, _ in self.outcomes(body, local):
                        for v in env:
                            a, b = env[v], after[v]
                            if a is b:
                                continue
                            if isinstance(a, frozenset):
                                new = a | ({b} if isinstance(b, str) else b)
                            else:
                                new = (min(a[0], b[0]), max(a[1], b[1]))
                                if new != a:
                                    moved[v] += 1
                                    if moved[v] > rounds:
                                        new = (MIN if new[0] < a[0] else new[0],
                                               MAX if new[1] > a[1] else new[1])
                            if new != a:
                                env[v] = new
                                changed = True
            if not changed:
                return {v: x for v, x in env.items() if not isinstance(x, frozenset)}
        raise AssertionError("range iteration did not converge")

    def _restrict(self, guard, env):
        """env narrowed to the states where every condition of the guard
        can hold, judged by its comparisons of a global with a constant
        under `&&` and `||`, or None when the guard cannot hold."""
        for c in guard:
            env = self._narrow(c, env)
            if env is None:
                return None
        return env

    def _narrow(self, e, env):
        if e[0] == "bin" and e[1] == "&&":
            env = self._narrow(e[2], env)
            return None if env is None else self._narrow(e[3], env)
        if e[0] == "bin" and e[1] == "||":
            a, b = self._narrow(e[2], env), self._narrow(e[3], env)
            if a is None or b is None:
                return a if b is None else b
            out = dict(a)
            for v, x in b.items():
                if x != a[v]:
                    out[v] = (min(x[0], a[v][0]), max(x[1], a[v][1]))
            return out
        if e[0] != "bin" or e[1] not in CMP:
            return env
        a, op, b = e[2], e[1], e[3]
        if b[0] == "var" and a[0] == "num":
            a, b, op = b, a, FLIP.get(op, op)
        if a[0] != "var" or b[0] != "num" or isinstance(env[a[1]], frozenset):
            return env
        lo, hi = env[a[1]]
        c = b[1]
        if op == "<":
            hi = min(hi, c - 1)
        elif op == "<=":
            hi = min(hi, c)
        elif op == ">":
            lo = max(lo, c + 1)
        elif op == ">=":
            lo = max(lo, c)
        elif op == "==":
            lo, hi = max(lo, c), min(hi, c)
        if lo > hi:
            return None
        if (lo, hi) == env[a[1]]:
            return env
        env = dict(env)
        env[a[1]] = (lo, hi)
        return env

    def _find_targets(self, stmts):
        for s in stmts:
            if s[0] == "set" and s[2][0] == "ptr":
                self.targets[s[1]].add(s[2][1])
            elif s[0] == "if":
                self._find_targets(s[2])
                self._find_targets(s[3])

    def _find_arith(self, stmts):
        def has_mod(e):
            if e[0] == "bin":
                return e[1] == "%" or has_mod(e[2]) or has_mod(e[3])
            if e[0] == "un":
                return has_mod(e[2])
            return False
        for s in stmts:
            if s[0] == "set" and has_mod(s[2]):
                self.abstract.add(s[1])
            elif s[0] == "if":
                self._find_arith(s[2])
                self._find_arith(s[3])

    def _atoms(self, f):
        """Call f on every comparison in a guard or a rule-body condition."""
        def atoms(e):
            if e[0] == "bin" and e[1] in ("&&", "||"):
                atoms(e[2])
                atoms(e[3])
            elif e[0] == "un" and e[1] == "!":
                atoms(e[2])
            elif e[0] == "bin" and e[1] in CMP:
                f(e)

        def walk(stmts):
            for s in stmts:
                if s[0] == "if":
                    atoms(s[1])
                    walk(s[2])
                    walk(s[3])

        for rs in self.rules.values():
            for guard, body in rs:
                for c in guard:
                    atoms(c)
                walk(body)

    def _all_cells(self):
        """The cells of each abstract global: the cut points are where some
        comparison `v op constant` changes value (a cut t separates t - 1
        from t)."""
        cuts = {v: set() for v in self.abstract}

        def note(e):
            a, b = e[2], e[3]
            op = e[1]
            if b[0] == "var" and a[0] == "num":
                a, b, op = b, a, FLIP.get(op, op)
            if a[0] == "var" and a[1] in cuts and b[0] == "num":
                c = b[1]
                cuts[a[1]].update({"<": [c], ">=": [c], "<=": [c + 1], ">": [c + 1]}
                                  .get(op, [c, c + 1]))

        self._atoms(note)
        cells = {}
        for v, cs in cuts.items():
            bounds = [MIN] + sorted(t for t in cs if MIN < t <= MAX) + [MAX + 1]
            cells[v] = [(bounds[k], bounds[k + 1] - 1) for k in range(len(bounds) - 1)]
        return cells

    def _compile(self, e, consts):
        """Python source for the two-valued truth of e on a state tuple `s`,
        or None when some comparison is not fixed by the state alone."""
        if e[0] == "bin" and e[1] in ("&&", "||"):
            a, b = self._compile(e[2], consts), self._compile(e[3], consts)
            if a is None or b is None:
                return None
            return f"({a} {'and' if e[1] == '&&' else 'or'} {b})"
        if e[0] == "un" and e[1] == "!":
            a = self._compile(e[2], consts)
            return None if a is None else f"(not {a})"
        if e[0] != "bin" or e[1] not in CMP:
            return None
        a, op, b = e[2], e[1], e[3]
        if b[0] in ("var", "idx") and a[0] == "num":
            a, b, op = b, a, FLIP.get(op, op)
        if b[0] != "num":
            return None
        c = b[1]
        if a[0] == "var":
            v = a[1]
            i = self.slot[v]
            if v in self.abstract:
                true = frozenset(k for k, cell in enumerate(self.cells[v])
                                 if compare(op, cell, (c, c)))
                consts.append(true)
                return f"(s[{i}] in K[{len(consts) - 1}])"
            return f"(s[{i}] {PYOP[op]} {c})"
        if a[0] == "idx" and a[2][0] == "num" and a[1] in self.slot:
            k = a[2][1]
            table = {}
            for t in self.targets[a[1]]:
                vals = self.arrays[t[2:]]
                if not 0 <= k < len(vals):
                    return None
                table[t] = compare(op, (vals[k], vals[k]), (c, c))
            consts.append(table)
            return f"K[{len(consts) - 1}][s[{self.slot[a[1]]}]]"
        return None

    def _compile_guards(self):
        """Per input, a function of the state that lists the rules that may
        fire, in order: the list ends at the first rule whose compiled guard
        holds, and a rule whose guard did not compile is listed for
        `step` to evaluate over intervals. Also per input, the rules."""
        consts = []
        pick = {}
        self.compiled = {}
        for inp, rs in self.rules.items():
            self.compiled[inp] = set()
            lines = ["def pick(s):", "    out = []"]
            for k, (guard, _) in enumerate(rs):
                parts = [self._compile(c, consts) for c in guard]
                if all(x is not None for x in parts):
                    self.compiled[inp].add(k)
                    lines.append(f"    if {' and '.join(parts) or 'True'}:")
                    lines.append(f"        out.append({k})")
                    lines.append("        return out")
                else:
                    lines.append(f"    out.append({k})")
            lines.append("    return out")
            space = {"K": consts}
            exec("\n".join(lines), space)
            pick[inp] = space["pick"]
        return pick

    def cell_of(self, v, x):
        for k, (lo, hi) in enumerate(self.cells[v]):
            if lo <= x <= hi:
                return k
        raise AssertionError

    def cells_meeting(self, v, iv):
        return [k for k, (lo, hi) in enumerate(self.cells[v]) if lo <= iv[1] and iv[0] <= hi]

    # An environment maps a global to an interval, or a pointer global to
    # the name of its target array.

    def value(self, e, env):
        kind = e[0]
        if kind == "num":
            return (e[1], e[1])
        if kind == "var":
            return env[e[1]]
        if kind == "idx":
            name = e[1]
            if name not in self.arrays:
                t = env[name]
                if not isinstance(t, str):  # a set of targets, in _ranges
                    vs = [self.value(("idx", x[2:], e[2]), env) for x in sorted(t)]
                    return (min(v[0] for v in vs), max(v[1] for v in vs))
                name = t[2:]
            vals = self.arrays[name]
            i = self.value(e[2], env)
            if i[0] < 0 or i[1] >= len(vals):
                self.oob += 1
                return FULL
            sel = vals[i[0]:i[1] + 1]
            return (min(sel), max(sel))
        if kind == "un":
            if e[1] == "-":
                a = self.value(e[2], env)
                return wrap(-a[1], -a[0])
            t = self.truth(e[2], env)
            return (0, 1) if t is None else (int(not t),) * 2
        op = e[1]
        if op in ("&&", "||", "==", "!=", "<", "<=", ">", ">="):
            t = self.truth(e, env)
            return (0, 1) if t is None else (int(t),) * 2
        return arith(op, self.value(e[2], env), self.value(e[3], env))

    def truth(self, e, env):
        """Three-valued truth of e, evaluated left to right as C does."""
        if e[0] == "bin" and e[1] in ("&&", "||"):
            a = self.truth(e[2], env)
            if a is (e[1] == "||"):
                return a
            b = self.truth(e[3], env)
            if a is None:
                return None if b is not (e[1] == "||") else b
            return b
        if e[0] == "un" and e[1] == "!":
            t = self.truth(e[2], env)
            return None if t is None else not t
        if e[0] == "bin" and e[1] in ("==", "!=", "<", "<=", ">", ">="):
            return compare(e[1], self.value(e[2], env), self.value(e[3], env))
        v = self.value(e, env)
        return False if v == (0, 0) else True if not v[0] <= 0 <= v[1] else None

    def outcomes(self, body, env):
        """All outcomes [(env, output)] of a rule body run from env."""
        return self.run_body(body, dict(env), None)

    def run_body(self, stmts, env, out):
        """Like outcomes, but updates env in place (the caller owns it)."""
        results = [(env, out)]
        for s in stmts:
            if s[0] == "set":
                e = s[2]
                for env, _ in results:
                    env[s[1]] = e[1] if e[0] == "ptr" else self.value(e, env)
            elif s[0] == "if":
                nxt = []
                for env, out in results:
                    t = self.truth(s[1], env)
                    if t is None:
                        nxt += self.run_body(s[2], dict(env), out)
                        nxt += self.run_body(s[3], env, out)
                    else:
                        nxt += self.run_body(s[2] if t else s[3], env, out)
                results = nxt
            elif s[0] == "out":
                results = [(env, s[1]) for env, _ in results]
        return results

    def initial(self):
        return tuple(self.cell_of(v, self.init[v]) if v in self.abstract else self.init[v]
                     for v in self.names)

    def env_of(self, state, box=None):
        """The environment of an abstract state. An abstract global takes
        its cell, or its interval in box (one per global of self.absnames)
        when given."""
        env = {}
        for v, x in zip(self.names, state):
            if v in self.abstract:
                env[v] = self.cells[v][x]
            elif isinstance(x, str):
                env[v] = x
            else:
                env[v] = (x, x)
        if box is not None:
            for v, iv in zip(self.absnames, box):
                env[v] = iv
        return env

    def states_of(self, env):
        """The abstract states [(state, box)] an environment after a step
        may lie in: an abstract global's interval is split over the cells
        it meets."""
        opts = []
        multi = False
        for v in self.names:
            x = env[v]
            if v in self.abstract:
                o = [(k, (max(x[0], lo), min(x[1], hi)))
                     for k, (lo, hi) in enumerate(self.cells[v]) if lo <= x[1] and x[0] <= hi]
            elif isinstance(x, str):
                o = ((x, None),)
            elif x[0] == x[1]:
                o = ((x[0], None),)
            elif x[1] - x[0] < 256:
                o = [(y, None) for y in range(x[0], x[1] + 1)]
            else:
                raise Unsupported(f"exact global {v} took the interval {x}")
            multi = multi or len(o) > 1
            opts.append(o)
        slots = self.absslots
        if not multi:
            return [(tuple(o[0][0] for o in opts), tuple(opts[k][0][1] for k in slots))]
        return [(tuple(c[0] for c in combo), tuple(combo[k][1] for k in slots))
                for combo in itertools.product(*opts)]

    def step(self, state, inp, box=None, env=None):
        """Successors {(output, state): box} on input inp, from the abstract
        state `state` whose abstract globals lie in box (their cells when
        None); empty if the input is rejected. env, when given, is
        env_of(state, box)."""
        if env is None:
            env = self.env_of(state, box)
        succ = {}
        rules = self.rules[inp]
        for k in self.fast[inp](state):
            guard, body = rules[k]
            t = True
            if k not in self.compiled[inp]:
                for c in guard:
                    u = self.truth(c, env)
                    if u is False:
                        t = False
                        break
                    if u is None:
                        t = None
                if t is False:
                    continue
            for env2, out in self.outcomes(body, env):
                for s, b in self.states_of(env2):
                    old = succ.get((out, s))
                    succ[(out, s)] = b if old is None else join(old, b)
            if t is True:
                break
        return succ


def join(a, b):
    """The interval hull of two boxes, globalwise."""
    return tuple(x if x == y else (min(x[0], y[0]), max(x[1], y[1])) for x, y in zip(a, b))


class Codec:
    """Packs an abstract state into bytes, one byte per global that some
    rule assigns; the others keep their initial value."""

    def __init__(self, m):
        assigned = set()

        def walk(stmts):
            for s in stmts:
                if s[0] == "set":
                    assigned.add(s[1])
                elif s[0] == "if":
                    walk(s[2])
                    walk(s[3])

        for rs in m.rules.values():
            for _, body in rs:
                walk(body)
        self.base = list(m.initial())
        self.pos = [k for k, v in enumerate(m.names) if v in assigned]
        self.codes = [{} for _ in self.pos]
        self.vals = [[] for _ in self.pos]

    def encode(self, state):
        out = bytearray(len(self.pos))
        for i, p in enumerate(self.pos):
            x = state[p]
            c = self.codes[i].get(x)
            if c is None:
                c = self.codes[i][x] = len(self.vals[i])
                if c > 255:
                    raise Unsupported("a global took more than 256 abstract values")
                self.vals[i].append(x)
            out[i] = c
        return bytes(out)

    def decode(self, packed):
        state = self.base[:]
        for i, p in enumerate(self.pos):
            state[p] = self.vals[i][packed[i]]
        return tuple(state)


def widen(old, new, cells):
    """Widen a growing interval to the next power of two (or the cell
    bound), so that an interval can grow only finitely often."""
    lo, hi = new
    if lo < old[0]:
        lo = max(cells[0], -(1 << max(0, (-lo).bit_length())) if lo < 0 else 0)
    if hi > old[1]:
        hi = min(cells[1], (1 << hi.bit_length()) - 1 if hi > 0 else 0)
    return (lo, hi)


class Graph:
    """The abstract graph: states keyed by their packed cells and exact
    globals, each with a box (an interval per abstract global, inside its
    cell) that holds every program state the abstract state stands for."""

    def __init__(self, m):
        self.m = m
        self.codec = Codec(m)
        self.index = {}
        self.packed = []
        self.flat = []  # each box as array('i') of lo, hi pairs, to save memory
        self.growth = []

    def box(self, k):
        f = self.flat[k]
        return tuple(zip(f[0::2], f[1::2]))

    def add(self, state, box):
        """Add box to state's box. Return (id, changed)."""
        key = self.codec.encode(state)
        k = self.index.get(key)
        if k is None:
            k = self.index[key] = len(self.packed)
            self.packed.append(key)
            self.flat.append(array.array("i", [x for iv in box for x in iv]))
            self.growth.append(0)
            return k, True
        old = self.box(k)
        new = join(old, box)
        if new == old:
            return k, False
        self.growth[k] += 1
        if self.growth[k] > WIDEN_AFTER:
            new = tuple(widen(o, n, self.m.cells[v][c])
                        for v, o, n, c in zip(self.m.absnames, old, new,
                                              (state[i] for i in self.m.absslots)))
        self.flat[k] = array.array("i", [x for iv in new for x in iv])
        return k, True

    def state(self, k):
        return self.codec.decode(self.packed[k])


WIDEN_AFTER = 8


def explore(m, max_states):
    """Find every abstract state and its final box by a worklist fixed
    point: a state is expanded again whenever its box grows. Returns
    (graph, complete, expansions)."""
    g = Graph(m)
    init = m.initial()
    g.add(init, tuple((m.init[v], m.init[v]) for v in m.absnames))
    queue = collections.deque([0])
    queued = bytearray([1])
    expansions = 0
    while queue:
        if len(g.packed) > max_states:
            return g, False, expansions
        k = queue.popleft()
        queued[k] = 0
        s = g.state(k)
        env = m.env_of(s, g.box(k))
        for inp in m.inputs:
            for (_, t), box in m.step(s, inp, g.box(k), env).items():
                dst, changed = g.add(t, box)
                if dst == len(queued):
                    queued.append(0)
                if changed and not queued[dst]:
                    queued[dst] = 1
                    queue.append(dst)
        expansions += 1
        if expansions % 100_000 == 0:
            print(f"expanded {expansions}, {len(g.packed)} states, {len(queue)} queued",
                  file=sys.stderr, flush=True)
    return g, True, expansions


def write_edges(g, edge_path):
    """Write one line `SRC INPUT OUTPUT DST` per edge of the fixed point,
    in SRC order, and check that each successor box lies inside its
    target's box. Returns the number of edges."""
    m = g.m
    nedges = 0
    with open(edge_path, "w") as fh:
        for k in range(len(g.packed)):
            s = g.state(k)
            env = m.env_of(s, g.box(k))
            lines = set()
            for inp in m.inputs:
                for (o, t), box in m.step(s, inp, g.box(k), env).items():
                    dst = g.index[g.codec.encode(t)]
                    assert join(g.box(dst), box) == g.box(dst), "not a fixed point"
                    lines.add(f"{k} {inp} {o} {dst}\n")
            fh.writelines(sorted(lines))
            nedges += len(lines)
    return nedges


def write_hoa(path, atoms, nstates, edge_path):
    """The abstract graph as a HOA automaton over one-event letters: state
    k is abstract state k, and each (state, input) pair gets a middle
    state from which the outputs leave."""
    labels = {}
    n = len(atoms)
    for k, a in enumerate(atoms):
        labels[a] = "&".join(str(j) if j == k else f"!{j}" for j in range(n))
    body = Path(f"{path}.body")
    mid = nstates

    def flush(src, by_input, out):
        nonlocal mid
        first = []
        for inp in sorted(by_input):
            first.append(f"[{labels[rers_seq.event('i', inp)]}] {mid}\n")
            out.write(f"State: {mid}\n")
            for o, d in sorted(by_input[inp]):
                out.write(f"[{labels[rers_seq.event('o', o)]}] {d}\n")
            mid += 1
        out.write(f"State: {src}\n")
        out.writelines(first)

    with open(edge_path) as fh, open(body, "w") as out:
        cur, by_input = 0, {}
        for line in fh:
            src, inp, o, dst = map(int, line.split())
            while src != cur:
                flush(cur, by_input, out)
                cur, by_input = cur + 1, {}
            by_input.setdefault(inp, set()).add((o, dst))
        while cur < nstates:
            flush(cur, by_input, out)
            cur, by_input = cur + 1, {}
    with open(path, "w") as fh:
        fh.write(f"HOA: v1\nStates: {mid}\nStart: 0\n")
        fh.write(f"AP: {n} " + " ".join(f'"{a}"' for a in atoms) + "\n")
        fh.write("Acceptance: 0 t\n--BODY--\n")
        with open(body) as b:
            for line in b:
                fh.write(line)
        fh.write("--END--\n")
    body.unlink()


def witness(model, neg):
    """A lasso (u, v) of events, u v^omega, accepted by both automata."""
    r = subprocess.run(["autfilt", f"--product={neg}", str(model), "--stats=%w"],
                       capture_output=True, text=True, timeout=7200, check=True)
    word = r.stdout.strip()
    stem, _, cycle = word.partition("cycle{")

    def events(text):
        out = []
        for letter in text.rstrip("}").split(";"):
            pos = [a.strip() for a in letter.split("&") if a.strip() and "!" not in a]
            if pos:
                assert len(pos) == 1, letter
                out.append(pos[0])
        return out

    return events(stem), events(cycle)


def check(year, prob, args):
    t0 = time.monotonic()
    out = Path(args.out_dir) / f"abs-{year}-{prob}.tsv"
    atoms, props = rers_seq.load_properties(year, prob)
    sols = rers_seq.load_solutions(year, prob)
    wd = Path(tempfile.mkdtemp(prefix=f"rers-abs-{year}-{prob}-", dir=args.tmp))
    edge_path = wd / "edges.txt"
    try:
        m = Machine(year, prob)
        g, complete, expansions = explore(m, args.max_states)
        nstates = len(g.packed)
        nedges = write_edges(g, edge_path) if complete else 0
    except (smv.Refuse, Unsupported) as e:
        out.write_text(f"# year={year} problem={prob} refused: {e}\n")
        return f"{year}/{prob}: refused: {e}"
    t_explore = time.monotonic() - t0
    head = (f"# year={year} problem={prob} abstract_globals={len(m.abstract)} "
            f"cells={sum(map(len, m.cells.values()))} states={nstates} "
            f"expansions={expansions} edges={nedges} "
            f"complete={complete} oob_reads={m.oob} explore_s={t_explore:.1f}")
    if not complete:
        out.write_text(head + "\n")
        return f"{year}/{prob}: stopped at {nstates} abstract states"
    model = wd / "abs.hoa"
    write_hoa(model, atoms, nstates, edge_path)
    edge_path.unlink()
    jobs = []
    for (n, _, f), (_, official, _) in zip(props, sols):
        fixed = rers_seq.corrected(f)
        jobs.append((n, "phi", "holds" if official else "fails", rers_seq.to_spot(f)))
        if fixed:
            fix = rers_seq.to_spot(fixed)
            jobs += [(n, "negphi", "", f"!({rers_seq.to_spot(f)})"),
                     (n, "fix", "", fix), (n, "negfix", "", f"!({fix})")]
    rows = []
    exe = None
    for k, (n, kind, official, f) in enumerate(jobs):
        neg = rers_seq.translate(f, wd / f"q{k}.hoa")
        verdict = "unknown" if rers_seq.violated(model, neg) else "holds"
        if verdict == "unknown" and kind in ("fix", "negfix"):
            # A violating run of the abstract graph that replays as a run
            # of the compiled program shows that the formula fails.
            if exe is None:
                exe = rers_seq.compile_problem(year, prob, wd, "-O1")
            u, v = witness(model, neg)
            if any(rers_seq.replay_lasso(exe, u, v, rounds=r) for r in (60, 600, 3000)) \
                    and rers_seq_walk.spot_accepts(atoms, neg, u, v):
                verdict = "fails"
        neg.unlink()
        rows.append((n, kind, official, verdict))
    bad = sum(1 for _, k, o, v in rows if k == "phi" and o == "fails" and v == "holds")
    proved = sum(1 for _, k, o, v in rows if k == "phi" and o == "holds" and v == "holds")
    trues = sum(1 for _, k, o, _ in rows if k == "phi" and o == "holds")
    with open(out, "w") as fh:
        fh.write(f"{head} official_true_proved={proved}/{trues} official_false_proved={bad} "
                 f"total_s={time.monotonic() - t0:.1f}\n")
        fh.write("prop\tformula\tofficial\tverdict\n")
        for r in rows:
            fh.write("\t".join(map(str, r)) + "\n")
    for p in wd.iterdir():
        p.unlink()
    wd.rmdir()
    return (f"{year}/{prob}: {nstates} abstract states, {proved}/{trues} official trues "
            f"proved, {bad} official falses proved, oob {m.oob}")


def crosscheck(year, prob, args):
    """Check the abstraction against the compiled program.

    The explicit explorer of bench/rers_seq.py dumps the bytes of the first
    --cross-states program states and their edges. Each state is mapped to
    its abstract state, through the globals' addresses from `nm`. That
    abstract state must be in the fixed point with a box holding the
    state's values, and every accepted input must lead to an abstract
    successor with the same output and the abstraction of the concrete
    target.
    """
    m = Machine(year, prob)
    g, complete, _ = explore(m, args.max_states)
    if not complete:
        return f"{year}/{prob}: abstract search stopped at {len(g.packed)} states"
    wd = Path(tempfile.mkdtemp(prefix=f"rers-cross-{year}-{prob}-", dir=args.tmp))
    exe = rers_seq.compile_problem(year, prob, wd, "-O1")
    syms = {}
    for line in rers_seq.run(["nm", str(exe)], 60).stdout.splitlines():
        parts = line.split()
        if len(parts) == 3:
            syms[parts[2]] = int(parts[0], 16)
    base = syms["__start_rers_state"]
    targets = {syms[a]: f"t_{a}" for a in m.arrays}  # pointer values are addresses

    def abstract(raw):
        state, values = [], {}
        for v in m.names:
            off = syms[v] - base
            if str(m.init[v]).startswith("t_"):
                state.append(targets[int.from_bytes(raw[off:off + 8], "little")])
                continue
            x = int.from_bytes(raw[off:off + 4], "little", signed=True)
            values[v] = x
            state.append(m.cell_of(v, x) if v in m.abstract else x)
        return tuple(state), tuple(values[v] for v in m.absnames)

    r = subprocess.run([str(exe), str(args.cross_states), "dump"], capture_output=True,
                       text=True, check=False)
    absmap, edges = {}, []
    for line in r.stdout.splitlines():
        if line.startswith("S "):
            _, k, hexs = line.split()
            absmap[int(k)] = abstract(bytes.fromhex(hexs))
        elif line[0].isdigit():
            nums = [int(x) for x in line.split()]
            edges.append(nums)
    for p in wd.iterdir():
        p.unlink()
    wd.rmdir()
    outside = 0
    for state, values in absmap.values():
        k = g.index.get(g.codec.encode(state))
        if k is None or any(not lo <= x <= hi for x, (lo, hi) in zip(values, g.box(k))):
            outside += 1
    checked = bad = 0
    cache = {}
    for src, inp, dst, *outs in edges:
        if not outs or dst not in absmap:
            continue
        state = absmap[src][0]
        key = (state, inp)
        if key not in cache:
            k = g.index.get(g.codec.encode(state))
            cache[key] = set() if k is None else m.step(state, inp, g.box(k))
        checked += 1
        if len(outs) != 1 or (outs[0], absmap[dst][0]) not in cache[key]:
            bad += 1
    hit = len({a[0] for a in absmap.values()})
    return (f"{year}/{prob}: {len(absmap)} program states, {outside} outside their box; "
            f"{checked} edges checked, {bad} outside the abstraction; "
            f"{hit} of {len(g.packed)} abstract states hit")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("command", choices=["check", "crosscheck"])
    ap.add_argument("--years", type=int, nargs="*", default=[2016, 2017])
    ap.add_argument("--problems", type=int, nargs="*", default=list(range(1, 10)))
    ap.add_argument("--max-states", type=int, default=50_000_000)
    ap.add_argument("--cross-states", type=int, default=100_000,
                    help="program states that crosscheck compares")
    ap.add_argument("--out-dir", default=".")
    ap.add_argument("--tmp", default=None)
    args = ap.parse_args()
    sys.setrecursionlimit(100000)
    for year in args.years:
        for prob in args.problems:
            task = check if args.command == "check" else crosscheck
            print(task(year, prob, args), flush=True)
