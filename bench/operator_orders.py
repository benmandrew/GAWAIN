#!/usr/bin/env python3
"""Decide the implication order between LTL and MTL operator templates with Spot.

usage: operator_orders.py OUT_DIR [--bound N]

Templates are formulas over the atoms p and q. Validity is closed under
uniform substitution, so an implication between two templates holds for all
operands exactly when it holds on the atoms. Each pair A, B is decided as the
unsatisfiability of A & !B in one `ltlfilt --satisfiable` batch.

Part A: LTL templates of small depth. Prints the covering edges of the order
between equivalence classes, and which covers one monotone rewrite realises
in one step, applied to any member of the class, under each rule set of
src/search/operators.rs (`RuleSet::Peredur` and `RuleSet::Ordered`). The
rewrites come from the code itself through target/release/rule_table, built
with `nix develop -c cargo build --release --bin rule_table`. Constant and
AddOperand never land on a template, so they drop out. M is left out, since
GAWAIN's parser rejects it.

Part B: MTL over discrete pointwise time, intervals [a, b] with a <= N and b
<= N or b = inf, expanded to LTL here. `Ui` is the usual until (the left
operand holds at every position before the witness), `Uc` is CEGIW's
(`X^a (l U[0,b-a] r)`, ~/projects/cegiw/src/logic/mtl.py:144), and `Ri` is
the dual of `Ui`. Each closed-form rule is checked against Spot on every pair
of the grid, and the mismatches are printed.

Writes OUT_DIR/operator-orders-ltl.csv (covers) and
OUT_DIR/operator-orders-mtl.csv (one row per rule and pair checked).
"""
import csv, itertools, os, subprocess, sys, tempfile

INF = None


# Formulas are tuples: ('ap', name), (op, child) for X F G, (op, l, r) for
# U W R M & |, and ('Fi'|'Gi', a, b, f), ('Ui'|'Uc'|'Ri', a, b, l, r).
def ap(n): return ('ap', n)


P, Q = ap('p'), ap('q')


def spot(f):
    k = f[0]
    if k == 'ap': return f[1]
    if k in ('X', 'F', 'G'): return f'{k}({spot(f[1])})'
    if k == '!': return f'!({spot(f[1])})'
    if k in ('U', 'W', 'R', 'M', '&', '|'): return f'({spot(f[1])}) {k} ({spot(f[2])})'
    if k in ('Fi', 'Gi'):
        a, b, g = f[1], f[2], spot(f[3])
        op = k[0]
        body = f'{op}({g})'
        if b is not INF:
            body = g
            joiner = '|' if op == 'F' else '&'
            for _ in range(b - a):
                body = f'({g}) {joiner} X({body})'
        return 'X(' * a + body + ')' * a
    if k in ('Ui', 'Uc', 'Ri'):
        a, b, l, r = f[1], f[2], f[3], f[4]
        if k == 'Ri':
            return f'!({spot(("Ui", a, b, ("!", l), ("!", r)))})'
        inner = until0(spot(l), spot(r), None if b is INF else b - a)
        shifted = 'X(' * a + inner + ')' * a
        if k == 'Uc' or a == 0:
            return shifted
        return f'({spot(("Gi", 0, a - 1, l))}) & ({shifted})'
    raise ValueError(f)


def until0(l, r, n):
    """l U[0,n] r, or l U r when n is None."""
    if n is None: return f'({l}) U ({r})'
    out = r
    for _ in range(n):
        out = f'({r}) | (({l}) & X({out}))'
    return out


def implications(pairs):
    """For each (A, B) of spot strings, whether A -> B is valid."""
    with tempfile.NamedTemporaryFile('w', suffix='.ltl', delete=False) as t:
        for a, b in pairs:
            t.write(f'({a}) & !({b})\n')
        name = t.name
    try:
        out = subprocess.run(['ltlfilt', '-F', name, '--satisfiable', '--format=%L'],
                             capture_output=True, text=True, check=False)
        if out.returncode not in (0, 1):
            sys.exit(f'ltlfilt failed: {out.stderr}')
        sat = {int(x) for x in out.stdout.split()}
    finally:
        os.unlink(name)
    return [i + 1 not in sat for i in range(len(pairs))]


def show(f):
    k = f[0]
    if k == 'ap': return f[1]
    if k in ('X', 'F', 'G'):
        c = show(f[1])
        return k + (c if f[1][0] in ('ap', 'X', 'F', 'G') else f'({c})')
    if k in ('U', 'W', 'R', 'M', '&', '|'):
        def side(g): return show(g) if g[0] in ('ap', 'X', 'F', 'G') else f'({show(g)})'
        return f'{side(f[1])} {k} {side(f[2])}'
    raise ValueError(f)


def size(f):
    return 1 + sum(size(c) for c in f[1:] if isinstance(c, tuple))


# GAWAIN's monotone rules at a node, both directions, without Constant and
# AddOperand. Every template here is negation-free, so every site is positive.
RULE_TABLE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                          'target', 'release', 'rule_table')
FROM_SEXP = {'and': '&', 'or': '|', 'U': 'U', 'W': 'W', 'R': 'R', 'X': 'X', 'F': 'F', 'G': 'G'}


def from_sexp(text):
    """A rule_table S-expression as a template tuple, or None when it holds an
    operator or constant that no template has."""
    tokens = text.replace('(', ' ( ').replace(')', ' ) ').split()
    pos = 0

    def item():
        nonlocal pos
        tok = tokens[pos]
        pos += 1
        if tok != '(':
            return ap(tok) if tok in ('p', 'q') else None
        op = tokens[pos]
        pos += 1
        args = []
        while tokens[pos] != ')':
            args.append(item())
        pos += 1
        if op not in FROM_SEXP or None in args: return None
        return (FROM_SEXP[op], *args)
    return item()


def rewrites_of(templates):
    """For each template, the outcomes of `monotone_outcomes` as
    {(weaken, set): [template]}, read from target/release/rule_table."""
    if not os.path.exists(RULE_TABLE):
        sys.exit(f'{RULE_TABLE} is missing: run '
                 'nix develop -c cargo build --release --bin rule_table')
    text = ''.join(spot(t) + '\n' for t in templates)
    out = subprocess.run([RULE_TABLE], input=text, capture_output=True, text=True, check=True)
    lines = out.stdout.splitlines()
    result = []
    for i in range(len(templates)):
        rows = {}
        for line in lines[4 * i:4 * i + 4]:
            _, d, rs, outs = (line.split('\t') + [''])[:4]
            got = [from_sexp(o) for o in outs.split(' ; ') if o]
            rows[(d == 'w', rs)] = [g for g in got if g is not None]
        result.append(rows)
    return result


def ltl_templates():
    unary = [P]
    frontier = [P]
    for _ in range(3):
        frontier = [(op, g) for g in frontier for op in 'XFG']
        unary += frontier
    wraps = [lambda g: g, lambda g: ('X', g), lambda g: ('F', g), lambda g: ('G', g)]
    binary = [Q, ('F', Q), ('G', Q), ('X', Q), ('G', P), ('F', P)]
    for op in ['U', 'W', 'R', '&', '|']:
        for wl, wr in itertools.product(wraps, wraps):
            binary.append((op, wl(P), wr(Q)))
        for op2 in 'XFG':
            binary.append((op2, (op, P, Q)))
    return unary, binary


def part_a(out_dir):
    unary, binary = ltl_templates()
    rows = []
    stats = {}
    for name, ts in (('unary', unary), ('binary', binary)):
        ts = list(dict.fromkeys(ts))
        pairs = [(i, j) for i in range(len(ts)) for j in range(len(ts)) if i != j]
        imp = dict(zip(pairs, implications([(spot(ts[i]), spot(ts[j])) for i, j in pairs])))
        le = lambda i, j: i == j or imp[(i, j)]
        # Classes by mutual implication, each named by its smallest member.
        cls = {}
        reps = []
        for i in range(len(ts)):
            for r in reps:
                if le(i, r) and le(r, i):
                    cls[i] = r
                    break
            else:
                reps.append(i)
                cls[i] = i
        for r in list(reps):
            best = min((i for i in cls if cls[i] == r), key=lambda i: (size(ts[i]), show(ts[i])))
            for i in cls:
                if cls[i] == r: cls[i] = best
            reps[reps.index(r)] = best
        strict = lambda a, b: a != b and le(a, b)
        covers = [(a, b) for a in reps for b in reps if strict(a, b)
                  and not any(strict(a, c) and strict(c, b) for c in reps)]
        index = {t: i for i, t in enumerate(ts)}
        outcomes = rewrites_of(ts)
        def step_set(rule_set):
            steps = set()
            for i, t in enumerate(ts):
                for w in outcomes[i][(True, rule_set)]:
                    if w in index and cls[index[w]] != cls[i]: steps.add((cls[i], cls[index[w]]))
                for s_ in outcomes[i][(False, rule_set)]:
                    if s_ in index and cls[index[s_]] != cls[i]: steps.add((cls[index[s_]], cls[i]))
            return steps
        steps, extra = step_set('peredur'), step_set('ordered')
        unsound = [(a, b) for a, b in steps | extra if not le(a, b)]
        realised = sum((a, b) in steps for a, b in covers)
        realised_p = sum((a, b) in extra for a, b in covers)
        skips = sum(1 for a, b in steps if (a, b) not in covers)
        stats[name] = (len(ts), len(reps), len(covers), realised, realised_p, skips, len(unsound))
        print(f'== Part A, {name}: {len(ts)} templates, {len(reps)} classes, '
              f'{len(covers)} covers, {realised} realised by one GAWAIN step, '
              f'{realised_p} with the ordered rules, '
              f'{skips} GAWAIN steps skip a level, {len(unsound)} unsound')
        members = {r: sorted((show(ts[i]) for i in cls if cls[i] == r and i != r), key=len) for r in reps}
        for a, b in sorted(covers, key=lambda e: (size(ts[e[0]]), show(ts[e[0]]), show(ts[e[1]]))):
            mark = 'gawain' if (a, b) in steps else 'ordered' if (a, b) in extra else 'missing'
            print(f'  {show(ts[a]):18} => {show(ts[b]):18} {mark}')
            rows.append({'part': name, 'stronger': show(ts[a]), 'weaker': show(ts[b]),
                         'gawain_step': int((a, b) in steps),
                         'ordered_step': int((a, b) in extra),
                         'stronger_class': ' = '.join(members[a][:4]),
                         'weaker_class': ' = '.join(members[b][:4])})
    with open(os.path.join(out_dir, 'operator-orders-ltl.csv'), 'w', newline='') as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    return stats


# Interval arithmetic for the closed-form rules. An interval is (a, b) with
# b = INF for an unbounded one.
def subset(i, j):
    return j[0] <= i[0] and (j[1] is INF or (i[1] is not INF and i[1] <= j[1]))


def meets(i, j):
    lo = max(i[0], j[0])
    hi = min(x for x in (i[1], j[1]) if x is not INF) if (i[1], j[1]) != (INF, INF) else INF
    return hi is INF or lo <= hi


def add(i, j):
    return (i[0] + j[0], INF if INF in (i[1], j[1]) else i[1] + j[1])


def intervals(n, unbounded=True):
    out = []
    for a in range(n + 1):
        out += [(a, b) for b in range(a, n + 1)]
        if unbounded: out.append((a, INF))
    return out


def fmt(i):
    return f'[{i[0]},{"inf" if i[1] is INF else i[1]}]'


def part_b(out_dir, n):
    I = intervals(n)
    small = intervals(2, unbounded=False)
    checks = []  # (rule, label, A, B, predicted)

    def rule(name, cases):
        for label, a, b, pred in cases:
            checks.append((name, label, a, b, pred))

    pairs = list(itertools.product(I, I))
    rule('F_I p => F_J p iff I sub J',
         [(f'{fmt(i)} {fmt(j)}', ('Fi', *i, P), ('Fi', *j, P), subset(i, j)) for i, j in pairs])
    rule('G_I p => G_J p iff J sub I',
         [(f'{fmt(i)} {fmt(j)}', ('Gi', *i, P), ('Gi', *j, P), subset(j, i)) for i, j in pairs])
    rule('G_I p => F_J p iff I meets J',
         [(f'{fmt(i)} {fmt(j)}', ('Gi', *i, P), ('Fi', *j, P), meets(i, j)) for i, j in pairs])
    rule('F_I p => G_J p iff I = J = [k,k]',
         [(f'{fmt(i)} {fmt(j)}', ('Fi', *i, P), ('Gi', *j, P), i == j and i[0] == i[1]) for i, j in pairs])
    rule('p Ui_I q => p Ui_J q iff I sub J',
         [(f'{fmt(i)} {fmt(j)}', ('Ui', *i, P, Q), ('Ui', *j, P, Q), subset(i, j)) for i, j in pairs])
    rule('p Ri_J q => p Ri_I q iff I sub J',
         [(f'{fmt(j)} {fmt(i)}', ('Ri', *j, P, Q), ('Ri', *i, P, Q), subset(i, j)) for i, j in pairs])
    rule('p Uc_I q => p Uc_J q iff a_I = a_J and I sub J',
         [(f'{fmt(i)} {fmt(j)}', ('Uc', *i, P, Q), ('Uc', *j, P, Q), i[0] == j[0] and subset(i, j))
          for i, j in pairs])
    rule('p Ui_I q => F_J q iff I sub J',
         [(f'{fmt(i)} {fmt(j)}', ('Ui', *i, P, Q), ('Fi', *j, Q), subset(i, j)) for i, j in pairs])
    rule('G_J q => p Ri_I q iff I sub J',
         [(f'{fmt(j)} {fmt(i)}', ('Gi', *j, Q), ('Ri', *i, P, Q), subset(i, j)) for i, j in pairs])
    rule('p Ui_I q => p Uc_I q, always',
         [(fmt(i), ('Ui', *i, P, Q), ('Uc', *i, P, Q), True) for i in I])
    rule('p Uc_I q => p Ui_I q iff a = 0',
         [(fmt(i), ('Uc', *i, P, Q), ('Ui', *i, P, Q), i[0] == 0) for i in I])
    # Nesting: equivalence to the Minkowski sum, both directions.
    for op in 'FG':
        cases = []
        for i, j in itertools.product(small, small):
            nested = (op + 'i', *i, (op + 'i', *j, P))
            flat = (op + 'i', *add(i, j), P)
            cases.append((f'{fmt(i)} {fmt(j)} ->', nested, flat, True))
            cases.append((f'{fmt(i)} {fmt(j)} <-', flat, nested, True))
        rule(f'{op}_I {op}_J p = {op}_(I+J) p', cases)
    # F_I G_J p => G_K F_L p iff I - K sub L - J (Minkowski differences).
    cases = []
    for i, j, k, l in itertools.product(small, repeat=4):
        lhs = (i[0] - k[1], i[1] - k[0])
        rhs = (l[0] - j[1], l[1] - j[0])
        pred = rhs[0] <= lhs[0] and lhs[1] <= rhs[1]
        cases.append((f'{fmt(i)}{fmt(j)} {fmt(k)}{fmt(l)}',
                      ('Fi', *i, ('Gi', *j, P)), ('Gi', *k, ('Fi', *l, P)), pred))
    rule('F_I G_J p => G_K F_L p iff I - K sub L - J', cases)

    verdicts = implications([(spot(a), spot(b)) for _, _, a, b, _ in checks])
    rows = []
    by_rule = {}
    for (name, label, _, _, pred), got in zip(checks, verdicts):
        s = by_rule.setdefault(name, [0, 0, []])
        s[0] += 1
        if pred != got:
            s[1] += 1
            s[2].append(f'{label}: predicted {pred}, Spot {got}')
        rows.append({'rule': name, 'case': label, 'predicted': int(pred), 'spot': int(got)})
    print(f'== Part B, bound {n}: {len(I)} intervals, {len(checks)} pairs')
    for name, (total, wrong, examples) in by_rule.items():
        print(f'  {name}: {total} pairs, {wrong} mismatches')
        for e in examples[:6]: print(f'      {e}')
    with open(os.path.join(out_dir, 'operator-orders-mtl.csv'), 'w', newline='') as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)


def main():
    args = sys.argv[1:]
    if not args: sys.exit(__doc__)
    n = 4
    if '--bound' in args:
        k = args.index('--bound')
        n = int(args[k + 1])
        del args[k:k + 2]
    os.makedirs(args[0], exist_ok=True)
    part_a(args[0])
    part_b(args[0], n)


if __name__ == '__main__':
    main()
