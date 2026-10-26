#!/usr/bin/env python3
"""Decide whether one event of a RERS sequential program can occur, by a
backward search over boxes.

A box is a conjunction of constraints `lo <= x <= hi`, where x is a global
or an expression over globals. The rule table comes from the parser of
bench/rers_seq_smv.py, and each rule body is split into its paths through
the nested if/else. The search starts from the guards of the paths that
take the event, and computes pre-images: the pre-image of box B under a
path is the path's guard and branch conditions conjoined with B, each
constraint of B rewritten through the path's assignments (substitution).
A rewritten constraint that is a constant is decided. One of the form
`u + c` or `c - u` (in 32-bit wrapping arithmetic, as C with -fwrapv)
becomes an interval on u. Any other is kept as an expression constraint,
after globals that the box pins to one value are replaced by that value,
`x / x` by 1 and `x - x` by 0 (a run with x = 0 traps at x / x); it is
dropped when it grows past EXPR_LIMIT nodes. A box is empty, and
discarded, when a constraint fails on interval evaluation, or on every
value of the globals it reads when they have few; when a global's
interval misses its invariant interval (rers_seq_abs.Machine._ranges) or
its value set (a guard-aware fixed point over all rule paths); or when a
guard or branch condition is false on the box. Guard conditions that are
not a comparison with a constant are only checked this way, and the rule
that only the first enabled rule fires is ignored. Each step therefore
keeps a superset of the true pre-image, and a box covered by an earlier
box is not kept. If the search runs out of boxes and no box holds the
initial state, no run of the program takes the event. Out-of-bounds
constant array reads met on the way are dropped and counted; a proof
counts only when that count is 0.

When a box holds the initial state, its chain of paths gives an input
word, which is run on the rule table with exact values: the event is then
`reachable` (the word is a witness) or the chain is `spurious`.

  rers_seq_back.py check 2017 5 iN [--max-boxes N] [--out FILE.tsv]
  rers_seq_back.py check 2017 9 oW --after iO --unless iC,iH
                          (only runs that take oW after iO, no iC or iH between)
  rers_seq_back.py validate 2017 5            (every atom of the problem)
  rers_seq_back.py validate-rules 2017 5      (single rules random runs fire)

`validate` and `validate-rules` check the search against random runs: an
event or rule that a run takes must never come out `unreachable`.
"""

import argparse
import collections
import itertools
import os
import sys
import time
from pathlib import Path

import rers_seq
import rers_seq_abs as A
import rers_seq_smv as smv

DEBUG = bool(os.environ.get("RERS_BACK_DEBUG"))
CMP = {"<", "<=", ">", ">=", "==", "!="}
NEG = {"<": ">=", "<=": ">", ">": "<=", ">=": "<", "==": "!=", "!=": "=="}
FLIP = {"<": ">", "<=": ">=", ">": "<", ">=": "<=", "==": "==", "!=": "!="}


def atom(e, neg=False):
    """(v, op, c) when e is a comparison of a global with a constant, else None."""
    if e[0] == "un" and e[1] == "!":
        return atom(e[2], not neg)
    if e[0] != "bin" or e[1] not in CMP:
        return None
    a, op, b = e[2], e[1], e[3]
    if a[0] == "num" and b[0] == "var":
        a, b, op = b, a, FLIP[op]
    if a[0] != "var" or b[0] != "num":
        return None
    return (a[1], NEG[op] if neg else op, b[1])


def flat(e, neg=False):
    """The conjuncts of e (or of !e) as atoms; others are dropped."""
    if e[0] == "un" and e[1] == "!":
        return flat(e[2], not neg)
    if e[0] == "bin" and e[1] == ("||" if neg else "&&"):
        return flat(e[2], neg) + flat(e[3], neg)
    a = atom(e, neg)
    return [a] if a is not None else []


def dnf(e, neg=False, cap=64):
    """e (or !e) as a disjunction of conjunctions of atoms, as a list of
    lists; a leaf that is not an atom is left out (weakening it to true).
    None when the disjunction would pass `cap` terms."""
    if e[0] == "un" and e[1] == "!":
        return dnf(e[2], not neg, cap)
    if e[0] == "bin" and e[1] in ("&&", "||"):
        a, b = dnf(e[2], neg, cap), dnf(e[3], neg, cap)
        if a is None or b is None:
            return None
        if (e[1] == "&&") != neg:
            if len(a) * len(b) > cap:
                return None
            return [x + y for x in a for y in b]
        return a + b if len(a) + len(b) <= cap else None
    x = atom(e, neg)
    return [[x]] if x is not None else [[]]


def pure(e):
    """Whether e is built from atoms with && and || only (so that dnf or
    flat captures it exactly, when dnf stays under its cap)."""
    if e[0] == "un" and e[1] == "!":
        return pure(e[2])
    if e[0] == "bin" and e[1] in ("&&", "||"):
        return pure(e[2]) and pure(e[3]) and dnf(e) is not None and dnf(e, True) is not None
    return atom(e) is not None


def meet(iv, op, c):
    lo, hi = iv
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
    elif op == "!=":
        if lo == c:
            lo += 1
        if hi == c:
            hi -= 1
    return (lo, hi)


def linear(e):
    """e as (coefficients, constant) when it is affine, else None. The
    caller checks that no intermediate value can overflow."""
    k = e[0]
    if k == "num":
        return {}, e[1]
    if k == "var":
        return {e[1]: 1}, 0
    if k == "un" and e[1] == "-":
        f = linear(e[2])
        return None if f is None else ({u: -a for u, a in f[0].items()}, -f[1])
    if k == "bin" and e[1] in ("+", "-"):
        f, g = linear(e[2]), linear(e[3])
        if f is None or g is None:
            return None
        s = 1 if e[1] == "+" else -1
        coef = dict(f[0])
        for u, a in g[0].items():
            coef[u] = coef.get(u, 0) + s * a
        return {u: a for u, a in coef.items() if a}, f[1] + s * g[1]
    if k == "bin" and e[1] == "*":
        f, g = linear(e[2]), linear(e[3])
        if f is None or g is None:
            return None
        if not f[0]:
            f, g = g, f
        if g[0]:
            return None
        return {u: a * g[1] for u, a in f[0].items()}, f[1] * g[1]
    return None


EXPR_LIMIT = int(os.environ.get("RERS_BACK_EXPR", "200"))


def size(e):
    if e[0] in ("bin",):
        return 1 + size(e[2]) + size(e[3])
    if e[0] in ("un", "idx"):
        return 1 + size(e[2])
    return 1


def simplify(e):
    """e with x / x replaced by 1 and x - x by 0, then folded. A run on
    which x is 0 traps at x / x, so the rewrite holds on every run that
    goes on."""
    if e[0] == "bin":
        a, b = simplify(e[2]), simplify(e[3])
        if a == b and e[1] in ("/", "-"):
            return ("num", 1 if e[1] == "/" else 0)
        e = ("bin", e[1], a, b)
    elif e[0] == "un":
        e = ("un", e[1], simplify(e[2]))
    return e


def subst(e, sigma):
    k = e[0]
    if k == "var":
        return sigma.get(e[1], e)
    if k in ("num", "ptr", "arr"):
        return e
    if k == "idx":
        # A pointer that the path has pointed at another array reads that
        # array.
        name = e[1]
        t = sigma.get(name)
        if t is not None:
            assert t[0] == "ptr", t
            name = t[1][2:]
        return ("idx", name, subst(e[2], sigma))
    if k == "un":
        return ("un", e[1], subst(e[2], sigma))
    return ("bin", e[1], subst(e[2], sigma), subst(e[3], sigma))


def paths(stmts, sigma, conds, out):
    """All paths of a rule body: [(conditions over the pre-state, sigma, output)]."""
    for i, s in enumerate(stmts):
        if s[0] == "set":
            sigma = dict(sigma)
            sigma[s[1]] = subst(s[2], sigma)
        elif s[0] == "if":
            c = subst(s[1], sigma)
            rest = stmts[i + 1:]
            return (paths(s[2] + rest, sigma, conds + [(c, False)], out)
                    + paths(s[3] + rest, sigma, conds + [(c, True)], out))
        elif s[0] == "out":
            out = s[1]
    return [(conds, sigma, out)]


class Back:
    def __init__(self, m):
        self.m = m
        self.inv = {v: m.ranges[v] for v in m.names if v not in m.targets}
        self.vals = {}
        self.oob = 0
        self.base = {v: frozenset(m.targets[v]) if v in m.targets else m.ranges[v]
                     for v in m.names}
        self.build_paths()
        # Value sets make the boxes of the paths smaller, and smaller boxes
        # give smaller value sets; two rounds are enough in practice.
        for _ in range(2):
            self.vals = self.value_sets()
            for v, xs in self.vals.items():
                lo, hi = m.ranges[v]
                self.inv[v] = (max(lo, xs[0]), min(hi, xs[-1]))
                self.base[v] = self.inv[v]
            self.build_paths()
        self.index_paths()

    def build_paths(self):
        """Every rule path: (input, rule, guard box, sigma, output, the
        globals it sets to constants, its conditions as (expr, truth)).
        A path whose guard is a disjunction is split, one per disjunct."""
        m = self.m
        self.paths = []
        for inp, rs in m.rules.items():
            for k, (guard, body) in enumerate(rs):
                for conds, sigma, out in paths(body, {}, [], None):
                    sigma = {v: smv.fold(simplify(e), {}, m.arrays)
                             for v, e in sigma.items()}
                    terms = [[]]
                    for c, neg in [(c, False) for c in guard] + conds:
                        d = dnf(c, neg)
                        if d is None or len(terms) * len(d) > 256:
                            d = [flat(c, neg)]
                        terms = [x + y for x in terms for y in d]
                    checks = [(c, True) for c in guard] + [(c, not neg) for c, neg in conds]
                    consts = {v: e[1] for v, e in sigma.items() if e[0] == "num"}
                    # Conditions that the atoms do not capture stay in the
                    # box as constraints on their truth value.
                    extra = {}
                    for c, want in checks:
                        if not pure(c):
                            extra[c] = (1, 1) if want else (0, 0)
                    for atoms in terms:
                        box = self.constrain({}, atoms)
                        if box is not None:
                            box.update(extra)
                        if box is not None and self.possible(box, checks):
                            self.paths.append((inp, k, box, sigma, out, consts, checks))

    def value_sets(self, cap=4096):
        """A finite set of values for each global that can take only few:
        the least fixed point of `the initial value, plus every value that a
        rule path can assign when the globals it reads range over their sets
        inside the path's guard box`. Every path stays in the fixed point,
        so it over-approximates every run. A global whose set would pass
        `cap` values, or that is assigned from a global without a set or
        from an array, gets no set."""
        m = self.m

        def has_idx(e):
            return e[0] == "idx" or (e[0] == "un" and has_idx(e[2])) or \
                (e[0] == "bin" and (has_idx(e[2]) or has_idx(e[3])))
        assigns = []
        for p in self.paths:
            for v, e in p[3].items():
                if v in self.inv and e[0] != "num":
                    assigns.append((v, e, p[2]))
        vals = {v: {m.init[v]} for v in self.inv}
        for p in self.paths:
            for v, c in p[5].items():
                if v in vals:
                    vals[v].add(c)
        general = set(m.targets)
        changed = True
        while changed:
            changed = False
            for v, e, gbox in assigns:
                if v in general:
                    continue
                used = sorted(smv.variables(e, set()))
                if e[0] == "ptr" or has_idx(e):
                    general.add(v)
                    changed = True
                    continue
                opts, size = [], 1
                for u in used:
                    lo, hi = gbox.get(u, (A.MIN, A.MAX))
                    if u in general:
                        opts.append(None)
                        size = None
                        continue
                    o = [x for x in vals[u] if lo <= x <= hi]
                    opts.append(o)
                    if size is not None:
                        size *= len(o)
                if any(o == [] for o in opts):
                    continue  # the path cannot run
                if size is not None and size <= 4096:
                    new = set()
                    for combo in itertools.product(*opts):
                        try:
                            new.add(m.value(e, {u: (c, c) for u, c in zip(used, combo)})[0])
                        except A.Unsupported:  # division by zero traps: no value
                            pass
                else:
                    # Interval evaluation over the hulls of the sets and the
                    # intervals of the globals without one.
                    env = {}
                    for u, o in zip(used, opts):
                        if o is None:
                            lo, hi = gbox.get(u, self.inv[u])
                            ilo, ihi = self.inv[u]
                            env[u] = (max(lo, ilo), min(hi, ihi))
                        else:
                            env[u] = (min(o), max(o))
                    try:
                        x = m.value(e, env)
                    except A.Unsupported:
                        continue
                    if x[1] - x[0] > cap:
                        if DEBUG:
                            print("general", v, "from", e, env, file=sys.stderr)
                        general.add(v)
                        changed = True
                        continue
                    new = set(range(x[0], x[1] + 1))
                if not new <= vals[v]:
                    vals[v] |= new
                    changed = True
                if len(vals[v]) > cap:
                    if DEBUG:
                        print("general", v, "cap", file=sys.stderr)
                    general.add(v)
                    changed = True
        return {v: sorted(x) for v, x in vals.items() if v not in general}

    def index_paths(self):
        """Bit masks over self.paths: for each global, the paths that leave
        it alone or set it to a non-constant, and for each constant, the
        paths that set it to that constant."""
        self.free = {}
        self.setto = {}
        everyone = (1 << len(self.paths)) - 1
        for v in self.inv:
            self.free[v] = everyone
            self.setto[v] = {}
        for i, p in enumerate(self.paths):
            for v, c in p[5].items():
                if v in self.inv:
                    self.free[v] &= ~(1 << i)
                    self.setto[v][c] = self.setto[v].get(c, 0) | (1 << i)

    def candidates(self, post):
        """The paths whose constant assignments agree with the box post."""
        mask = (1 << len(self.paths)) - 1
        for v, (lo, hi) in post.items():
            if isinstance(v, str):
                m = self.free[v]
                for c, bits in self.setto[v].items():
                    if lo <= c <= hi:
                        m |= bits
                mask &= m
        out = []
        while mask:
            low = mask & -mask
            out.append(self.paths[low.bit_length() - 1])
            mask ^= low
        return out

    def possible(self, box, checks):
        """False when some condition is certainly violated in box."""
        env = dict(self.base)
        env.update(box)
        return all(self.m.truth(c, env) is not (not want) for c, want in checks)

    def feasible(self, v, iv):
        lo, hi = iv
        if lo > hi:
            return False
        ilo, ihi = self.inv[v]
        if hi < ilo or lo > ihi:
            return False
        vs = self.vals.get(v)
        if vs is not None and not any(lo <= x <= hi for x in vs):
            return False
        return True

    def constrain(self, box, atoms):
        """box conjoined with the atoms, or None when empty."""
        box = dict(box)
        for v, op, c in atoms:
            if v not in self.inv:
                continue
            iv = meet(box.get(v, self.inv[v]), op, c)
            if not self.feasible(v, iv):
                return None
            box[v] = iv
        return box

    def pre(self, post, path):
        """The pre-image boxes of the box `post` under one rule path: a
        list whose union contains the true pre-image (empty when it is).

        A box maps a global to an interval, or an expression over globals
        (a tuple) to the interval its value must lie in. A constraint is
        rewritten through the path's assignments; it becomes a constraint
        on one global when the rewritten expression is that global plus a
        constant, is decided when it is a constant, and is kept as an
        expression otherwise, unless it has grown past EXPR_LIMIT nodes,
        when it is dropped."""
        _, _, gbox, sigma, _, consts, checks = path
        for v, iv in post.items():
            if isinstance(v, str):
                c = consts.get(v)
                if c is not None and not iv[0] <= c <= iv[1]:
                    return []
        box = dict(gbox)
        exprs = []
        for v, iv in post.items():
            if isinstance(v, str):
                e = sigma.get(v, ("var", v))
                if e[0] == "num":
                    continue
            else:
                try:
                    e = smv.fold(simplify(subst(v, sigma)), {}, self.m.arrays)
                except smv.Refuse as err:
                    if "division by zero" in str(err):
                        return []  # the run traps
                    # A constant array read out of bounds: C leaves it
                    # undefined. The constraint is dropped and counted.
                    self.oob += 1
                    continue
                if e[0] == "num":
                    if not iv[0] <= e[1] <= iv[1]:
                        return []
                    continue
            u, iv2 = self.solve(e, iv)
            if u is not None:
                cur = box.get(u, self.inv[u])
                new = (max(cur[0], iv2[0]), min(cur[1], iv2[1]))
                if not self.feasible(u, new):
                    return []
                box[u] = new
            else:
                exprs.append((e, iv))
        # Globals the box pins to one value become constants in the
        # expression constraints, which often fold them away.
        pins = {u: ("num", x[0]) for u, x in box.items()
                if isinstance(u, str) and x[0] == x[1]}
        env = dict(self.base)
        env.update(box)
        for e, iv in exprs:
            try:
                e = smv.fold(simplify(subst(e, pins)), {}, self.m.arrays)
            except smv.Refuse as err:
                if "division by zero" in str(err):
                    return []
                self.oob += 1
                continue
            if e[0] == "num":
                if not iv[0] <= e[1] <= iv[1]:
                    return []
                continue
            u, iv2 = self.solve(e, iv)
            if u is not None:
                cur = box.get(u, self.inv[u])
                new = (max(cur[0], iv2[0]), min(cur[1], iv2[1]))
                if not self.feasible(u, new):
                    return []
                box[u] = env[u] = new
                continue
            if size(e) > EXPR_LIMIT:
                continue
            try:
                x = self.m.value(e, env)
            except A.Unsupported:
                return []  # a certain division by zero: the run traps
            if x[1] < iv[0] or x[0] > iv[1]:
                return []
            if iv[0] <= x[0] and x[1] <= iv[1]:
                continue  # implied by the box
            verdict = self.enumerate(e, iv, box)
            if verdict is False:
                return []
            if verdict is None:
                old = box.get(e)
                box[e] = iv if old is None else (max(old[0], iv[0]), min(old[1], iv[1]))
                if box[e][0] > box[e][1]:
                    return []
        return [box] if self.possible(box, checks) else []

    def enumerate(self, e, iv, box):
        """Whether `e in iv` can hold in box, by trying every value of the
        globals e reads, when they have few: True (always), False (never)
        or None (sometimes, or too many values to try)."""
        used = sorted(smv.variables(e, set()))
        opts, n = [], 1
        for u in used:
            lo, hi = box.get(u, self.inv[u])
            if u in self.vals:
                o = [x for x in self.vals[u] if lo <= x <= hi]
            elif hi - lo < 32:
                o = range(lo, hi + 1)
            else:
                return None
            opts.append(o)
            n *= len(o)
            if n > 4096:
                return None
        seen = set()
        for combo in itertools.product(*opts):
            try:
                env = dict(self.base)
                env.update({u: (c, c) for u, c in zip(used, combo)})
                x = self.m.value(e, env)
                if x[0] != x[1]:  # an array read through a pointer: inexact
                    return None
                x = x[0]
            except A.Unsupported:
                continue  # division by zero: the run traps
            seen.add(iv[0] <= x <= iv[1])
            if len(seen) == 2:
                return None
        return seen.pop() if seen else False

    def solve(self, e, iv):
        """(u, interval) when `e in iv` is exactly `u in interval` for one
        global u (e is u plus a constant, or minus u plus a constant, in
        32-bit wrapping arithmetic, with no wrap inside the interval);
        else (None, None)."""
        f = linear(e)
        if f is None or len(f[0]) != 1:
            return None, None
        (u, a), = f[0].items()
        if u not in self.inv or a not in (1, -1):
            return None, None
        c = f[1]
        lo, hi = (iv[0] - c, iv[1] - c) if a == 1 else (c - iv[1], c - iv[0])
        if A.MIN < lo and hi <= A.MAX:
            return u, (lo, hi)
        return None, None

    def covers(self, a, b):
        """Box a contains box b: every constraint of a is implied by the
        same constraint of b (or, for a global, by its invariant)."""
        for v, (lo, hi) in a.items():
            if v in b:
                blo, bhi = b[v]
            elif isinstance(v, str):
                blo, bhi = self.inv[v]
            else:
                return False
            if blo < lo or bhi > hi:
                return False
        return True

    def holds_init(self, box, path):
        """Whether the initial state lies in box and meets the guard and
        branch conditions of the path that box is a pre-image under."""
        env = {v: (x, x) for v, x in self.m.init.items() if v not in self.m.targets}
        for v in self.m.targets:
            env[v] = self.m.init[v]
        if not all(self.m.truth(c, env) is want for c, want in path[6]):
            return False
        for v, (lo, hi) in box.items():
            x = env[v][0] if isinstance(v, str) else self.m.value(v, env)[0]
            if not lo <= x <= hi:
                return False
        return True


def targets(m, event):
    """The test for the rule paths that take the event: `iX` (input X is
    accepted), `oX` (output X is printed) or `rX:K` (rule K of input X
    fires, for validation)."""
    want = ord(event[1]) - 64
    if event[0] == "r":
        k = int(event[3:])
        return lambda path: path[0] == want and path[1] == k
    return lambda path: (path[0] == want) if event[0] == "i" else (path[4] == want)


def search(b, event, max_boxes, log_every=10000, scope=None):
    """Backward search. Returns (hit chain or None, boxes, complete).

    With scope = (opener, closers), input values, the search looks for a
    run that takes the event after an opener input with no closer input
    in between (the opener's own step counts). A box is then tagged 0 while
    the opener is still to come (only paths of other inputs than the
    closers lead to it) and 1 once it is behind, and only a box tagged 1
    can hold the initial state."""
    is_target = targets(b.m, event)
    opener, closers = scope if scope else (None, ())
    boxes = []  # (box, parent index or None, path, tag)
    queue = collections.deque()
    # Boxes indexed by tag and set of constrained globals, for subsumption.
    by_keys = collections.defaultdict(list)

    def add(box, parent, path, tag, force=False):
        keys = frozenset(box)
        for (t, ks), lst in by_keys.items() if not force else ():
            if t == tag and ks <= keys:
                for j in lst:
                    if b.covers(boxes[j][0], box):
                        return None
        boxes.append((box, parent, path, tag))
        by_keys[(tag, keys)].append(len(boxes) - 1)
        queue.append(len(boxes) - 1)
        return len(boxes) - 1

    def tag_after(path, tag):
        """The tag of a pre-image under path of a box tagged tag, or None
        when the path may not be taken there."""
        if scope is None or tag == 1:
            return 1
        if path[0] in closers:
            return None
        return 1 if path[0] == opener else 0

    # A box holds the initial state only when its path's conditions hold
    # there too, so every box is tested before the covering test can drop it.
    for p in b.paths:
        if is_target(p):
            tag = tag_after(p, 0)
            if tag is None:
                continue
            if tag == 1 and b.holds_init(p[2], p):
                return chain(boxes, add(p[2], None, p, tag, True)), len(boxes), False
            add(p[2], None, p, tag)
    t0 = time.monotonic()
    expanded = 0
    while queue:
        if len(boxes) > max_boxes:
            return None, len(boxes), False
        j = queue.popleft()
        post, tag0 = boxes[j][0], boxes[j][3]
        for p in b.candidates(post):
            tag = tag_after(p, tag0)
            if tag is None:
                continue
            for box in b.pre(post, p):
                if tag == 1 and b.holds_init(box, p):
                    return chain(boxes, add(box, j, p, tag, True)), len(boxes), False
                add(box, j, p, tag)
        expanded += 1
        if expanded % log_every == 1:
            print(f"  expanded {expanded - 1}, {len(boxes)} boxes, {len(queue)} queued, "
                  f"{time.monotonic() - t0:.0f} s", file=sys.stderr, flush=True)
    return None, len(boxes), True


def chain(boxes, j):
    """The rule paths from the initial state to the event, in run order,
    each with the box (last) that the state before it lies in."""
    out = []
    while j is not None:
        box, parent, path, _ = boxes[j]
        out.append(path + (box,))
        j = parent
    return out


def replay(m, word, event):
    """Run the input word on the rule table with exact values (points).
    Returns the index of the step that takes the event, or None."""
    env = {v: (frozenset([x]) if v in m.targets else (x, x)) for v, x in m.init.items()}
    for v in m.targets:
        env[v] = m.init[v]
    want = ord(event[1]) - 64
    for n, inp in enumerate(word):
        fired = None
        for k, (guard, body) in enumerate(m.rules[inp]):
            ts = [m.truth(c, env) for c in guard]
            assert None not in ts
            if all(ts):
                fired = k
                break
        if fired is None:
            return None
        (env, out), = m.outcomes(m.rules[inp][fired][1], env)
        if (event[0] == "i" and inp == want) or (event[0] == "o" and out == want) or \
                (event[0] == "r" and inp == want and fired == int(event[3:])):
            return n
    return None


def decide(m, b, event, max_boxes, scope=None):
    """The verdict on whether the event can occur, as a dict: verdict
    (unreachable, reachable, spurious or open), boxes, seconds, the
    out-of-bounds array reads met, and for a chain, its input word."""
    t0 = time.monotonic()
    b.oob, m.oob = 0, 0
    hit, n, complete = search(b, event, max_boxes, scope=scope)
    r = {"verdict": "unreachable" if complete else "open", "boxes": n,
         "seconds": round(time.monotonic() - t0), "oob_dropped": b.oob,
         "oob_interval": m.oob, "word": ""}
    if hit is not None:
        word = [p[0] for p in hit]
        r["word"] = "".join(chr(64 + x) for x in word)
        r["verdict"] = "reachable" if replay(m, word, event) is not None else "spurious"
    return r


def line(r):
    return " ".join(f"{k}={v}" for k, v in r.items() if v != "")


def walk_events(m, walks, steps, seed):
    """The events seen on random runs of the rule table (exact values)."""
    import random
    rng = random.Random(seed)
    seen = set()
    for _ in range(walks):
        env = {v: (x, x) for v, x in m.init.items() if v not in m.targets}
        for v in m.targets:
            env[v] = m.init[v]
        for _ in range(steps):
            inp = rng.choice(m.inputs)
            fired = None
            for k, (guard, body) in enumerate(m.rules[inp]):
                if all(m.truth(c, env) for c in guard):
                    fired = k
                    break
            if fired is None:
                break
            (env, out), = m.outcomes(m.rules[inp][fired][1], env)
            seen |= {f"i{chr(64 + inp)}", f"o{chr(64 + out)}"}
    return seen


def fired_rules(m, walks, steps, seed):
    """{(input, rule): first depth} over random runs that pick among the
    accepted inputs only, so that they do not end early."""
    import random
    rng = random.Random(seed)
    first = {}
    for _ in range(walks):
        env = {v: (x, x) for v, x in m.init.items() if v not in m.targets}
        for v in m.targets:
            env[v] = m.init[v]
        for depth in range(steps):
            options = []
            for inp in m.inputs:
                for k, (guard, body) in enumerate(m.rules[inp]):
                    if all(m.truth(c, env) for c in guard):
                        options.append((inp, k))
                        break
            if not options:
                break
            inp, k = rng.choice(options)
            first.setdefault((inp, k), depth)
            (env, _), = m.outcomes(m.rules[inp][k][1], env)
    return first


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check", help="decide whether one event can occur")
    c.add_argument("year", type=int)
    c.add_argument("prob", type=int)
    c.add_argument("event")
    c.add_argument("--max-boxes", type=int, default=1_000_000)
    c.add_argument("--out", help="append the verdict to this TSV")
    c.add_argument("--after", help="only runs that take the event after this input "
                   "atom, e.g. iO (the scope of a pattern)")
    c.add_argument("--unless", default="", help="with --after: input atoms that close "
                   "the scope, comma-separated, e.g. iC,iH")
    v = sub.add_parser("validate", help="decide every event of a problem and compare "
                       "with the events random runs see")
    v.add_argument("year", type=int)
    v.add_argument("prob", type=int)
    v.add_argument("--max-boxes", type=int, default=100_000)
    v.add_argument("--walks", type=int, default=300)
    v.add_argument("--steps", type=int, default=300)
    r = sub.add_parser("validate-rules", help="decide whether single rules can fire, "
                       "for rules that random runs fire, deepest first")
    r.add_argument("year", type=int)
    r.add_argument("prob", type=int)
    r.add_argument("--max-boxes", type=int, default=30_000)
    r.add_argument("--walks", type=int, default=50)
    r.add_argument("--steps", type=int, default=500)
    r.add_argument("--rules", type=int, default=100)
    args = ap.parse_args()
    sys.setrecursionlimit(100000)
    t0 = time.monotonic()
    m = A.Machine(args.year, args.prob)
    b = Back(m)
    print(f"{args.year}/{args.prob}: {len(b.paths)} rule paths, {len(b.vals)} globals "
          f"with value sets, built in {time.monotonic() - t0:.0f} s", flush=True)
    if args.cmd == "check":
        scope = None
        if args.after:
            scope = (ord(args.after[1]) - 64,
                     {ord(a[1]) - 64 for a in args.unless.split(",") if a})
        r = decide(m, b, args.event, args.max_boxes, scope)
        print(f"RESULT {args.event} {line(r)}")
        if args.out:
            cols = ["year", "problem", "event"] + list(r)
            new = not Path(args.out).exists()
            with open(args.out, "a") as fh:
                if new:
                    fh.write("\t".join(cols) + "\n")
                ev = args.event
                if scope:  # never to be read as the event's own reachability
                    ev += f" after {args.after} unless {args.unless}"
                vals = [args.year, args.prob, ev] + list(r.values())
                fh.write("\t".join(map(str, vals)) + "\n")
        return
    if args.cmd == "validate-rules":
        first = fired_rules(m, args.walks, args.steps, 1)
        print(f"{len(first)} rules fired on random runs", flush=True)
        bad = 0
        for (inp, k), depth in sorted(first.items(), key=lambda x: -x[1])[:args.rules]:
            ev = f"r{chr(64 + inp)}:{k}"
            r = decide(m, b, ev, args.max_boxes)
            wrong = r["verdict"] == "unreachable"
            bad += wrong
            print(f"{ev} depth {depth} {line(r)}{'  CONTRADICTION' if wrong else ''}",
                  flush=True)
        print(f"VALIDATE {args.year}/{args.prob}: {bad} contradictions")
        return
    atoms, _ = rers_seq.load_properties(args.year, args.prob)
    seen = walk_events(m, args.walks, args.steps, 1)
    bad = 0
    for ev in atoms:
        r = decide(m, b, ev, args.max_boxes)
        wrong = ev in seen and r["verdict"] == "unreachable"
        bad += wrong
        print(f"{ev} {'seen' if ev in seen else 'unseen'} {line(r)}"
              f"{'  CONTRADICTION' if wrong else ''}", flush=True)
    print(f"VALIDATE {args.year}/{args.prob}: {bad} contradictions")


if __name__ == "__main__":
    main()
