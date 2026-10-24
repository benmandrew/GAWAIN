#!/usr/bin/env python3
"""Translate a RERS 2016/2017 sequential problem from C to an SMV model.

The C programs are event-condition-action tables. `calculate_output(input)`
sets `cf = 1`, then walks nested `if` blocks whose conditions all contain
the conjunct `cf == 1`. A rule is an `if` whose body starts with `cf = 0`:
it updates globals (possibly under nested if/else) and prints one output.
Because nothing but a rule body writes a global, and a rule clears `cf`,
the first rule in program order whose enclosing conditions hold on the
pre-state is the one that fires, and no later condition can hold. The
translator checks every one of these facts and refuses the file when one
fails.

SMV model, one position per event (as in the RERS property files):

  ev    the event at this position: start, an input i<X>, an output o<X>,
        or dead (absorbing; reached on an input that no rule accepts);
  po    the output of the last input, shown at the next position;
  inp   an input variable (IVAR) choosing the next input.

Globals assigned only constants become integer enumerations; the others,
and every global that meets one of them in an expression, become
`signed word[32]`, whose `/`, `mod` and overflow match C on x86 with
-fwrapv. A property phi of the RERS file is checked as
`X (G !dead -> phi)`: from the first input, over the runs that never take
an invalid input, as RERS reads its properties (checked against the
official verdicts by bench/rers_seq.py).

Problems 3, 6 and 9 read constant arrays, directly or through pointer
globals that are only ever set to an array. A read becomes a case over the
index (and over the pointer's targets). C leaves an out-of-range read
undefined, so every read that may execute adds a disjunct to the DEFINE
`oob`, and `check` uses a model only when nuXmv proves `G !oob` and every
decided shipped property matches its official verdict.

  rers_seq_smv.py model [--years ...] [--problems ...] > model.smv
  rers_seq_smv.py check --out-dir DIR [--jobs N] [--timeout S] ...

`check` writes DIR/smv-YEAR-PROB.tsv. Each of the --jobs nuXmv sessions
builds the model once and checks its share of the properties with IC3
(`check_ltlspec_ic3`). Bit-level IC3 is slow on the 32-bit arithmetic,
and the models of problems 6 to 9 (13 MB to 118 MB) do not build within
8 GB, so in practice this route decides problems 1 and 2.
"""

import concurrent.futures as cf
import re
import subprocess
import sys
import time
from pathlib import Path

TOKEN = re.compile(r"""
    (?P<ws>\s+|//[^\n]*|/\*.*?\*/)
  | (?P<str>"(?:[^"\\]|\\.)*")
  | (?P<num>\d+)
  | (?P<id>[A-Za-z_]\w*)
  | (?P<op>==|!=|<=|>=|&&|\|\||[-+*/%<>=!(){}\[\];,&])
""", re.S | re.X)


class Refuse(Exception):
    """The file is outside the subset this translator handles."""


def tokenize(src):
    src = re.sub(r"^\s*#.*$", "", src, flags=re.M)
    out, pos = [], 0
    while pos < len(src):
        m = TOKEN.match(src, pos)
        if not m:
            raise Refuse(f"cannot tokenise at {src[pos:pos + 40]!r}")
        pos = m.end()
        if m.lastgroup != "ws":
            out.append(m.group(m.lastgroup))
    return out


class Parser:
    def __init__(self, toks):
        self.t, self.i = toks, 0
        self.arrays, self.pointers = {}, set()

    def peek(self, k=0):
        return self.t[self.i + k] if self.i + k < len(self.t) else None

    def take(self, want=None):
        tok = self.peek()
        if want is not None and tok != want:
            raise Refuse(f"expected {want!r}, got {tok!r} near token {self.i}")
        self.i += 1
        return tok

    # --- top level ---
    def program(self):
        globals_, funcs, inputs = {}, {}, None
        while self.peek() is not None:
            if self.peek() == "extern":
                while self.take() != ";":
                    pass
            elif self.peek() == "void" or (self.peek() == "int" and self.peek(1) == "main"):
                self.take()
                name = self.take()
                self.take("(")
                while self.take() != ")":
                    pass
                if self.peek() == ";":
                    self.take()
                elif name == "main":
                    self.skip_block()
                else:
                    funcs[name] = self.block()
            elif self.peek() == "int":
                self.take()
                if self.peek() == "*":
                    self.take()
                    name = self.take()
                    self.take("=")
                    target = self.take()
                    self.take(";")
                    if target not in self.arrays:
                        raise Refuse(f"pointer {name} to {target}")
                    self.pointers.add(name)
                    globals_[name] = f"t_{target}"
                    continue
                name = self.take()
                if self.peek() == "[":
                    self.take("[")
                    self.take("]")
                    self.take("=")
                    self.take("{")
                    vals = []
                    while self.peek() != "}":
                        vals.append(int(self.take()))
                        if self.peek() == ",":
                            self.take()
                    self.take("}")
                    self.take(";")
                    if name == "inputs":
                        inputs = vals
                    self.arrays[name] = vals
                else:
                    self.take("=")
                    sign = -1 if self.peek() == "-" and self.take() else 1
                    globals_[name] = sign * int(self.take())
                    self.take(";")
            else:
                raise Refuse(f"unexpected top-level token {self.peek()!r}")
        return globals_, funcs, inputs, self.arrays

    def skip_block(self):
        self.take("{")
        depth = 1
        while depth:
            tok = self.take()
            depth += {"{": 1, "}": -1}.get(tok, 0)

    # --- statements ---
    def block(self):
        self.take("{")
        stmts = []
        while self.peek() != "}":
            stmts.append(self.statement())
        self.take("}")
        return stmts

    def statement(self):
        tok = self.peek()
        if tok == "{":
            return ("seq", self.block())
        if tok == "if":
            self.take()
            self.take("(")
            cond = self.expr()
            self.take(")")
            then = self.block() if self.peek() == "{" else [self.statement()]
            other = []
            if self.peek() == "else":
                self.take()
                other = self.block() if self.peek() == "{" else [self.statement()]
            return ("if", cond, then, other)
        if tok in ("printf", "fprintf", "fflush"):
            self.take()
            self.take("(")
            args = []
            while self.peek() != ")":
                args.append(self.take())
            self.take(")")
            self.take(";")
            if tok == "printf":
                if len(args) != 3 or args[0] != '"%d\\n"' or not args[2].isdigit():
                    raise Refuse(f"printf {args}")
                return ("out", int(args[2]))
            if tok == "fprintf" and args[:2] != ["stderr", ","]:
                raise Refuse(f"fprintf {args}")
            return ("nop",)
        if self.peek(1) == "(":
            name = self.take()
            self.take("(")
            arg = None if self.peek() == ")" else self.take()
            self.take(")")
            self.take(";")
            if arg not in (None, "input"):
                raise Refuse(f"call {name}({arg})")
            return ("call", name)
        if self.peek(1) == "=":
            name = self.take()
            self.take("=")
            e = self.expr()
            self.take(";")
            if name in self.pointers:
                if e[0] != "arr":
                    raise Refuse(f"pointer {name} set to {e}")
                return ("set", name, ("ptr", f"t_{e[1]}"))
            if name in self.arrays:
                raise Refuse(f"array {name} assigned")
            return ("set", name, e)
        raise Refuse(f"unexpected statement at {tok!r}")

    # --- expressions, C precedence ---
    LEVELS = [("||",), ("&&",), ("==", "!="), ("<", "<=", ">", ">="), ("+", "-"),
              ("*", "/", "%")]

    def expr(self, level=0):
        if level == len(self.LEVELS):
            return self.unary()
        lhs = self.expr(level + 1)
        while self.peek() in self.LEVELS[level]:
            op = self.take()
            lhs = ("bin", op, lhs, self.expr(level + 1))
        return lhs

    def unary(self):
        tok = self.peek()
        if tok in ("-", "!"):
            self.take()
            return ("un", tok, self.unary())
        if tok == "(":
            self.take()
            e = self.expr()
            self.take(")")
            return e
        if tok == "*":
            self.take()
            return self.read(self.take(), ("num", 0))
        self.take()
        if tok.isdigit():
            return ("num", int(tok))
        if self.peek() == "[":
            self.take()
            e = self.expr()
            self.take("]")
            return self.read(tok, e)
        if tok in self.arrays:
            return ("arr", tok)  # only valid as the right side of `pointer = array`
        if tok in self.pointers:
            raise Refuse(f"pointer {tok} used as a value")
        return ("var", tok)

    def read(self, name, index):
        if name not in self.arrays and name not in self.pointers:
            raise Refuse(f"read of {name}[...]")
        return ("idx", name, index)


# --- Constant folding with the input fixed ---------------------------------

def fold(e, env, arrays):
    """Simplify e, where env maps names to constants (input and cf) and
    arrays maps the constant arrays to their contents."""
    kind = e[0]
    if kind in ("num", "ptr"):
        return e
    if kind == "arr":
        raise Refuse(f"array {e[1]} used as a value")
    if kind == "idx":
        i = fold(e[2], env, arrays)
        if i[0] == "num" and e[1] in arrays:
            if not 0 <= i[1] < len(arrays[e[1]]):
                raise Refuse(f"constant read {e[1]}[{i[1]}] out of bounds")
            return ("num", arrays[e[1]][i[1]])
        return ("idx", e[1], i)
    if kind == "var":
        return ("num", env[e[1]]) if e[1] in env else e
    if kind == "un":
        a = fold(e[2], env, arrays)
        if a[0] == "num":
            return ("num", -a[1] if e[1] == "-" else int(not a[1]))
        return ("un", e[1], a)
    op, a, b = e[1], fold(e[2], env, arrays), fold(e[3], env, arrays)
    if op == "&&":
        if a[0] == "num":
            return b if a[1] else ("num", 0)
        if b[0] == "num":
            return a if b[1] else ("num", 0)
    if op == "||":
        if a[0] == "num":
            return ("num", 1) if a[1] else b
        if b[0] == "num":
            return ("num", 1) if b[1] else a
    if a[0] == "num" and b[0] == "num":
        x, y = a[1], b[1]
        if op in ("/", "%"):
            if y == 0:
                raise Refuse("constant division by zero")
            q = abs(x) // abs(y) * (1 if (x >= 0) == (y >= 0) else -1)  # C truncates
            return ("num", q if op == "/" else x - y * q)
        val = {"==": x == y, "!=": x != y, "<": x < y, "<=": x <= y, ">": x > y,
               ">=": x >= y, "+": x + y, "-": x - y, "*": x * y}[op]
        return ("num", int(val))
    return ("bin", op, a, b)


def conjuncts(e):
    if e[0] == "bin" and e[1] == "&&":
        return conjuncts(e[2]) + conjuncts(e[3])
    return [e]


def has_cf_guard(e):
    return ("bin", "==", ("var", "cf"), ("num", 1)) in conjuncts(e) or \
        ("bin", "==", ("num", 1), ("var", "cf")) in conjuncts(e)


# --- Rules -----------------------------------------------------------------

def rules_for_input(funcs, value, arrays):
    """The rules that can fire on `value`, in program order.

    Each rule is (guard, body): guard is the list of folded enclosing
    conditions, body the statements after `cf = 0`.
    """
    rules = []

    def walk(stmts, guard, depth):
        for s in stmts:
            if s[0] == "if":
                if not has_cf_guard(s[1]):
                    raise Refuse("an if outside a rule body lacks the cf == 1 conjunct")
                c = fold(s[1], {"input": value, "cf": 1}, arrays)
                if s[3]:
                    raise Refuse("else outside a rule body")
                if c == ("num", 0):
                    continue
                body = s[2]
                if body and body[0] == ("set", "cf", ("num", 0)):
                    rules.append((guard + [c], fold_body(body[1:], value, arrays)))
                else:
                    walk(body, guard + [c], depth)
            elif s[0] == "call":
                if s[1] == "errorCheck":
                    if funcs["errorCheck"]:
                        raise Refuse("errorCheck is not empty")
                    continue
                if depth > 3:
                    raise Refuse("call depth")
                walk(funcs[s[1]], guard, depth + 1)
            elif s[0] == "nop":
                continue
            elif s[0] == "set" and s[1] == "cf" and s[2] == ("num", 1) and not guard and \
                    depth == 0:
                continue
            else:
                raise Refuse(f"statement {s[0]} outside a rule body")

    walk(funcs["calculate_output"], [], 0)
    return rules


def fold_body(stmts, value, arrays):
    env = {"input": value, "cf": 0}  # a rule body runs after `cf = 0`
    out = []
    for s in stmts:
        if s[0] == "set":
            out.append(("set", s[1], fold(s[2], env, arrays)))
        elif s[0] == "if":
            out.append(("if", fold(s[1], env, arrays), fold_body(s[2], value, arrays),
                        fold_body(s[3], value, arrays)))
        else:
            out.append(s)
    return out


def check_body(body):
    """A rule body: assignments and nested if/else, exactly one output on
    every path, and the output last."""
    def outs(stmts):
        n = 0
        for s in stmts:
            if s[0] == "out":
                n += 1
            elif s[0] == "if":
                a, b = outs(s[2]), outs(s[3])
                if a != b:
                    raise Refuse("branches of a rule print different numbers of outputs")
                n += a
            elif s[0] == "set":
                if s[1] == "cf":
                    raise Refuse("cf set inside a rule body")
                if n:
                    raise Refuse("assignment after the output")
            elif s[0] != "nop":
                raise Refuse(f"{s[0]} in a rule body")
        return n
    if outs(body) != 1:
        raise Refuse("rule body without exactly one output")


# --- SMV output ------------------------------------------------------------

def variables(e, acc):
    """The numeric globals e reads (a pointer read adds only its index's)."""
    if e[0] == "var":
        acc.add(e[1])
    elif e[0] == "idx":
        variables(e[2], acc)
    elif e[0] == "un":
        variables(e[2], acc)
    elif e[0] == "bin":
        variables(e[2], acc)
        variables(e[3], acc)
    return acc


def word_vars(globals_, rules):
    """Globals that need signed word[32]: any assigned a non-constant, and
    every global that shares an arithmetic expression or comparison with
    one of them."""
    words = set()
    links = []  # sets of globals that must share a type

    def scan(e):
        if e[0] == "bin":
            if e[1] not in ("&&", "||"):
                links.append(variables(e, set()))
            scan(e[2])
            scan(e[3])
        elif e[0] == "un":
            if e[1] == "-":
                links.append(variables(e, set()))
            scan(e[2])

    def scan_body(stmts):
        for s in stmts:
            if s[0] == "set":
                if s[2][0] not in ("num", "ptr"):
                    words.add(s[1])
                    links.append(variables(s[2], set()) | {s[1]})
                scan(s[2])
            elif s[0] == "if":
                scan(s[1])
                scan_body(s[2])
                scan_body(s[3])

    for guard, body in rules:
        for c in guard:
            scan(c)
        scan_body(body)
    changed = True
    while changed:
        changed = False
        for link in links:
            if link & words and not link <= words:
                words |= link
                changed = True
    return words & set(globals_)


class Smv:
    def __init__(self, words, arrays, targets):
        self.words = words
        self.arrays = arrays    # constant array -> contents
        self.targets = targets  # pointer -> the arrays it can point to
        self.defines = []
        self.count = 0
        self.gate = "TRUE"      # when the reads being translated execute
        self.bounds = []        # gate & index out of range, per read

    def is_word(self, e):
        return bool(variables(e, set()) & self.words)

    def num(self, n, word):
        if not word:
            return str(n)
        return f"0sd32_{n}" if n >= 0 else f"-0sd32_{-n}"

    def expr(self, e, env, word=None):
        """e as SMV text; env maps globals to their current SMV names."""
        kind = e[0]
        if word is None:
            word = self.is_word(e)
        if kind == "num":
            return self.num(e[1], word)
        if kind == "var":
            return env.get(e[1], e[1])
        if kind == "ptr":
            return e[1]
        if kind == "idx":
            return self.read(e, env, word)
        if kind == "un":
            if e[1] == "!":
                return f"!({self.boolean(e[2], env)})"
            return f"-({self.expr(e[2], env, word)})"
        op = e[1]
        if op in ("&&", "||"):
            return f"(({self.boolean(e[2], env)}) {'&' if op == '&&' else '|'} " \
                   f"({self.boolean(e[3], env)}))"
        if op in ("==", "!=", "<", "<=", ">", ">="):
            w = self.is_word(e[2]) or self.is_word(e[3])
            smv_op = {"==": "=", "!=": "!="}.get(op, op)
            return f"({self.expr(e[2], env, w)} {smv_op} {self.expr(e[3], env, w)})"
        smv_op = {"%": "mod"}.get(op, op)
        return f"({self.expr(e[2], env, word)} {smv_op} {self.expr(e[3], env, word)})"

    def read(self, e, env, word):
        """An array read as a case over the index (and over the pointer's
        targets). Out-of-range indices are recorded in self.bounds."""
        wi = self.is_word(e[2])
        i = self.expr(e[2], env, wi)

        def table(name):
            vals = self.arrays[name]
            if e[2][0] == "num" and 0 <= e[2][1] < len(vals):
                return self.num(vals[e[2][1]], word), None
            arms = "; ".join(f"{i} = {self.num(k, wi)} : {self.num(x, word)}"
                             for k, x in enumerate(vals))
            inside = f"{i} >= {self.num(0, wi)} & {i} < {self.num(len(vals), wi)}"
            return f"case {arms}; TRUE : {self.num(vals[0], word)}; esac", inside

        if e[1] in self.arrays:
            text, inside = table(e[1])
            if inside:
                self.bounds.append(self.define(f"({self.gate}) & !({inside})"))
            return text
        if len(self.targets[e[1]]) == 1:  # a constant pointer
            (t,) = self.targets[e[1]]
            text, inside = table(t[2:])
            if inside:
                self.bounds.append(self.define(f"({self.gate}) & !({inside})"))
            return text
        p = env.get(e[1], e[1])
        arms = []
        for t in sorted(self.targets[e[1]]):
            text, inside = table(t[2:])
            arms.append(f"{p} = {t} : {text}")
            if inside:
                self.bounds.append(self.define(f"({self.gate}) & {p} = {t} & !({inside})"))
        return f"case {'; '.join(arms)}; esac"

    def boolean(self, e, env):
        if e[0] == "num":
            return "TRUE" if e[1] else "FALSE"
        if e[0] == "var" or (e[0] == "bin" and e[1] in ("+", "-", "*", "/", "%")):
            raise Refuse("integer used as a condition")
        return self.expr(e, env)

    def define(self, text):
        self.count += 1
        name = f"d{self.count}"
        self.defines.append(f"  {name} := {text};")
        return name

    def body(self, stmts, env):
        """Run a rule body symbolically. Return (env after, output code).

        Every assignment becomes a DEFINE, so the sequential reads of the
        C body keep their meaning and shared subterms stay shared."""
        out = None
        for s in stmts:
            if s[0] == "set" and s[2][0] == "ptr" and len(self.targets[s[1]]) == 1:
                continue  # a constant pointer set to its only value
            if s[0] == "set":
                word = s[1] in self.words
                text = self.expr(s[2], env, word)
                env = dict(env)
                env[s[1]] = self.define(text)
            elif s[0] == "if":
                c = self.define(self.boolean(s[1], env))
                e1, o1 = self.body(s[2], env)
                e2, o2 = self.body(s[3], env)
                if o1 != o2:
                    out = (c, o1, o2)
                elif o1 is not None:
                    out = o1
                merged = dict(env)
                for v in set(e1) | set(e2):
                    if e1.get(v, v) != e2.get(v, v):
                        merged[v] = self.define(
                            f"case {c} : {e1.get(v, v)}; TRUE : {e2.get(v, v)}; esac")
                env = merged
            elif s[0] == "out":
                out = s[1]
        return env, out


def out_text(out):
    if isinstance(out, int):
        return f"e_o{chr(64 + out)}"
    c, a, b = out
    return f"case {c} : {out_text(a)}; TRUE : {out_text(b)}; esac"


def translate(c_source, atoms):
    globals_, funcs, inputs, arrays = Parser(tokenize(c_source)).program()
    if inputs is None or "cf" not in globals_:
        raise Refuse("no inputs array or no cf")
    globals_.pop("cf")
    per_input = {v: rules_for_input(funcs, v, arrays) for v in inputs}
    all_rules = [r for rs in per_input.values() for r in rs]
    for _, body in all_rules:
        check_body(body)
    for guard, body in all_rules:
        for c in guard:
            if variables(c, set()) - set(globals_):
                raise Refuse(f"unknown name in {c}")
    words = word_vars(globals_, all_rules)
    events = ["s_start", "s_dead"] + [f"e_{a}" for a in atoms]
    outputs = [f"e_{a}" for a in atoms if a[0] == "o"]

    # Value sets of the enumerated globals.
    values = {v: {init} for v, init in globals_.items() if v not in words}

    def collect(stmts):
        for s in stmts:
            if s[0] == "set" and s[1] in values:
                values[s[1]].add(s[2][1])
            elif s[0] == "if":
                collect(s[2])
                collect(s[3])

    for _, body in all_rules:
        collect(body)
    smv = Smv(words, arrays, values)

    env0 = {v: v for v in globals_}
    cases = {v: [] for v in globals_}
    po_cases, en = [], {}
    for v in inputs:
        if f"i{chr(64 + v)}" not in atoms:
            raise Refuse(f"input {v} is not an atom")
        guards = []
        for guard, body in per_input[v]:
            smv.gate = f"ready & inp = {v}"
            g = smv.define(" & ".join(f"({smv.boolean(c, env0)})" for c in guard) or "TRUE")
            # The first rule whose guard holds fires; later ones see cf = 0.
            fires = smv.define(" & ".join([g] + [f"!{h}" for h in guards]))
            guards.append(g)
            smv.gate = f"ready & inp = {v} & {fires}"
            env, out = smv.body(body, env0)
            fire = f"inp = {v} & {fires}"
            for var in globals_:
                if env[var] != var:
                    cases[var].append(f"ready & {fire} : {env[var]};")
            po_cases.append(f"ready & {fire} : {out_text(out)};")
        en[v] = smv.define(" | ".join(guards) or "FALSE")

    lines = ["MODULE main", "IVAR", f"  inp : {{{', '.join(map(str, inputs))}}};", "VAR",
             f"  ev : {{{', '.join(events)}}};",
             f"  po : {{{', '.join(outputs)}}};"]
    for v in globals_:
        if v in words:
            lines.append(f"  {v} : signed word[32];")
        else:
            if len(values[v]) > 1:
                lines.append(f"  {v} : {{{', '.join(map(str, sorted(values[v])))}}};")
    # A global with one value is a constant (nuXmv refuses init() on it).
    state = [v for v in globals_ if v in words or len(values[v]) > 1]
    lines.append("DEFINE")
    lines += [f"  {v} := {globals_[v]};" for v in globals_
              if v not in state and not str(globals_[v]).startswith("t_")]
    lines.append("  dead := ev = s_dead;")
    lines.append(f"  ready := ev = s_start | ev in {{{', '.join(outputs)}}};")
    for a in atoms:
        lines.append(f"  {a} := ev = e_{a};")
    lines += smv.defines
    lines.append(f"  oob := {' | '.join(smv.bounds) or 'FALSE'};")
    lines.append("ASSIGN")
    lines.append("  init(ev) := s_start;")
    lines.append(f"  init(po) := {outputs[0]};")
    for v in state:
        lines.append(f"  init({v}) := {smv.num(globals_[v], v in words)};")
    ev_cases = [f"ready & inp = {v} & {en[v]} : e_i{chr(64 + v)};" for v in inputs]
    lines.append("  next(ev) := case")
    lines += [f"    {c}" for c in ev_cases]
    lines.append(f"    ev in {{{', '.join(f'e_i{chr(64 + v)}' for v in inputs)}}} : po;")
    lines.append("    TRUE : s_dead;")
    lines.append("  esac;")
    lines.append("  next(po) := case")
    lines += [f"    {c}" for c in po_cases]
    lines.append("    TRUE : po;")
    lines.append("  esac;")
    for v in state:
        lines.append(f"  next({v}) := case")
        lines += [f"    {c}" for c in cases[v]]
        lines.append(f"    TRUE : {v};")
        lines.append("  esac;")
    return "\n".join(lines) + "\n"


# --- LTL -------------------------------------------------------------------

LTL_TOKEN = re.compile(r"\s*(WU|[()!&|]|[A-Za-z]\w*)")


def ltl_to_smv(f):
    """A RERS LTL formula in nuXmv syntax. W is spelled out, as nuXmv's
    LTL has no weak until: a W b = (a U b) | G a."""
    toks = LTL_TOKEN.findall(f)
    if "".join(toks) != re.sub(r"\s+", "", f):
        raise ValueError(f"cannot tokenise {f!r}")
    i = 0

    def peek():
        return toks[i] if i < len(toks) else None

    def take():
        nonlocal i
        i += 1
        return toks[i - 1]

    def binary(ops, sub):
        def rule():
            lhs = sub()
            while peek() in ops:
                op = take()
                rhs = sub()
                if op == "WU":
                    lhs = f"(({lhs}) U ({rhs}) | G ({lhs}))"
                else:
                    lhs = f"({lhs} {'V' if op == 'R' else op} {rhs})"
            return lhs
        return rule

    def unary():
        t = peek()
        if t in ("!", "X"):
            take()
            return f"{t} {unary()}"
        if t == "(":
            take()
            e = disj()
            assert take() == ")", f
            return f"({e})"
        take()
        return {"true": "TRUE", "false": "FALSE"}.get(t, t)

    temporal = binary(("U", "R", "WU"), unary)
    conj = binary(("&",), temporal)
    disj = binary(("|",), conj)
    out = disj()
    assert i == len(toks), f
    return out


def spec(f):
    """The nuXmv property for RERS formula f on the translated model."""
    return f"X ((G !dead) -> ({ltl_to_smv(f)}))"


def nuxmv(model, props, engine, timeout, memory_gb):
    """Check LTL properties in one nuXmv session, so the model is built
    once. Return [(verdict, seconds)] in order, and the build seconds.
    A property the session did not reach (timeout, memory cap) is unknown."""
    build = {"ic3": "go_bmc", "bdd": "go"}[engine]
    command = {"ic3": "check_ltlspec_ic3 -d", "bdd": "check_ltlspec"}[engine]
    lines = ["set on_failure_script_quits 1", build, "time", "echo @@build"]
    for k, prop in enumerate(props):
        lines += [f'{command} -p "{prop}"', "time", f"echo @@{k}"]
    cmd = ["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={memory_gb}G",
           "-p", "MemorySwapMax=0", "nice", "-n", "10", "timeout", str(timeout),
           "nuXmv", "-int", str(model)]
    r = subprocess.run(cmd, input="\n".join(lines + ["quit", ""]), capture_output=True,
                       text=True)
    segments = re.split(r"@@(\w+)[ \t]*$", r.stdout, flags=re.M)
    done = dict(zip(segments[1::2], segments[0::2]))

    def elapsed(text):
        m = re.search(r"elapse: ([\d.]+) seconds", text or "")
        return float(m.group(1)) if m else float("nan")

    if r.returncode == 124:
        why = "unknown (timeout)"
    else:
        tail = (r.stderr or r.stdout).strip().splitlines()[-1:] or [""]
        why = f"unknown (exit {r.returncode}: {tail[0][:60]})"
    out = []
    for k in range(len(props)):
        text = done.get(str(k))
        found = re.findall(r"^-- .*specification .* is (true|false)\s*$", text or "", re.M)
        if text is None:
            out.append((why, float("nan")))
        elif len(found) == 1:
            out.append(({"true": "holds", "false": "fails"}[found[0]], elapsed(text)))
        else:
            out.append(("unknown (no verdict)", elapsed(text)))
    return out, elapsed(done.get("build"))


def check(year, prob, args):
    """Translate one problem and check its properties; write a TSV.

    Rows: every shipped property phi (for validation against the official
    verdicts) and, for each mistranslated one, !phi, the fix and !fix.
    A verdict `holds` for !f means no run satisfies f.
    """
    import rers_seq
    atoms, props = rers_seq.load_properties(year, prob)
    sols = rers_seq.load_solutions(year, prob)
    out = Path(args.out_dir) / f"smv-{year}-{prob}.tsv"
    model = Path(args.out_dir) / f"smv-{year}-{prob}.smv"
    try:
        model.write_text(translate(rers_seq.problem_source(year, prob).read_text(), atoms))
    except Refuse as e:
        out.write_text(f"# year={year} problem={prob} refused: {e}\n")
        return f"{year}/{prob}: refused: {e}"
    # The bounds check and the affected properties first, so that a
    # session that times out loses only validation rows.
    jobs, others = [(-1, "bounds", "holds", "G !oob")], []
    for (n, _, f), (_, official, _) in zip(props, sols):
        fixed = rers_seq.corrected(f)
        row = (n, "phi", "holds" if official else "fails", spec(f))
        if fixed is None:
            if not args.only_affected:
                others.append(row)
            continue
        jobs += [row, (n, "negphi", "", f"X ((G !dead) -> !({ltl_to_smv(f)}))"),
                 (n, "fix", "", spec(fixed)),
                 (n, "negfix", "", f"X ((G !dead) -> !({ltl_to_smv(fixed)}))")]
    jobs += others
    res = [None] * len(jobs)
    # A model without array reads has `oob := FALSE`; nothing to check.
    if "  oob := FALSE;" in model.read_text():
        res[0] = ("holds", 0.0)
    todo = [i for i in range(len(jobs)) if res[i] is None]
    batches = [todo[k::args.jobs] for k in range(args.jobs) if todo[k::args.jobs]]
    with cf.ThreadPoolExecutor(len(batches)) as ex:
        runs = list(ex.map(lambda b: nuxmv(model, [jobs[i][3] for i in b], args.engine,
                                           args.timeout, args.memory_gb), batches))
    for b, (verdicts, _) in zip(batches, runs):
        for i, v in zip(b, verdicts):
            res[i] = v
    builds = " ".join(f"{t:.1f}" for _, t in runs)
    with open(out, "w") as fh:
        fh.write(f"# year={year} problem={prob} engine={args.engine} timeout={args.timeout}"
                 f" sessions={len(batches)} build_s={builds}\n")
        fh.write("prop\tformula\tofficial\tverdict\tseconds\n")
        for (n, kind, official, _), (v, dt) in zip(jobs, res):
            fh.write(f"{n}\t{kind}\t{official}\t{v}\t{dt:.1f}\n")
    bad = sum(1 for (_, k, o, _), (v, _) in zip(jobs, res) if k in ("phi", "bounds") and
              v != o and not v.startswith("unknown"))
    unk = sum(1 for v, _ in res if v.startswith("unknown"))
    return f"{year}/{prob}: {len(jobs)} checks, {unk} unknown, {bad} disagree with official"


if __name__ == "__main__":
    import argparse

    import rers_seq
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("command", choices=["model", "check"])
    ap.add_argument("--years", type=int, nargs="*", default=[2016, 2017])
    ap.add_argument("--problems", type=int, nargs="*", default=list(range(1, 10)))
    ap.add_argument("--engine", choices=["ic3", "bdd"], default="ic3")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--timeout", type=int, default=3600, help="seconds per nuXmv session")
    ap.add_argument("--memory-gb", type=int, default=4, help="cap per nuXmv call")
    ap.add_argument("--only-affected", action="store_true")
    ap.add_argument("--out-dir", default=".")
    args = ap.parse_args()
    for year in args.years:
        for prob in args.problems:
            if args.command == "model":
                atoms, _ = rers_seq.load_properties(year, prob)
                print(translate(rers_seq.problem_source(year, prob).read_text(), atoms), end="")
            else:
                print(check(year, prob, args), flush=True)
