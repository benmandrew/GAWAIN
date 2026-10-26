#!/usr/bin/env python3
"""Build bench/sets/rers-precedes.csv: the mistranslated RERS 2016/2017
sequential properties, each with its intended pattern as a weakening.

Three Dwyer patterns are translated into the RERS 2016 and 2017 sequential
property files as `false R (Q & psi)`, that is G(Q & psi), where the
pattern is G(Q -> psi). G(Q & psi) needs Q at every position, which no
run of an input/output program has, so every such property is false. The
three patterns, with their LTL from Dwyer, Avrunin and Corbett,
"Patterns in Property Specifications for Finite-State Verification"
(ICSE 1999), catalogue at https://matthewbdwyer.github.io/psp/:

  "P precedes S after Q until R"      G((Q & !R) -> (!S W (P | R)))
  "S responds to P after Q until R"   G((Q & !R) -> ((P -> (!R U (S & !R))) W R))
  "S always responds to P, T"         G((P & X F T) -> X F(T & F S))

(Dwyer names the events the other way round; the letters here follow the
RERS gloss.) For each affected property the script

  1. builds the fix by replacing `(Q &` with `(! Q |`, and checks with
     Spot that the fix is equivalent to the pattern instantiated from the
     English gloss;
  2. checks with Spot that phi implies the fix and that the fix does not
     imply phi, both in plain LTL and under the RERS trace assumption that
     exactly one event holds at each position;
  3. merges the verdicts on M of phi, !phi, the fix and !fix from these
     sound routes, and stops if they disagree: bench/rers_seq.py (explicit
     search plus the official lassos, and implication from officially true
     properties), bench/rers_seq_smv.py (nuXmv IC3 on a translation of
     the C program, used only when it matches the official verdicts),
     bench/rers_seq_abs.py (an interval abstraction of the program, with
     abstract counterexamples kept only when they replay), the output
     alphabet (outputs that no printf in the program can print are false
     on every run) and bench/rers_seq_walk.py (random walks that close a
     lasso, kept only when the lasso replays and Spot accepts it) and
     bench/rers_seq_back.py (a backward search that proves an event never
     occurs; such events join the never-printed outputs of the alphabet);
  4. shows phi degenerate (M |= !phi) with Spot: phi is unsatisfiable on
     any infinite run with one event per position and infinitely many
     inputs and outputs, which every run of M is.

The parallel track (Promela) properties carry no English gloss and none
has the G(Q & ...) shape, so the set is sequential only.

Columns: `phi` is the shipped formula and `weakening` the fix, both in
Spot syntax; `*_verdict` is holds, fails or unknown on M; `degenerate`
and `weakening_degenerate` say whether M |= !phi and M |= !fix; each
`*_route` names the routes (spot, explicit, nuxmv, abstract, alphabet,
backward, walk, official)
that agree on the value next to it.

Usage: rers_precedes.py EXPLICIT_DIR SMV_DIR ABS_DIR WALK_DIR BACK_DIR
(the output directories of rers_seq.py, rers_seq_smv.py, rers_seq_abs.py,
rers_seq_walk.py and rers_seq_back.py)
"""

import csv
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rers_seq  # noqa: E402

HERE = Path(__file__).resolve().parent
OUT = HERE / "sets" / "rers-precedes.csv"

EV = r"(input|output) ([A-Z])"
PATTERNS = [
    ("precedence after-until",
     re.compile(rf"^{EV} precedes {EV} after {EV} until {EV}$"),
     "G(({Q} & !{R}) -> (!{S} W ({P} | {R})))", "PSQR"),
    ("response after-until",
     re.compile(rf"^{EV} responds to {EV} after {EV} until {EV}$"),
     "G(({Q} & !{R}) -> (({P} -> (!{R} U ({S} & !{R}))) W {R}))", "SPQR"),
    ("response chain 2-1 globally",
     re.compile(rf"^{EV} always responds to {EV}, {EV}$"),
     "G(({P} & X F {T}) -> X F({T} & F {S}))", "SPT"),
]


def atom(kind, letter):
    return ("i" if kind == "input" else "o") + letter


def from_gloss(gloss):
    for name, rx, template, roles in PATTERNS:
        m = rx.match(gloss)
        if m:
            g = m.groups()
            names = {r: atom(g[2 * k], g[2 * k + 1]) for k, r in enumerate(roles)}
            return name, template.format(**names)
    raise ValueError(f"no pattern for gloss {gloss!r}")


def unsat(f):
    r = subprocess.run(["ltlfilt", "--equivalent-to=false", "-f", f], capture_output=True,
                       text=True, timeout=600, check=False)
    if r.returncode not in (0, 1):
        raise RuntimeError(r.stderr)
    return bool(r.stdout.strip())


def implies(a, b, assume="true"):
    return unsat(f"({assume}) & ({a}) & !({b})")


def one_event(atoms):
    """G(exactly one atom holds)."""
    some = " | ".join(atoms)
    pairs = " & ".join(f"!({a} & {b})" for k, a in enumerate(atoms) for b in atoms[k + 1:])
    return f"G(({some}) & {pairs})"


def read_table(path):
    """Rows of a verdict TSV keyed by (prop, formula kind), and its header."""
    if not path.exists():
        return {}, ""
    text = path.read_text().splitlines()
    header = text[0] if text and text[0].startswith("#") else ""
    rows = [ln for ln in text if not ln.startswith("#")]
    return list(csv.DictReader(rows, delimiter="\t")), header


def smv_verdicts(path, official):
    """The nuXmv verdicts of bench/rers_seq_smv.py, as {(prop, kind): verdict},
    or {} when the problem was refused or its model fails validation: the
    bounds invariant must hold, and every decided phi must match the
    official verdict (a mismatch stops the build)."""
    rows, header = read_table(path)
    if not rows or "refused" in header:
        return {}
    got = {(int(r["prop"]), r["formula"]): r["verdict"] for r in rows}
    if got.get((-1, "bounds")) != "holds":
        return {}
    for (n, kind), v in got.items():
        if kind == "phi" and v in ("holds", "fails"):
            assert v == official[n], (path, n, v, official[n])
    return {k: v for k, v in got.items() if v in ("holds", "fails")}


def abs_verdicts(path, official):
    """The decided verdicts of bench/rers_seq_abs.py, as {(prop, kind):
    verdict}, or {} when the abstract search did not finish or saw an
    out-of-bounds array read. An abstract `holds` on an officially false
    property stops the build."""
    rows, header = read_table(path)
    if not rows or "complete=True" not in header or "oob_reads=0" not in header:
        return {}
    got = {(int(r["prop"]), r["formula"]): r["verdict"] for r in rows}
    for (n, kind), v in got.items():
        if kind == "phi" and v == "holds":
            assert official[n] == "holds", (path, n)
    return {k: v for k, v in got.items() if v in ("holds", "fails")}


def printed_outputs(year, prob):
    """The output values the program can print: the constants of its
    `printf("%d\\n", K)` calls. Stops on any other printf, so no output
    can come from a computed value."""
    text = rers_seq.problem_source(year, prob).read_text()
    calls = re.findall(r"\bf?printf\s*\(([^;]*)\);", text)
    printed = set()
    for c in calls:
        m = re.fullmatch(r'"%d\\n",\s*(\d+)', c.strip())
        if m:
            printed.add(int(m.group(1)))
        else:
            assert c.startswith("stderr,"), (year, prob, c)
    return printed


def walk_verdicts(walk_dir):
    """The counterexamples of bench/rers_seq_walk.py in every walk-*.tsv of
    walk_dir, as {(year, problem, prop): "fails"}. The walk reports a fix
    as failing only for a lasso that replays and that Spot accepts in the
    automaton of !fix."""
    got = {}
    for path in sorted(Path(walk_dir).glob("walk-*.tsv")):
        rows, _ = read_table(path)
        for r in rows:
            if r["verdict"] == "fails":
                got[(int(r["year"]), int(r["problem"]), int(r["prop"]))] = "fails"
    return got


def back_unreachable(back_dir):
    """The events that bench/rers_seq_back.py proves never occur, as
    {(year, problem): {atom}}. A proof counts only when the search dropped
    no out-of-bounds array read."""
    got = {}
    for path in sorted(Path(back_dir).glob("back-*.tsv")):
        rows, _ = read_table(path)
        for r in rows:
            if r["verdict"] == "unreachable" and r["oob_dropped"] == "0" and \
                    r["oob_interval"] == "0":
                got.setdefault((int(r["year"]), int(r["problem"])), set()).add(r["event"])
    return got


def decide(claims, what):
    """Merge (route, answer) claims; all decided routes must agree."""
    claims = [(r, a) for r, a in claims if a is not None]
    answers = {a for _, a in claims}
    assert len(answers) <= 1, (what, claims)
    if not claims:
        return "unknown", ""
    return answers.pop(), "+".join(sorted({r for r, _ in claims}))


def main(explicit_dir, smv_dir, abs_dir, walk_dir, back_dir):
    walks = walk_verdicts(walk_dir)
    unreachable = back_unreachable(back_dir)
    rows = []
    alphabet_proved = {}
    back_proved = {}
    for year in (2016, 2017):
        for prob in range(1, 10):
            atoms, props = rers_seq.load_properties(year, prob)
            official = {n: "holds" if s[1] else "fails"
                        for (n, _, _), s in zip(props, rers_seq.load_solutions(year, prob))}
            ex, _ = read_table(Path(explicit_dir) / f"{year}-{prob}.tsv")
            ex = {int(r["prop"]): r for r in ex}
            sm = smv_verdicts(Path(smv_dir) / f"smv-{year}-{prob}.tsv", official)
            ab = abs_verdicts(Path(abs_dir) / f"abs-{year}-{prob}.tsv", official)
            mutex = one_event(atoms)
            ins = " | ".join(a for a in atoms if a[0] == "i")
            outs = " | ".join(a for a in atoms if a[0] == "o")
            printed = printed_outputs(year, prob)
            never = " & ".join(f"G !{a}" for a in atoms
                               if a[0] == "o" and ord(a[1]) - 64 not in printed) or "true"
            runs = f"({mutex}) & GF({ins}) & GF({outs}) & {never}"
            # Validate the alphabet route on every shipped property: it may
            # prove no officially false one.
            if never != "true":
                for n, _, f in props:
                    if unsat(f"{runs} & !({rers_seq.to_spot(f)})"):
                        assert official[n] == "holds", (year, prob, n, "alphabet")
                        alphabet_proved[official[n]] = alphabet_proved.get(official[n], 0) + 1
            # Unreachable events: like the alphabet, false at every position
            # of every run. Validated the same way on every shipped property.
            gone = unreachable.get((year, prob), set())
            back_runs = runs + "".join(f" & G !{a}" for a in sorted(gone))
            if gone:
                for n, _, f in props:
                    if unsat(f"{back_runs} & !({rers_seq.to_spot(f)})"):
                        assert official[n] == "holds", (year, prob, n, "backward")
                        back_proved[official[n]] = back_proved.get(official[n], 0) + 1
            for n, gloss, f in props:
                fixed = rers_seq.corrected(f)
                if fixed is None:
                    continue
                phi, fix = rers_seq.to_spot(f), rers_seq.to_spot(fixed)
                pattern, dwyer = from_gloss(gloss)
                assert implies(fix, dwyer) and implies(dwyer, fix), (year, prob, n)
                assert implies(phi, fix), (year, prob, n)
                assert not implies(fix, phi), (year, prob, n)
                assert not implies(fix, phi, mutex), (year, prob, n)
                assert official[n] == "fails", (year, prob, n)
                e = ex.get(n, {})

                def ex_answer(col, mapping):
                    return mapping.get(e.get(col, ""))

                hv = {"holds": "holds", "fails": "fails"}
                phi_v, phi_r = decide([("nuxmv", hv.get(sm.get((n, "phi")))),
                                       ("abstract", ab.get((n, "phi"))),
                                       ("explicit", ex_answer("phi", hv))], (year, prob, n))
                assert phi_v in ("fails", "unknown"), (year, prob, n, phi_v)
                # Degenerate: M |= !phi. Spot shows it for every model whose
                # runs alternate single input and output events forever.
                spot = unsat(f"({mutex}) & GF({ins}) & GF({outs}) & ({phi})")
                yes = {"holds": "yes", "fails": "no"}
                deg, deg_r = decide([("spot", "yes" if spot else None),
                                     ("nuxmv", yes.get(sm.get((n, "negphi")))),
                                     ("abstract", yes.get(ab.get((n, "negphi")))),
                                     ("explicit", ex_answer("phi_sat", {"no": "yes",
                                                                        "yes": "no"}))],
                                    (year, prob, n, "degenerate"))
                # Alphabet: an output the program never prints is false on
                # every run, and the fix may hold on every such word.
                alpha = "holds" if unsat(f"{runs} & !({fix})") else None
                back = "holds" if gone and not alpha and \
                    unsat(f"{back_runs} & !({fix})") else None
                fix_v, fix_r = decide([("alphabet", alpha),
                                       ("backward", back),
                                       ("nuxmv", hv.get(sm.get((n, "fix")))),
                                       ("abstract", ab.get((n, "fix"))),
                                       ("walk", walks.get((year, prob, n))),
                                       ("explicit", ex_answer("fix", hv)),
                                       ("official", ex_answer("fix_official", hv))],
                                      (year, prob, n, "fix"))
                wdeg, wdeg_r = decide([("nuxmv", yes.get(sm.get((n, "negfix")))),
                                     ("abstract", yes.get(ab.get((n, "negfix")))),
                                     ("explicit", ex_answer("fix_sat", {"no": "yes",
                                                                        "yes": "no"})),
                                     ("official", ex_answer("fix_sat_official", {"no": "yes"}))],
                                    (year, prob, n, "weakening degenerate"))
                rows.append({
                    "year": year, "track": "seq", "problem": prob, "prop": n,
                    "pattern": pattern, "gloss": gloss, "phi": phi, "weakening": fix,
                    "phi_official": "fails", "phi_verdict": phi_v, "phi_route": phi_r,
                    "degenerate": deg, "degenerate_route": deg_r,
                    "weakening_verdict": fix_v, "weakening_route": fix_r,
                    "weakening_degenerate": wdeg, "weakening_degenerate_route": wdeg_r,
                })
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with open(OUT, "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    print(f"{len(rows)} rows -> {OUT}", file=sys.stderr)
    print("shipped properties the alphabet route proves, by official verdict:",
          alphabet_proved, file=sys.stderr)
    print("shipped properties the alphabet with unreachable events proves:",
          back_proved, file=sys.stderr)
    for key in ("weakening_verdict", "weakening_degenerate", "degenerate"):
        counts = {}
        for r in rows:
            counts[r[key]] = counts.get(r[key], 0) + 1
        print(key, dict(sorted(counts.items())), file=sys.stderr)


if __name__ == "__main__":
    main(*sys.argv[1:6])
