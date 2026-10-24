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
  3. merges the verdicts on M of phi, !phi, the fix and !fix from two
     sound routes, and stops if they disagree: bench/rers_seq.py (explicit
     search plus the official lassos, and implication from officially true
     properties) and bench/rers_seq_smv.py (nuXmv IC3 on a translation of
     the C program, used only when it matches the official verdicts);
  4. shows phi degenerate (M |= !phi) with Spot: phi is unsatisfiable on
     any infinite run with one event per position and infinitely many
     inputs and outputs, which every run of M is.

The parallel track (Promela) properties carry no English gloss and none
has the G(Q & ...) shape, so the set is sequential only.

Columns: `phi` is the shipped formula and `weakening` the fix, both in
Spot syntax; `*_verdict` is holds, fails or unknown on M; `degenerate`
and `weakening_degenerate` say whether M |= !phi and M |= !fix; each
`*_route` names the routes (spot, explicit, nuxmv, official) that agree
on the value next to it.

Usage: rers_precedes.py EXPLICIT_DIR SMV_DIR   (the --out-dir of each check)
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


def decide(claims, what):
    """Merge (route, answer) claims; all decided routes must agree."""
    claims = [(r, a) for r, a in claims if a is not None]
    answers = {a for _, a in claims}
    assert len(answers) <= 1, (what, claims)
    if not claims:
        return "unknown", ""
    return answers.pop(), "+".join(sorted({r for r, _ in claims}))


def main(explicit_dir, smv_dir):
    rows = []
    for year in (2016, 2017):
        for prob in range(1, 10):
            atoms, props = rers_seq.load_properties(year, prob)
            official = {n: "holds" if s[1] else "fails"
                        for (n, _, _), s in zip(props, rers_seq.load_solutions(year, prob))}
            ex, _ = read_table(Path(explicit_dir) / f"{year}-{prob}.tsv")
            ex = {int(r["prop"]): r for r in ex}
            sm = smv_verdicts(Path(smv_dir) / f"smv-{year}-{prob}.tsv", official)
            mutex = one_event(atoms)
            ins = " | ".join(a for a in atoms if a[0] == "i")
            outs = " | ".join(a for a in atoms if a[0] == "o")
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
                                       ("explicit", ex_answer("phi", hv))], (year, prob, n))
                assert phi_v in ("fails", "unknown"), (year, prob, n, phi_v)
                # Degenerate: M |= !phi. Spot shows it for every model whose
                # runs alternate single input and output events forever.
                spot = unsat(f"({mutex}) & GF({ins}) & GF({outs}) & ({phi})")
                yes = {"holds": "yes", "fails": "no"}
                deg, deg_r = decide([("spot", "yes" if spot else None),
                                     ("nuxmv", yes.get(sm.get((n, "negphi")))),
                                     ("explicit", ex_answer("phi_sat", {"no": "yes",
                                                                        "yes": "no"}))],
                                    (year, prob, n, "degenerate"))
                fix_v, fix_r = decide([("nuxmv", hv.get(sm.get((n, "fix")))),
                                       ("explicit", ex_answer("fix", hv)),
                                       ("official", ex_answer("fix_official", hv))],
                                      (year, prob, n, "fix"))
                wdeg, wdeg_r = decide([("nuxmv", yes.get(sm.get((n, "negfix")))),
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
    for key in ("weakening_verdict", "weakening_degenerate", "degenerate"):
        counts = {}
        for r in rows:
            counts[r[key]] = counts.get(r[key], 0) + 1
        print(key, dict(sorted(counts.items())), file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
